//! Distinct values of a column: an index for columns with few of them, and
//! a one-off count for columns with many.
//!
//! The index gives every row a code (`u32`) naming its distinct value.
//! Codes are ordered like the values themselves (nulls first), so code order
//! is display order. The checkbox filter keeps a set of codes, and counting
//! values is a single pass over the codes. The index is kept for the life of
//! the document, so it is only built for columns with at most
//! [`INDEX_LIMIT`] distinct values; a column of ids or UUIDs would cost
//! hundreds of megabytes.

use std::{
    cmp::Reverse,
    collections::{hash_map::DefaultHasher, BinaryHeap, HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::Arc,
};

use arrow::{
    array::{Array, ArrayRef, AsArray, PrimitiveArray, UInt32Array},
    compute::take,
    datatypes::{ArrowPrimitiveType, DataType, Float16Type, Float32Type, Float64Type},
    row::{RowConverter, SortField},
};

use crate::{
    analysis::AnalysisContext, column::format, predicate::Predicate, scope::Selection,
    selection::RowSelection, Result,
};

/// A column's value index, built on first use.
#[derive(Clone)]
pub(crate) enum Indexed {
    Built(Arc<ValueIndex>),
    /// More than [`INDEX_LIMIT`] distinct values: counted on demand instead.
    TooMany,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistinctValue {
    /// Stable for the lifetime of the document; pass it to [`crate::Filter::InValues`].
    pub key: u64,
    /// Displayed text; `None` is the null value.
    pub label: Option<String>,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistinctResult {
    /// Sorted by value, nulls first.
    pub values: Vec<DistinctValue>,
    /// Number of distinct values before `limit` was applied.
    pub total: u64,
    pub truncated: bool,
    /// Whether the keys name values for [`crate::Filter::InValues`]. False for
    /// columns with too many distinct values to index; `values` then holds
    /// the most common ones.
    pub keyed: bool,
}

/// Most distinct values a column may have and still be indexed.
/// Aligned with UI choices limit (CHOICE_LIMIT = 500) with a safe headroom.
/// Columns with higher cardinality bypass the index and use streaming counts/searches.
pub(crate) const INDEX_LIMIT: usize = 1_000;

/// Rows converted to row format at a time, to bound memory.
const CHUNK: usize = 64 * 1024;

/// Whether the whole column has at most `limit` distinct values, null included.
///
/// Only one row-conversion batch and at most `limit` distinct encoded values
/// are retained. Unlike building an index, this does not allocate source-row
/// codes or format labels. The first new value beyond the limit stops the scan.
pub(crate) fn has_at_most(array: &ArrayRef, limit: usize) -> Result<bool> {
    let converter = RowConverter::new(vec![SortField::new(array.data_type().clone())])?;
    let mut seen: HashSet<Box<[u8]>> = HashSet::new();
    for start in (0..array.len()).step_by(CHUNK) {
        let len = CHUNK.min(array.len() - start);
        let rows = converter.convert_columns(&[array.slice(start, len)])?;
        for row in rows.iter() {
            let bytes: &[u8] = row.as_ref();
            if !seen.contains(bytes) {
                if seen.len() == limit {
                    return Ok(false);
                }
                seen.insert(bytes.into());
            }
        }
    }
    Ok(true)
}

/// Count the total number of distinct values in the array.
pub(crate) fn count_distinct(array: &ArrayRef) -> Result<usize> {
    if array.is_empty() {
        return Ok(0);
    }
    let counts = count_values(array, &Selection::All(array.len()), 0)?;
    Ok(counts.distinct as usize)
}

pub(crate) struct ValueIndex {
    /// Code of each source row.
    codes: Vec<u32>,
    /// Display label of each code, `None` for null.
    labels: Vec<Option<String>>,
}

impl ValueIndex {
    /// Index `array`, or `None` if it has more than `limit` distinct values.
    pub fn build(array: &ArrayRef, limit: usize) -> Result<Option<Self>> {
        let converter = RowConverter::new(vec![SortField::new(array.data_type().clone())])?;

        // Codes in order of first appearance; each distinct value's row-format
        // bytes (which sort like the values, nulls first) and first row.
        let mut seen: HashMap<Box<[u8]>, u32> = HashMap::new();
        let mut first_row: Vec<usize> = Vec::new();
        let mut codes = Vec::with_capacity(array.len());
        for start in (0..array.len()).step_by(CHUNK) {
            let len = CHUNK.min(array.len() - start);
            let rows = converter.convert_columns(&[array.slice(start, len)])?;
            for (i, row) in rows.iter().enumerate() {
                let bytes: &[u8] = row.as_ref();
                let code = match seen.get(bytes) {
                    Some(&c) => c,
                    None => {
                        if first_row.len() == limit {
                            return Ok(None);
                        }
                        let c = first_row.len() as u32;
                        seen.insert(bytes.into(), c);
                        first_row.push(start + i);
                        c
                    }
                };
                codes.push(code);
            }
        }

        let mut by_code: Vec<(&[u8], u32)> = seen.iter().map(|(b, &c)| (&b[..], c)).collect();
        by_code.sort_unstable();
        let mut remap = vec![0u32; by_code.len()];
        for (new, &(_, old)) in by_code.iter().enumerate() {
            remap[old as usize] = new as u32;
        }
        for c in codes.iter_mut() {
            *c = remap[*c as usize];
        }

        let f = format::formatter(array.as_ref())?;
        let labels = by_code
            .iter()
            .map(|&(_, old)| format::cell(array.as_ref(), &f, first_row[old as usize]))
            .collect();

        Ok(Some(Self { codes, labels }))
    }

    pub fn len(&self) -> usize {
        self.labels.len()
    }

    /// Each distinct value with its count over `rows`, in value order,
    /// leaving out values with no rows.
    pub fn counts(&self, rows: &Selection) -> Vec<DistinctValue> {
        self.counts_with_zero(rows, false)
    }

    /// Every source-column value in value order, including those absent from
    /// `rows`. Stable options remain selectable as other filters change.
    pub fn counts_all(&self, rows: &Selection) -> Vec<DistinctValue> {
        self.counts_with_zero(rows, true)
    }

    fn counts_with_zero(&self, rows: &Selection, include_zero: bool) -> Vec<DistinctValue> {
        let mut counts = vec![0u64; self.len()];
        match rows {
            Selection::All(_) => self.codes.iter().for_each(|&c| counts[c as usize] += 1),
            Selection::Rows(rows) => rows
                .iter()
                .for_each(|&r| counts[self.codes[r as usize] as usize] += 1),
        }
        counts
            .into_iter()
            .enumerate()
            .filter(|&(_, n)| include_zero || n > 0)
            .map(|(code, count)| DistinctValue {
                key: code as u64,
                label: self.labels[code].clone(),
                count,
            })
            .collect()
    }

    /// Returns the internal code for the given row index.
    #[inline]
    pub fn code(&self, row: usize) -> u32 {
        self.codes[row]
    }

    /// Resolve matching keys for a list of displayed labels (`None` = null).
    pub fn keys_for_labels(&self, labels: &[Option<String>]) -> Vec<u64> {
        let target: HashSet<&Option<String>> = labels.iter().collect();
        self.labels
            .iter()
            .enumerate()
            .filter(|(_, l)| target.contains(l))
            .map(|(k, _)| k as u64)
            .collect()
    }

    fn make_chunk_predicate(self: &Arc<Self>, allowed: Vec<bool>) -> Predicate<'static> {
        let index = Arc::clone(self);
        Predicate::new(Box::new(move |rows, incoming, output, _cancel| {
            match (rows, incoming) {
                (RowSelection::Range(range), None) => {
                    let slice = &index.codes[range];
                    for (i, &code) in slice.iter().enumerate() {
                        output[i] = allowed[code as usize];
                    }
                }
                (RowSelection::Range(range), Some(mask)) => {
                    let slice = &index.codes[range];
                    for (i, &code) in slice.iter().enumerate() {
                        output[i] = allowed[code as usize] & mask[i];
                    }
                }
                (RowSelection::Indices(indices), None) => {
                    for (i, &row) in indices.iter().enumerate() {
                        output[i] = allowed[index.code(row) as usize];
                    }
                }
                (RowSelection::Indices(indices), Some(mask)) => {
                    for (i, &row) in indices.iter().enumerate() {
                        output[i] = allowed[index.code(row) as usize] & mask[i];
                    }
                }
            }
            Ok(())
        }))
    }

    /// Prepare a row-based predicate for filtering by key codes.
    pub fn prepare_predicate(self: &Arc<Self>, keys: &[u64]) -> Predicate<'static> {
        let mut allowed = vec![false; self.len()];
        for &key in keys {
            if let Ok(key) = usize::try_from(key) {
                if let Some(slot) = allowed.get_mut(key) {
                    *slot = true;
                }
            }
        }
        self.make_chunk_predicate(allowed)
    }

    /// Prepare a row-based predicate for filtering by text values (`None` for null).
    pub fn prepare_text_predicate(
        self: &Arc<Self>,
        values: &[Option<String>],
        exclude: bool,
    ) -> Predicate<'static> {
        let keys = self.keys_for_labels(values);
        let mut allowed = vec![exclude; self.len()];
        for k in keys {
            if let Some(slot) = allowed.get_mut(k as usize) {
                *slot = !exclude;
            }
        }
        self.make_chunk_predicate(allowed)
    }
}

/// Distinct values counted once, without keeping an index.
pub(crate) struct ValueCounts {
    /// Distinct values among the rows, null included.
    pub distinct: u64,
    /// The `keep` most common values (ties: earliest first), in value
    /// order. Their keys only number them (0, 1, ...): there is no index to
    /// name them, so they cannot be used in a filter.
    pub top: Vec<DistinctValue>,
}

/// Count the distinct values of `array` over `rows`, keeping the `keep`
/// most common. Values are told apart by a 64-bit hash, so memory is a few
/// words per distinct value; a collision (unlikely below billions of
/// values) would merge two values. Floats use exact normalized bit keys instead:
/// signed zeros form one group, all NaNs another, and null remains separate.
pub(crate) fn count_values(array: &ArrayRef, rows: &Selection, keep: usize) -> Result<ValueCounts> {
    count_values_with_context(array, rows, keep, &AnalysisContext::default())
}

pub(crate) fn count_values_with_context(
    array: &ArrayRef,
    rows: &Selection,
    keep: usize,
    context: &AnalysisContext,
) -> Result<ValueCounts> {
    context.check()?;
    let converter = RowConverter::new(vec![SortField::new(array.data_type().clone())])?;
    let counts = match array.data_type() {
        DataType::Float16 => count_floats(array.as_primitive::<Float16Type>(), rows, context)?,
        DataType::Float32 => count_floats(array.as_primitive::<Float32Type>(), rows, context)?,
        DataType::Float64 => count_floats(array.as_primitive::<Float64Type>(), rows, context)?,
        _ => {
            // hash -> (count, first row)
            let mut counts: HashMap<u64, (u64, u32)> = HashMap::new();
            let mut tally =
                |chunk: &ArrayRef, source: &mut dyn Iterator<Item = usize>| -> Result<()> {
                    context.check()?;
                    let converted = converter.convert_columns(std::slice::from_ref(chunk))?;
                    for (i, (row, r)) in converted.iter().zip(source).enumerate() {
                        context.check_at(i)?;
                        let mut h = DefaultHasher::new();
                        row.as_ref().hash(&mut h);
                        counts.entry(h.finish()).or_insert((0, r as u32)).0 += 1;
                    }
                    Ok(())
                };
            match rows {
                Selection::All(n) => {
                    for start in (0..*n).step_by(CHUNK) {
                        let len = CHUNK.min(n - start);
                        tally(&array.slice(start, len), &mut (start..start + len))?;
                    }
                }
                Selection::Rows(rows) => {
                    for chunk in rows.chunks(CHUNK) {
                        context.check()?;
                        let taken = take(array.as_ref(), &UInt32Array::from(chunk.to_vec()), None)?;
                        tally(&taken, &mut chunk.iter().map(|&r| r as usize))?;
                    }
                }
            }
            counts
        }
    };

    let distinct = counts.len() as u64;
    // Keep only the requested candidates, not a second collection of every
    // distinct value. Higher counts and earlier source rows win ties.
    let mut candidates = BinaryHeap::with_capacity(keep.min(counts.len()));
    for (i, (count, row)) in counts.into_values().enumerate() {
        context.check_at(i)?;
        let candidate = Reverse((count, Reverse(row)));
        if candidates.len() < keep {
            candidates.push(candidate);
        } else if let Some(mut smallest) = candidates.peek_mut() {
            if candidate.0 > smallest.0 {
                *smallest = candidate;
            }
        }
    }
    let top: Vec<(u64, u32)> = candidates
        .into_iter()
        .map(|Reverse((count, Reverse(row)))| (count, row))
        .collect();

    // Back into value order.
    let first_rows = UInt32Array::from(top.iter().map(|&(_, r)| r).collect::<Vec<_>>());
    let taken = take(array.as_ref(), &first_rows, None)?;
    let converted = converter.convert_columns(&[taken])?;
    let mut order: Vec<usize> = (0..top.len()).collect();
    order.sort_by(|&a, &b| converted.row(a).cmp(&converted.row(b)));

    let f = format::formatter(array.as_ref())?;
    let nulls = array.logical_nulls();
    let top = order
        .into_iter()
        .enumerate()
        .map(|(key, i)| DistinctValue {
            key: key as u64,
            label: if nulls
                .as_ref()
                .is_some_and(|nulls| nulls.is_null(top[i].1 as usize))
            {
                None
            } else {
                format::cell(array.as_ref(), &f, top[i].1 as usize)
            },
            count: top[i].0,
        })
        .collect();
    context.check()?;
    Ok(ValueCounts { distinct, top })
}

/// Float equality for analysis groups signed zero and NaN payloads. Native
/// scalar reads avoid a row-format allocation or widened copy of the column.
fn count_floats<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    rows: &Selection,
    context: &AnalysisContext,
) -> Result<HashMap<u64, (u64, u32)>>
where
    T::Native: Into<f64>,
{
    const NAN_KEY: u64 = f64::NAN.to_bits();
    // All source NaNs become NAN_KEY, leaving this payload exclusively for null.
    const NULL_KEY: u64 = NAN_KEY | 1;
    let mut counts = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        context.check_at(index)?;
        let key = if array.is_null(row) {
            NULL_KEY
        } else {
            let value: f64 = array.value(row).into();
            if value == 0.0 {
                0
            } else if value.is_nan() {
                NAN_KEY
            } else {
                value.to_bits()
            }
        };
        counts.entry(key).or_insert((0, row as u32)).0 += 1;
    }
    context.check()?;
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Float64Array, Int32Array, Int64Array, StringArray};

    use super::*;

    #[test]
    fn float_counts_group_signed_zero_and_nan_payloads_separately_from_null() {
        let source = Float64Array::from(vec![
            Some(-0.0),
            Some(0.0),
            Some(f64::NAN),
            Some(f64::from_bits(0x7ff8_1000_0000_0000)),
            Some(f64::from_bits(0xfff8_2000_0000_0000)),
            None,
            None,
            Some(1.0),
        ]);
        for data_type in [DataType::Float16, DataType::Float32, DataType::Float64] {
            let array = arrow::compute::cast(&source, &data_type).unwrap();
            for rows in [
                Selection::All(array.len()),
                Selection::Rows(Arc::new(vec![7, 6, 5, 4, 3, 2, 1, 0])),
            ] {
                let result = count_values(&array, &rows, 8).unwrap();
                assert_eq!(result.distinct, 4);
                assert_eq!(result.top.len(), 4);
                for value in result.top {
                    let expected = match value.label {
                        None => 2,
                        Some(label) => match label.parse::<f64>().unwrap() {
                            value if value.is_nan() => 3,
                            0.0 => 2,
                            1.0 => 1,
                            value => panic!("unexpected value {value}"),
                        },
                    };
                    assert_eq!(value.count, expected);
                }
            }
        }
    }

    #[test]
    fn codes_follow_value_order() {
        let a: ArrayRef = Arc::new(StringArray::from(vec![
            Some("dog"),
            None,
            Some("cat"),
            Some("dog"),
        ]));
        let idx = ValueIndex::build(&a, INDEX_LIMIT).unwrap().unwrap();
        assert_eq!(idx.labels, vec![
            None,
            Some("cat".into()),
            Some("dog".into())
        ]);
        assert_eq!(idx.codes, vec![2, 0, 1, 2]);
    }

    #[test]
    fn numbers_sort_numerically() {
        let a: ArrayRef = Arc::new(Int32Array::from(vec![10, -3, 2, 10]));
        let idx = ValueIndex::build(&a, INDEX_LIMIT).unwrap().unwrap();
        assert_eq!(idx.labels, vec![
            Some("-3".into()),
            Some("2".into()),
            Some("10".into())
        ]);
    }

    #[test]
    fn too_many_values_are_not_indexed() {
        let a: ArrayRef = Arc::new(Int64Array::from_iter_values(0..1000));
        assert!(ValueIndex::build(&a, 999).unwrap().is_none());
        assert!(ValueIndex::build(&a, 1000).unwrap().is_some());
    }

    #[test]
    fn cardinality_probe_counts_null_and_duplicates_at_the_limit() {
        let a: ArrayRef = Arc::new(StringArray::from(vec![
            Some("b"),
            None,
            Some("a"),
            Some("b"),
            None,
        ]));
        assert!(!has_at_most(&a, 2).unwrap());
        assert!(has_at_most(&a, 3).unwrap());
        assert!(has_at_most(&a, 4).unwrap());

        let empty: ArrayRef = Arc::new(Int64Array::from(Vec::<i64>::new()));
        assert!(has_at_most(&empty, 0).unwrap());
        let nulls: ArrayRef = Arc::new(Int64Array::from(vec![None, None]));
        assert!(!has_at_most(&nulls, 0).unwrap());
        assert!(has_at_most(&nulls, 1).unwrap());
    }

    #[test]
    fn cardinality_probe_visits_later_batches() {
        let a: ArrayRef = Arc::new(Int64Array::from_iter_values(
            (0..CHUNK * 2 + 1).map(|row| i64::from(row == CHUNK * 2)),
        ));
        assert!(!has_at_most(&a, 1).unwrap());
        assert!(has_at_most(&a, 2).unwrap());

        let many: ArrayRef = Arc::new(Int64Array::from_iter_values(0..1000));
        assert!(!has_at_most(&many, 999).unwrap());
        assert!(has_at_most(&many, 1000).unwrap());
    }

    #[test]
    fn all_counts_keep_zero_count_options_and_stable_keys() {
        let a: ArrayRef = Arc::new(StringArray::from(vec![
            Some("b"),
            Some("a"),
            None,
            Some("b"),
        ]));
        let index = ValueIndex::build(&a, 3).unwrap().unwrap();
        let selection = Selection::Rows(Arc::new(vec![0, 3]));
        let all = index.counts_all(&selection);
        assert_eq!(all, vec![
            DistinctValue {
                key: 0,
                label: None,
                count: 0
            },
            DistinctValue {
                key: 1,
                label: Some("a".into()),
                count: 0
            },
            DistinctValue {
                key: 2,
                label: Some("b".into()),
                count: 2
            },
        ]);
        assert_eq!(index.counts(&selection), vec![all[2].clone()]);
        let empty = index.counts_all(&Selection::Rows(Arc::new(vec![])));
        assert_eq!(empty.iter().map(|value| value.key).collect::<Vec<_>>(), [
            0, 1, 2
        ]);
        assert!(empty.iter().all(|value| value.count == 0));
        assert_eq!(
            index.counts_all(&Selection::All(a.len())),
            index.counts(&Selection::All(a.len()))
        );
    }

    #[test]
    fn counting_without_an_index() {
        let a: ArrayRef = Arc::new(StringArray::from(vec![
            Some("b"),
            Some("a"),
            None,
            Some("b"),
            Some("c"),
            Some("b"),
            Some("a"),
        ]));
        let all = count_values(&a, &Selection::All(a.len()), 2).unwrap();
        assert_eq!(all.distinct, 4);
        let top: Vec<_> = all
            .top
            .iter()
            .map(|v| (v.label.clone().unwrap(), v.count))
            .collect();
        assert_eq!(top, [("a".to_string(), 2), ("b".to_string(), 3)]);

        let some = count_values(&a, &Selection::Rows(Arc::new(vec![2, 4])), 10).unwrap();
        assert_eq!(some.distinct, 2);
        assert_eq!(some.top[0].label, None);
    }
}
