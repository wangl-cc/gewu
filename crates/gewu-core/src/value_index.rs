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
    collections::{hash_map::DefaultHasher, HashMap},
    hash::{Hash, Hasher},
};

use arrow::{
    array::{Array, ArrayRef, UInt32Array},
    compute::take,
    row::{RowConverter, SortField},
};

use crate::{format, scope::Selection, DistinctValue, Result};

/// Most distinct values a column may have and still be indexed.
pub(crate) const INDEX_LIMIT: usize = 100_000;

/// Rows converted to row format at a time, to bound memory.
const CHUNK: usize = 64 * 1024;

pub(crate) struct ValueIndex {
    /// Code of each source row.
    pub codes: Vec<u32>,
    /// Display label of each code, `None` for null.
    pub labels: Vec<Option<String>>,
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
            .filter(|&(_, n)| n > 0)
            .map(|(code, count)| DistinctValue {
                key: code as u64,
                label: self.labels[code].clone(),
                count,
            })
            .collect()
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
/// values) would merge two values.
pub(crate) fn count_values(array: &ArrayRef, rows: &Selection, keep: usize) -> Result<ValueCounts> {
    let converter = RowConverter::new(vec![SortField::new(array.data_type().clone())])?;
    // hash -> (count, first row)
    let mut counts: HashMap<u64, (u64, u32)> = HashMap::new();
    let mut tally = |chunk: &ArrayRef, source: &mut dyn Iterator<Item = usize>| -> Result<()> {
        let converted = converter.convert_columns(std::slice::from_ref(chunk))?;
        for (row, r) in converted.iter().zip(source) {
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
                let taken = take(array.as_ref(), &UInt32Array::from(chunk.to_vec()), None)?;
                tally(&taken, &mut chunk.iter().map(|&r| r as usize))?;
            }
        }
    }

    let distinct = counts.len() as u64;
    let mut top: Vec<(u64, u32)> = counts.into_values().collect();
    top.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    top.truncate(keep);

    // Back into value order.
    let first_rows = UInt32Array::from(top.iter().map(|&(_, r)| r).collect::<Vec<_>>());
    let taken = take(array.as_ref(), &first_rows, None)?;
    let converted = converter.convert_columns(&[taken])?;
    let mut order: Vec<usize> = (0..top.len()).collect();
    order.sort_by(|&a, &b| converted.row(a).cmp(&converted.row(b)));

    let f = format::formatter(array.as_ref())?;
    let top = order
        .into_iter()
        .enumerate()
        .map(|(key, i)| DistinctValue {
            key: key as u64,
            label: format::cell(array.as_ref(), &f, top[i].1 as usize),
            count: top[i].0,
        })
        .collect();
    Ok(ValueCounts { distinct, top })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Int32Array, Int64Array, StringArray};

    use super::*;

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
