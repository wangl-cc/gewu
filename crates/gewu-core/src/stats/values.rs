//! Fixed-capacity inspector estimates. Only representative row IDs, never owned values,
//! are retained.
//!
//! Columns with a compact distinct-value encoding count their codes exactly;
//! complete views reuse the cached per-value totals. Other columns use
//! fixed-capacity estimates with rapidhash: the 4,096 smallest hashes estimate
//! distinct cardinality, and 256 Misra–Gries counters select candidates,
//! keeping source row IDs rather than owned values. A second scan recounts the
//! selected candidates exactly, so displayed counts are not scaled samples.
//! Candidate selection can miss values that rank high across the whole table,
//! and 64-bit hash collisions can merge groups, so results are always marked
//! as estimated. Native dictionary arrays hash each dictionary entry once and
//! then follow keys.

use std::{cmp::Reverse, collections::BinaryHeap};

use arrow::{
    array::{downcast_dictionary_array, downcast_primitive_array, Array, AsArray, PrimitiveArray},
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

#[derive(Debug, Clone)]
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

/// Hash every dictionary entry once; rows then follow their keys.
fn entry_hashes(values: &dyn Array) -> Result<Vec<Option<u64>>> {
    let mut hashes = vec![None; values.len()];
    let all = Selection::Range(0..values.len());
    for_each_hash(values, &all, |index, key| hashes[index] = Some(key))?;
    Ok(hashes)
}

/// Call `f(row, hash)` for every selected non-null value. Dispatch happens
/// once; each arm is a monomorphic loop over native values and the validity bitmap.
fn for_each_hash(
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
        Dictionary(..) => downcast_dictionary_array!(array => {
            let entries = entry_hashes(array.values().as_ref())?;
            for_each_valid(array.keys(), selection, |index, key| {
                if let Some(Some(hash)) = entries.get(key.as_usize()) {
                    f(index, *hash);
                }
            });
        }, _ => unreachable!()),
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
    if let Ok(Some(dictionary)) = view.column().dictionary() {
        return encoded(view, dictionary);
    }
    let array = view.array().as_ref();
    let selection = view.selection();
    let mut sketch = Sketch::default();
    for_each_hash(array, &selection, |row, key| sketch.add(key, row))?;
    let mut counts = sketch.nominees();
    for_each_hash(array, &selection, |_, key| {
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
fn encoded(view: ColView<'_>, dictionary: &Dictionary) -> Result<ValueSummary> {
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
            crate::stats::scan::for_each_valid_row(nulls, &selection, |row| {
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
#[path = "values_tests.rs"]
mod tests;
