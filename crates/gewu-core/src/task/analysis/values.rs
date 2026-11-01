//! Fixed-capacity inspector estimates. Aggregate reports keep their separate
//! full-count contract. Only representative row IDs, never owned values, are retained.

use std::{
    collections::{hash_map::DefaultHasher, BinaryHeap, HashMap},
    hash::{Hash, Hasher},
};

use arrow::{
    array::{
        downcast_dictionary_array, downcast_integer_array, downcast_temporal_array, Array,
        ArrayRef, AsArray, PrimitiveArray,
    },
    datatypes::*,
};

use super::{AnalysisContext, ValueSummary};
use crate::{column::format, scope::Selection, DistinctValue, Error, Result};

const DISTINCT_CAPACITY: usize = 4096;
const CANDIDATE_CAPACITY: usize = 256;
const OUTPUT_LIMIT: usize = 8;
type HashValue<'a> = Box<dyn Fn(usize) -> Option<u64> + 'a>;

fn hash(value: impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn primitive<T: ArrowPrimitiveType>(array: &PrimitiveArray<T>) -> HashValue<'_>
where
    T::Native: Hash,
{
    Box::new(move |row| array.is_valid(row).then(|| hash(array.value(row))))
}

fn real<T: ArrowPrimitiveType>(array: &PrimitiveArray<T>) -> HashValue<'_>
where
    T::Native: Into<f64>,
{
    Box::new(move |row| {
        array.is_valid(row).then(|| {
            let value: f64 = array.value(row).into();
            hash(crate::types::identity::normalized_bits(value))
        })
    })
}

/// Dispatch once. Dictionaries follow keys directly, avoiding Arrow row encoding
/// of the complete dictionary and a whole-column logical-null allocation.
fn hasher(array: &dyn Array) -> Result<HashValue<'_>> {
    use DataType::*;
    macro_rules! bytes {
        ($a:expr) => {{
            let a = $a;
            Ok(Box::new(move |row| {
                a.is_valid(row).then(|| hash(a.value(row)))
            }))
        }};
    }
    match array.data_type() {
        Dictionary(..) => downcast_dictionary_array!(array => {
            let value = hasher(array.values().as_ref())?;
            Ok(Box::new(move |row| array.key(row).and_then(&value)))
        }, _ => unreachable!()),
        Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64 => {
            downcast_integer_array!(array => Ok(primitive(array)), _ => unreachable!())
        }
        Float16 => Ok(real(array.as_primitive::<Float16Type>())),
        Float32 => Ok(real(array.as_primitive::<Float32Type>())),
        Float64 => Ok(real(array.as_primitive::<Float64Type>())),
        Decimal32(..) => Ok(primitive(array.as_primitive::<Decimal32Type>())),
        Decimal64(..) => Ok(primitive(array.as_primitive::<Decimal64Type>())),
        Decimal128(..) => Ok(primitive(array.as_primitive::<Decimal128Type>())),
        Decimal256(..) => Ok(primitive(array.as_primitive::<Decimal256Type>())),
        Duration(TimeUnit::Second) => Ok(primitive(array.as_primitive::<DurationSecondType>())),
        Duration(TimeUnit::Millisecond) => {
            Ok(primitive(array.as_primitive::<DurationMillisecondType>()))
        }
        Duration(TimeUnit::Microsecond) => {
            Ok(primitive(array.as_primitive::<DurationMicrosecondType>()))
        }
        Duration(TimeUnit::Nanosecond) => {
            Ok(primitive(array.as_primitive::<DurationNanosecondType>()))
        }
        Date32 | Date64 | Time32(_) | Time64(_) | Timestamp(..) => {
            downcast_temporal_array!(array => Ok(primitive(array)), _ => Err(Error::Unsupported("unsupported temporal values".into())))
        }
        Boolean => bytes!(array.as_boolean()),
        Utf8 => bytes!(array.as_string::<i32>()),
        LargeUtf8 => bytes!(array.as_string::<i64>()),
        Utf8View => bytes!(array.as_string_view()),
        _ => Err(Error::Unsupported(format!(
            "{} does not support value analysis",
            array.data_type()
        ))),
    }
}

#[derive(Default)]
struct Sketch {
    // KMV: the smallest distinct hashes and one representative row for each.
    ranks: BinaryHeap<u64>,
    samples: HashMap<u64, usize>,
    overflowed: bool,
    non_null: u64,
    // Misra-Gries counters nominate frequent values; these are NOT displayed counts.
    candidates: HashMap<u64, (u64, usize)>,
}

impl Sketch {
    fn add(&mut self, key: u64, row: usize) {
        self.non_null += 1;
        if !self.samples.contains_key(&key) {
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

    fn nominees(&self) -> HashMap<u64, (u64, usize)> {
        let mut result: HashMap<_, _> = self
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

pub(super) fn analyze(
    array: &ArrayRef,
    rows: &Selection,
    context: &AnalysisContext,
) -> Result<ValueSummary> {
    context.check()?;
    let value = hasher(array.as_ref())?;
    let mut sketch = Sketch::default();
    for (index, row) in rows.iter().enumerate() {
        context.check_at(index)?;
        if let Some(key) = value(row) {
            sketch.add(key, row);
        }
    }
    let mut counts = sketch.nominees();
    for (index, row) in rows.iter().enumerate() {
        context.check_at(index)?;
        if let Some(count) = value(row).and_then(|key| counts.get_mut(&key)) {
            count.0 += 1;
        }
    }
    let mut top: Vec<_> = counts.into_values().collect();
    top.sort_unstable_by_key(|&(count, row)| (std::cmp::Reverse(count), row));
    top.truncate(OUTPUT_LIMIT);
    let formatter = format::formatter(array.as_ref())?;
    let values = top
        .into_iter()
        .enumerate()
        .map(|(key, (count, row))| DistinctValue {
            key: key as u64,
            label: Some(formatter.value(row).to_string()),
            count,
        })
        .collect();
    context.check()?;
    Ok(ValueSummary {
        distinct_count: sketch.distinct(),
        estimated: true,
        values,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{DictionaryArray, Float64Array, Int32Array, Int64Array, StringArray};

    use super::*;

    #[test]
    fn bounded_sketch_preserves_heavy_hitters_and_recounts_candidates() {
        let mut sketch = Sketch::default();
        for row in 0..50_000 {
            sketch.add(hash(0_u64), row);
        }
        for row in 50_000..1_050_000 {
            sketch.add(hash(row as u64), row);
        }
        assert_eq!(sketch.samples.len(), DISTINCT_CAPACITY);
        assert!(sketch.candidates.len() <= CANDIDATE_CAPACITY);
        assert!(sketch.nominees().contains_key(&hash(0_u64)));
        assert!((950_000..1_050_000).contains(&sketch.distinct()));
        // Counters lose mass during candidate selection; output counts must not.
        let array: ArrayRef = Arc::new(Int64Array::from_iter_values(
            (0..50_000).map(|_| 0).chain(1..100_001),
        ));
        let summary = analyze(
            &array,
            &Selection::All(array.len()),
            &AnalysisContext::default(),
        )
        .unwrap();
        assert_eq!(
            (summary.values[0].label.as_deref(), summary.values[0].count),
            (Some("0"), 50_000)
        );
        let mut unique = Sketch::default();
        for row in 0..DISTINCT_CAPACITY {
            unique.add(hash(row), row);
        }
        assert!(!unique.overflowed);
        assert_eq!(unique.distinct(), DISTINCT_CAPACITY as u64);
        // Complete MG eviction cycles still have representative fallback values.
        let array: ArrayRef = Arc::new(Int64Array::from_iter_values(0..257 * 20));
        let summary = analyze(
            &array,
            &Selection::All(array.len()),
            &AnalysisContext::default(),
        )
        .unwrap();
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
        let summary = analyze(&array, &Selection::All(5), &AnalysisContext::default()).unwrap();
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
        let summary = analyze(
            &dictionary,
            &Selection::Rows(Arc::new(vec![4, 2, 0, 1, 3])),
            &AnalysisContext::default(),
        )
        .unwrap();
        assert_eq!(summary.distinct_count, 2);
        assert_eq!(
            (summary.values[0].label.as_deref(), summary.values[0].count),
            (Some("same"), 2)
        );
        let context = AnalysisContext::default();
        context.cancel();
        assert!(matches!(
            analyze(&dictionary, &Selection::All(5), &context),
            Err(Error::Cancelled)
        ));
    }
}
