//! Fixed-capacity frequency sketch over native values. Arrow dictionary input additionally
//! uses one temporary hash per dictionary entry, reused across both scans.

use std::{cmp::Reverse, collections::BinaryHeap};

use arrow::{
    array::{downcast_integer_array, downcast_primitive_array, Array, AsArray, PrimitiveArray},
    datatypes::*,
};
use rapidhash::{v3::rapidhash_v3, RapidHashMap};

use crate::{
    col_view::{ColView, Selection},
    column::Dictionary,
    format,
    stats::{
        scan::{for_each_valid, for_each_valid_index},
        Result,
    },
};

/// A displayed non-null value and its frequency in the captured view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueFrequency {
    pub label: String,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValueSummary {
    /// Non-null distinct values, estimated with a fixed-capacity sketch.
    pub distinct_count: u64,
    /// Approximate candidate selection, recounted over all selected rows.
    /// 64-bit hash collisions may merge values.
    pub values: Vec<ValueFrequency>,
}

/// Scalar values with comparable labels and bounded distinct estimates.
pub(super) fn supports(data_type: &DataType) -> bool {
    use DataType::*;
    matches!(
        crate::types::value_type(data_type),
        Utf8 | LargeUtf8
            | Utf8View
            | Boolean
            | Int8
            | Int16
            | Int32
            | Int64
            | UInt8
            | UInt16
            | UInt32
            | UInt64
            | Float16
            | Float32
            | Float64
            | Decimal32(..)
            | Decimal64(..)
            | Decimal128(..)
            | Decimal256(..)
            | Date32
            | Date64
            | Time32(_)
            | Time64(_)
            | Timestamp(..)
            | Duration(_)
    )
}

const DISTINCT_CAPACITY: usize = 4096;
const CANDIDATE_CAPACITY: usize = 256;
const OUTPUT_LIMIT: usize = 8;

/// Native values hash their bytes; KMV estimates need well-mixed 64-bit keys.
#[inline]
fn hash_bytes(bytes: &[u8]) -> u64 {
    rapidhash_v3(bytes)
}

#[inline]
fn hash_native<N: ArrowNativeType>(value: N) -> u64 {
    hash_bytes(std::slice::from_ref(&value).to_byte_slice())
}

fn normalize_float_bits(value: f64) -> u64 {
    if value == 0.0 {
        0
    } else if value.is_nan() {
        f64::NAN.to_bits()
    } else {
        value.to_bits()
    }
}

/// Both passes read the same hashes for dictionary entries. Native arrays hash selected values
/// directly, keeping memory independent of the number of rows.
enum ValueHashes<'a> {
    Native(&'a dyn Array),
    Dictionary {
        keys: &'a dyn Array,
        entries: Vec<Option<u64>>,
    },
}

impl<'a> ValueHashes<'a> {
    fn prepare(array: &'a dyn Array) -> Result<Self> {
        let Some(dictionary) = array.as_any_dictionary_opt() else {
            return Ok(Self::Native(array));
        };
        let values = dictionary.values();
        let mut entries = vec![None; values.len()];
        Self::prepare(values.as_ref())?.visit(
            &Selection::Range(0..values.len()),
            |row, hash| {
                entries[row] = Some(hash);
            },
        )?;
        Ok(Self::Dictionary {
            keys: dictionary.keys(),
            entries,
        })
    }

    fn visit(&self, selection: &Selection<'_>, mut f: impl FnMut(usize, u64)) -> Result<()> {
        match self {
            Self::Native(array) => for_each_native_hash(*array, selection, f),
            Self::Dictionary { keys, entries } => {
                let keys = *keys;
                downcast_integer_array!(keys => {
                    for_each_valid(keys, selection, |row, key| {
                        if let Some(hash) = entries[key.as_usize()] {
                            f(row, hash);
                        }
                    });
                }, _ => unreachable!("Arrow dictionary keys are integers"));
                Ok(())
            }
        }
    }
}

/// Dispatch once per native scan, then visit each selected non-null value.
fn for_each_native_hash(
    array: &dyn Array,
    selection: &Selection<'_>,
    mut f: impl FnMut(usize, u64),
) -> Result<()> {
    use DataType::*;
    fn primitive<T: ArrowPrimitiveType>(
        array: &PrimitiveArray<T>,
        selection: &Selection<'_>,
        f: &mut impl FnMut(usize, u64),
    ) {
        for_each_valid(array, selection, |index, value| {
            f(index, hash_native(value))
        });
    }
    fn real<T: ArrowPrimitiveType>(
        array: &PrimitiveArray<T>,
        selection: &Selection<'_>,
        f: &mut impl FnMut(usize, u64),
    ) where
        T::Native: Into<f64>,
    {
        for_each_valid(array, selection, |index, value| {
            let bits = normalize_float_bits(value.into());
            f(index, hash_bytes(&bits.to_le_bytes()));
        });
    }
    macro_rules! bytes {
        ($a:expr) => {{
            let a = $a;
            for_each_valid_index(a, selection, |index| {
                f(index, hash_bytes(a.value(index).as_ref()))
            });
        }};
    }
    match array.data_type() {
        Float16 => real(array.as_primitive::<Float16Type>(), selection, &mut f),
        Float32 => real(array.as_primitive::<Float32Type>(), selection, &mut f),
        Float64 => real(array.as_primitive::<Float64Type>(), selection, &mut f),
        // Integers, decimals, dates, times, timestamps and durations hash
        // their native bytes.
        data_type if data_type.is_primitive() => {
            downcast_primitive_array!(array => primitive(array, selection, &mut f), _ => unreachable!())
        }
        Boolean => {
            let a = array.as_boolean();
            for_each_valid_index(a, selection, |index| {
                f(index, hash_bytes(&[u8::from(a.value(index))]))
            });
        }
        Utf8 => bytes!(array.as_string::<i32>()),
        LargeUtf8 => bytes!(array.as_string::<i64>()),
        Utf8View => bytes!(array.as_string_view()),
        _ => {
            return Err(crate::InternalError::Invariant(format!(
                "{} passed value-analysis support checking but cannot be hashed",
                array.data_type()
            ))
            .into())
        }
    }
    Ok(())
}

#[derive(Default)]
struct Sketch {
    // KMV: the smallest distinct hashes and one representative row for each.
    ranks: BinaryHeap<u64>,
    samples: RapidHashMap<u64, usize>,
    overflowed: bool,
    non_null: u64,
    // Misra-Gries counters nominate frequent values; these are NOT displayed counts.
    candidates: RapidHashMap<u64, (u64, usize)>,
}

impl Sketch {
    fn add(&mut self, key: u64, row: usize) {
        self.non_null += 1;
        if self.samples.len() == DISTINCT_CAPACITY && key > *self.ranks.peek().unwrap() {
            // Larger than every retained hash, so neither retained nor retainable.
            self.overflowed = true;
        } else if !self.samples.contains_key(&key) {
            if self.samples.len() < DISTINCT_CAPACITY {
                self.samples.insert(key, row);
                self.ranks.push(key);
            } else {
                self.overflowed = true;
                let largest = *self.ranks.peek().unwrap();
                if key < largest {
                    self.samples.remove(&largest);
                    *self.ranks.peek_mut().unwrap() = key;
                    self.samples.insert(key, row);
                }
            }
        }
        if let Some((count, _)) = self.candidates.get_mut(&key) {
            *count += 1;
        } else if self.candidates.len() < CANDIDATE_CAPACITY {
            self.candidates.insert(key, (1, row));
        } else {
            self.candidates.retain(|_, (count, _)| {
                *count -= 1;
                *count != 0
            });
        }
    }

    fn distinct(&self) -> u64 {
        if !self.overflowed {
            return self.samples.len() as u64;
        }
        let threshold = (*self.ranks.peek().unwrap() as f64 + 1.0) / (u64::MAX as f64 + 1.0);
        (((DISTINCT_CAPACITY - 1) as f64 / threshold).round() as u64)
            .clamp(DISTINCT_CAPACITY as u64 + 1, self.non_null)
    }

    fn nominees(&self) -> RapidHashMap<u64, (u64, usize)> {
        let mut result: RapidHashMap<_, _> = self
            .candidates
            .iter()
            .map(|(&key, &(_, row))| (key, (0, row)))
            .collect();
        if result.len() < OUTPUT_LIMIT {
            let mut samples: Vec<_> = self.samples.iter().collect();
            samples.sort_unstable_by_key(|&(key, _)| key);
            for (&key, &row) in samples {
                result.entry(key).or_insert((0, row));
                if result.len() >= OUTPUT_LIMIT {
                    break;
                }
            }
        }
        result
    }
}

pub(crate) fn analyze(view: ColView<'_>) -> Result<ValueSummary> {
    if !supports(view.data_type()) {
        return Err(crate::AnalysisError::UnsupportedOperation {
            operation: crate::AnalysisOperation::Values,
            data_type: view.data_type().clone(),
        });
    }
    // Low-cardinality columns already have (or cheaply build) a shared code
    // per row: count codes exactly instead of hashing values.
    if let Some(dictionary) = view.column().dictionary()? {
        return encoded(&view, dictionary);
    }
    let array = view.array().as_ref();
    let selection = view.selection();
    let hashes = ValueHashes::prepare(array)?;
    let mut sketch = Sketch::default();
    hashes.visit(selection, |row, key| sketch.add(key, row))?;
    let mut counts = sketch.nominees();
    hashes.visit(selection, |_, key| {
        if let Some(count) = counts.get_mut(&key) {
            count.0 += 1;
        }
    })?;
    let top = counts
        .into_values()
        .map(|(count, row)| (count, row as u32, row));
    let formatter = format::formatter(array)?;
    Ok(ValueSummary {
        distinct_count: sketch.distinct(),
        values: ranked(top, |row| formatter.value(row).to_string()),
    })
}

/// Exact counts from the column's value codes. Representative rows are the
/// first occurrence in scan order, as in the sketch path.
fn encoded(view: &ColView<'_>, dictionary: &Dictionary) -> Result<ValueSummary> {
    let n = dictionary.all_counts().len();
    let codes = dictionary.codes();
    let nulls = dictionary.nulls();
    let mut first = vec![u32::MAX; n];
    let counts = match view.selection() {
        Selection::Range(_) => {
            let counts = dictionary.all_counts().to_vec();
            let mut missing = counts.iter().filter(|&&count| count > 0).count();
            let mut visit = |row: usize| {
                let slot = &mut first[codes[row] as usize];
                if *slot == u32::MAX {
                    *slot = row as u32;
                    missing -= 1;
                }
                missing == 0
            };
            match nulls.filter(|nulls| nulls.null_count() > 0) {
                None => _ = (0..codes.len()).any(&mut visit),
                Some(nulls) => _ = nulls.valid_indices().any(&mut visit),
            }
            counts
        }
        selection => {
            let mut counts = vec![0_u64; n];
            crate::stats::scan::for_each_valid_row(nulls, selection, |row| {
                let code = codes[row] as usize;
                if counts[code] == 0 {
                    first[code] = row as u32;
                }
                counts[code] += 1;
            });
            counts
        }
    };
    let distinct_count = counts.iter().filter(|&&count| count > 0).count() as u64;
    let top = counts
        .into_iter()
        .zip(first)
        .filter(|&(count, _)| count > 0)
        .map(|(count, row)| (count, row, row as usize));
    let formatter = format::formatter(view.array().as_ref())?;
    Ok(ValueSummary {
        distinct_count,
        values: ranked(top, |row| formatter.value(row).to_string()),
    })
}

/// The most frequent `(count, source row, label row)` entries, ties by source row.
fn ranked(
    entries: impl Iterator<Item = (u64, u32, usize)>,
    label: impl Fn(usize) -> String,
) -> Vec<ValueFrequency> {
    let mut top: Vec<_> = entries.collect();
    top.sort_unstable_by_key(|&(count, row, _)| (Reverse(count), row));
    top.truncate(OUTPUT_LIMIT);
    top.into_iter()
        .map(|(count, _, index)| ValueFrequency {
            label: label(index),
            count,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{
        ArrayRef, DictionaryArray, Float64Array, Int32Array, Int64Array, StringArray,
    };

    use super::*;

    fn hash(value: usize) -> u64 {
        hash_native(value as u64)
    }

    #[test]
    fn dictionary_construction_errors_reach_value_analysis() {
        let array: ArrayRef = Arc::new(
            arrow::array::TimestampSecondArray::from(vec![0]).with_timezone("Unknown/Timezone"),
        );
        let fixture = crate::col_view::test::Fixture::all(&array);
        assert!(matches!(
            analyze(fixture.view()),
            Err(crate::AnalysisError::Column(crate::ColumnError::Internal(
                crate::InternalError::Arrow(_)
            )))
        ));
    }

    #[test]
    fn bounded_sketch_preserves_heavy_hitters_and_recounts_candidates() {
        let mut sketch = Sketch::default();
        for row in 0..50_000 {
            sketch.add(hash(0), row);
        }
        for row in 50_000..1_050_000 {
            sketch.add(hash(row), row);
        }
        assert_eq!(sketch.samples.len(), DISTINCT_CAPACITY);
        assert!(sketch.candidates.len() <= CANDIDATE_CAPACITY);
        assert!(sketch.nominees().contains_key(&hash(0)));
        assert!((950_000..1_050_000).contains(&sketch.distinct()));
        // Counters lose mass during candidate selection; output counts must not.
        let array: ArrayRef = Arc::new(Int64Array::from_iter_values(
            (0..50_000).map(|_| 0).chain(1..100_001),
        ));
        let selected = crate::col_view::test::Fixture::all(&array);
        let summary = analyze(selected.view()).unwrap();
        assert_eq!(
            (summary.values[0].label.as_str(), summary.values[0].count),
            ("0", 50_000)
        );
        let mut unique = Sketch::default();
        for row in 0..DISTINCT_CAPACITY {
            unique.add(hash(row), row);
        }
        assert!(!unique.overflowed);
        assert_eq!(unique.distinct(), DISTINCT_CAPACITY as u64);
        // Complete MG eviction cycles still have representative fallback values.
        let array: ArrayRef = Arc::new(Int64Array::from_iter_values(0..257 * 20));
        let selected = crate::col_view::test::Fixture::all(&array);
        let summary = analyze(selected.view()).unwrap();
        assert_eq!(summary.values.len(), OUTPUT_LIMIT);
        assert!(summary.values.iter().all(|value| value.count == 1));
    }

    #[test]
    fn native_hashing_respects_float_and_dictionary_value_identity() {
        let array: ArrayRef = Arc::new(Float64Array::from(vec![
            Some(-0.0),
            Some(0.0),
            Some(f64::NAN),
            Some(f64::from_bits(0x7ff8000000000001)),
            None,
        ]));
        let all = crate::col_view::test::Fixture::all(&array);
        let summary = analyze(all.view()).unwrap();
        assert_eq!(summary.distinct_count, 2);
        assert!(summary.values.iter().all(|value| value.count == 2));
        let dictionary: ArrayRef = Arc::new(
            DictionaryArray::<Int32Type>::try_new(
                Int32Array::from(vec![Some(0), Some(1), Some(2), None, Some(3)]),
                Arc::new(StringArray::from(vec![
                    Some("same"),
                    Some("same"),
                    None,
                    Some("other"),
                ])),
            )
            .unwrap(),
        );
        let selected = crate::col_view::test::Fixture::new(&dictionary, vec![4, 2, 0, 1, 3]);
        let summary = analyze(selected.view()).unwrap();
        assert_eq!(summary.distinct_count, 2);
        assert_eq!(
            (summary.values[0].label.as_str(), summary.values[0].count),
            ("same", 2)
        );
    }

    #[test]
    fn nested_dictionary_hashes_value_identity_and_skips_nulls_at_each_level() {
        let values = Arc::new(StringArray::from(vec![
            Some("one"),
            None,
            Some("one"),
            Some("two"),
        ]));
        // More entries than the compact code space force the hash-based analysis.
        let entries = Int32Array::from_iter((0..300).map(|key| match key {
            0..=2 => Some(key),
            3 => None,
            _ => Some(3),
        }));
        let inner = Arc::new(DictionaryArray::<Int32Type>::new(entries, values));
        let keys = Int32Array::from(vec![Some(0), Some(1), Some(2), Some(3), None, Some(299)]);
        let nested: ArrayRef = Arc::new(DictionaryArray::<Int32Type>::new(keys, inner));

        let all = crate::col_view::test::Fixture::all(&nested);
        let selected = crate::col_view::test::Fixture::masked(&nested, vec![0, 1, 2, 3, 4]);
        let one = ValueFrequency {
            label: "one".into(),
            count: 2,
        };
        let two = ValueFrequency {
            label: "two".into(),
            count: 1,
        };
        assert_eq!(analyze(all.view()).unwrap(), ValueSummary {
            distinct_count: 2,
            values: vec![one.clone(), two],
        });
        assert_eq!(analyze(selected.view()).unwrap(), ValueSummary {
            distinct_count: 1,
            values: vec![one],
        });
    }

    /// Columns above the code-space limit take the hashing path. With fewer
    /// than 4,096 distinct values it is exact, so it must agree with the
    /// encoded path's semantics for every supported storage type.
    #[test]
    fn hashing_path_counts_every_storage_type_like_the_encoded_path() {
        use arrow::array::{
            BooleanArray, Decimal128Array, DurationSecondArray, Float32Array, LargeStringArray,
            StringViewArray, TimestampMillisecondArray, UInt16Array,
        };
        const DISTINCT: usize = 300;
        // Value `i % DISTINCT`, plus four extra rows of value 7, plus a null.
        let ids: Vec<Option<usize>> = (0..2 * DISTINCT)
            .map(|i| Some(i % DISTINCT))
            .chain([Some(7); 4])
            .chain([None])
            .collect();
        let text = |id: usize| format!("v{id:03}");
        let arrays: Vec<ArrayRef> = vec![
            Arc::new(Int64Array::from_iter(
                ids.iter().map(|id| id.map(|id| id as i64)),
            )),
            Arc::new(Float32Array::from_iter(
                ids.iter().map(|id| id.map(|id| id as f32)),
            )),
            // Signed zeros and NaN payloads are one value each.
            Arc::new(Float64Array::from_iter(ids.iter().enumerate().map(
                |(row, id)| {
                    id.map(|id| match (id, row < DISTINCT) {
                        (0, true) => -0.0,
                        (0, false) => 0.0,
                        (1, true) => f64::NAN,
                        (1, false) => f64::from_bits(0x7ff8_0000_0000_0001),
                        _ => id as f64,
                    })
                },
            ))),
            Arc::new(
                Decimal128Array::from_iter(ids.iter().map(|id| id.map(|id| id as i128)))
                    .with_precision_and_scale(10, 2)
                    .unwrap(),
            ),
            Arc::new(DurationSecondArray::from_iter(
                ids.iter().map(|id| id.map(|id| id as i64)),
            )),
            Arc::new(TimestampMillisecondArray::from_iter(
                ids.iter().map(|id| id.map(|id| id as i64)),
            )),
            Arc::new(StringArray::from_iter(ids.iter().map(|id| id.map(text)))),
            Arc::new(LargeStringArray::from_iter(
                ids.iter().map(|id| id.map(text)),
            )),
            Arc::new(StringViewArray::from_iter(
                ids.iter().map(|id| id.map(text)),
            )),
            Arc::new(arrow::array::Float16Array::from_iter(
                ids.iter()
                    .map(|id| id.map(|id| half::f16::from_f32(id as f32))),
            )),
            Arc::new(
                arrow::array::Decimal256Array::from_iter(
                    ids.iter().map(|id| id.map(|id| i256::from(id as i64))),
                )
                .with_precision_and_scale(40, 1)
                .unwrap(),
            ),
            Arc::new(arrow::array::DurationMillisecondArray::from_iter(
                ids.iter().map(|id| id.map(|id| id as i64)),
            )),
            // A native dictionary with more entries than the compact code space:
            // keys repeat entries, and two entries share one value.
            Arc::new(
                DictionaryArray::<Int32Type>::try_new(
                    Int32Array::from_iter(ids.iter().map(|id| id.map(|id| id as i32))),
                    Arc::new(StringArray::from_iter_values(
                        (0..DISTINCT).map(|id| text(if id == 299 { 298 } else { id })),
                    )),
                )
                .unwrap(),
            ),
        ];
        for array in arrays {
            let expected_distinct = match array.data_type() {
                DataType::Dictionary(..) => DISTINCT as u64 - 1,
                _ => DISTINCT as u64,
            };
            let all = crate::col_view::test::Fixture::all(&array);
            let summary = analyze(all.view()).unwrap();
            assert_eq!(
                summary.distinct_count,
                expected_distinct,
                "{}",
                array.data_type()
            );
            assert_eq!(summary.values[0].count, 6, "{}", array.data_type());
            assert_eq!(summary.values.len(), OUTPUT_LIMIT);
            // Explicit rows, including the null row, scan only the selection.
            let null_row = ids.len() as u32 - 1;
            let selected = crate::col_view::test::Fixture::new(&array, vec![null_row, 7, 307, 8]);
            let summary = analyze(selected.view()).unwrap();
            assert_eq!(summary.distinct_count, 2, "{}", array.data_type());
            assert_eq!(summary.values[0].count, 2, "{}", array.data_type());
            assert_eq!(summary.values[1].count, 1, "{}", array.data_type());
        }
        // A native dictionary of booleans larger than the code space hashes
        // its two values once per entry.
        let array: ArrayRef = Arc::new(
            DictionaryArray::<Int32Type>::try_new(
                Int32Array::from_iter_values((0..600).map(|i| i % 300)),
                Arc::new(BooleanArray::from_iter((0..300).map(|i| Some(i % 3 == 0)))),
            )
            .unwrap(),
        );
        let summary = analyze(crate::col_view::test::Fixture::all(&array).view()).unwrap();
        assert_eq!(summary.distinct_count, 2);
        assert_eq!(
            (summary.values[0].label.as_str(), summary.values[0].count),
            ("false", 400)
        );
        // Low-cardinality storage types use exact codes, also over explicit
        // rows with nulls.
        let array: ArrayRef = Arc::new(BooleanArray::from(vec![
            Some(true),
            None,
            Some(false),
            Some(true),
        ]));
        let selected = crate::col_view::test::Fixture::new(&array, vec![3, 1, 0]);
        let summary = analyze(selected.view()).unwrap();
        assert_eq!(summary.distinct_count, 1);
        assert_eq!(
            (summary.values[0].label.as_str(), summary.values[0].count),
            ("true", 2)
        );
        let array: ArrayRef = Arc::new(UInt16Array::from(vec![Some(1), None, Some(1)]));
        assert_eq!(
            analyze(crate::col_view::test::Fixture::all(&array).view())
                .unwrap()
                .values[0]
                .count,
            2
        );
    }
}
