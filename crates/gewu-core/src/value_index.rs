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
//!
//! Values are told apart by their bytes: the native value of a number, date
//! or time (bitwise, so `-0.0`, `0.0` and each NaN differ, like Arrow's sort
//! order), the UTF-8 of a string, the value a dictionary key points to.
//! Other types (lists, structs, ...) are converted to Arrow's row format
//! first, a chunk at a time. Only the distinct values are then sorted, with
//! Arrow's own comparator.

use arrow::{
    array::{downcast_primitive_array, make_comparator, Array, ArrayRef, AsArray, UInt32Array},
    compute::{take, SortOptions},
    datatypes::DataType,
    row::{RowConverter, SortField},
};

use crate::{
    format, hash,
    scope::{for_rows, Selection},
    DistinctValue, Result,
};

/// Most distinct values a column may have and still be indexed.
pub(crate) const INDEX_LIMIT: usize = 100_000;

/// Rows converted to row format at a time, to bound memory.
const CHUNK: usize = 64 * 1024;

/// The bytes that identify row `r`'s value, `None` for null.
type ValueBytes<'a> = Box<dyn Fn(usize) -> Option<&'a [u8]> + 'a>;

/// Direct access to each row's identifying bytes, for the types that have
/// them without conversion.
fn value_bytes(array: &dyn Array) -> Option<ValueBytes<'_>> {
    let data_type = array.data_type();
    if data_type.is_primitive() {
        let width = data_type.primitive_width()?;
        let bytes: &[u8] = downcast_primitive_array!(
            array => array.values().inner().as_slice(),
            _ => return None
        );
        return Some(Box::new(move |r| {
            array
                .is_valid(r)
                .then(|| &bytes[r * width..(r + 1) * width])
        }));
    }
    Some(match data_type {
        DataType::Utf8 => {
            let a = array.as_string::<i32>();
            Box::new(move |r| a.is_valid(r).then(|| a.value(r).as_bytes()))
        }
        DataType::LargeUtf8 => {
            let a = array.as_string::<i64>();
            Box::new(move |r| a.is_valid(r).then(|| a.value(r).as_bytes()))
        }
        DataType::Utf8View => {
            let a = array.as_string_view();
            Box::new(move |r| a.is_valid(r).then(|| a.value(r).as_bytes()))
        }
        DataType::Binary => {
            let a = array.as_binary::<i32>();
            Box::new(move |r| a.is_valid(r).then(|| a.value(r)))
        }
        DataType::LargeBinary => {
            let a = array.as_binary::<i64>();
            Box::new(move |r| a.is_valid(r).then(|| a.value(r)))
        }
        DataType::Dictionary(..) => {
            let dict = array.as_any_dictionary();
            let keys = dict.normalized_keys();
            let values = value_bytes(dict.values().as_ref())?;
            Box::new(move |r| {
                if array.is_valid(r) {
                    values(keys[r])
                } else {
                    None
                }
            })
        }
        _ => return None,
    })
}

pub(crate) struct ValueIndex {
    /// Code of each source row.
    pub codes: Vec<u32>,
    /// Display label of each code, `None` for null.
    pub labels: Vec<Option<String>>,
}

/// Hands out codes in order of first appearance, up to a limit.
struct Codes {
    /// First row of each code.
    first_row: Vec<usize>,
    limit: usize,
}

impl Codes {
    /// A code for a value first seen at `row`; `None` past the limit.
    fn next(&mut self, row: usize) -> Option<u32> {
        if self.first_row.len() == self.limit {
            return None;
        }
        self.first_row.push(row);
        Some(self.first_row.len() as u32 - 1)
    }
}

impl ValueIndex {
    /// Index `array`, or `None` if it has more than `limit` distinct values
    /// (null counts as one).
    pub fn build(array: &ArrayRef, limit: usize) -> Result<Option<Self>> {
        let Some(bytes) = value_bytes(array.as_ref()) else {
            return Self::build_from_rows(array, limit);
        };
        let mut seen: hash::Map<&[u8], u32> = hash::Map::default();
        let mut null = None;
        let mut new = Codes {
            first_row: Vec::new(),
            limit,
        };
        let mut codes = Vec::with_capacity(array.len());
        for r in 0..array.len() {
            let code = match bytes(r) {
                Some(b) => match seen.get(b) {
                    Some(&c) => c,
                    None => {
                        let Some(c) = new.next(r) else {
                            return Ok(None);
                        };
                        seen.insert(b, c);
                        c
                    }
                },
                None => match null {
                    Some(c) => c,
                    None => {
                        let Some(c) = new.next(r) else {
                            return Ok(None);
                        };
                        *null.insert(c)
                    }
                },
            };
            codes.push(code);
        }

        // Only the distinct values are sorted, by their first rows.
        let first_row = new.first_row;
        let cmp = make_comparator(array.as_ref(), array.as_ref(), SortOptions {
            descending: false,
            nulls_first: true,
        })?;
        let mut order: Vec<u32> = (0..first_row.len() as u32).collect();
        order.sort_unstable_by(|&a, &b| cmp(first_row[a as usize], first_row[b as usize]));
        Self::finish(array, codes, &first_row, &order).map(Some)
    }

    /// The general path: tell values apart by their Arrow row format,
    /// converted a chunk at a time.
    fn build_from_rows(array: &ArrayRef, limit: usize) -> Result<Option<Self>> {
        let converter = RowConverter::new(vec![SortField::new(array.data_type().clone())])?;
        let mut seen: hash::Map<Box<[u8]>, u32> = hash::Map::default();
        let mut new = Codes {
            first_row: Vec::new(),
            limit,
        };
        let mut codes = Vec::with_capacity(array.len());
        for start in (0..array.len()).step_by(CHUNK) {
            let len = CHUNK.min(array.len() - start);
            let rows = converter.convert_columns(&[array.slice(start, len)])?;
            for (i, row) in rows.iter().enumerate() {
                let bytes: &[u8] = row.as_ref();
                let code = match seen.get(bytes) {
                    Some(&c) => c,
                    None => {
                        let Some(c) = new.next(start + i) else {
                            return Ok(None);
                        };
                        seen.insert(bytes.into(), c);
                        c
                    }
                };
                codes.push(code);
            }
        }
        // Row-format bytes sort like the values, nulls first.
        let mut by_bytes: Vec<(&[u8], u32)> = seen.iter().map(|(b, &c)| (&b[..], c)).collect();
        by_bytes.sort_unstable();
        let order: Vec<u32> = by_bytes.into_iter().map(|(_, c)| c).collect();
        Self::finish(array, codes, &new.first_row, &order).map(Some)
    }

    /// Renumber codes so that `order` (codes in value order) becomes
    /// 0, 1, 2, ..., and label them.
    fn finish(
        array: &ArrayRef,
        mut codes: Vec<u32>,
        first_row: &[usize],
        order: &[u32],
    ) -> Result<Self> {
        let mut remap = vec![0u32; order.len()];
        for (new, &old) in order.iter().enumerate() {
            remap[old as usize] = new as u32;
        }
        for c in codes.iter_mut() {
            *c = remap[*c as usize];
        }
        let f = format::formatter(array.as_ref())?;
        let labels = order
            .iter()
            .map(|&old| format::cell(array.as_ref(), &f, first_row[old as usize]))
            .collect();
        Ok(Self { codes, labels })
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
/// most common. Values are told apart by a 64-bit fingerprint, so memory is
/// a few words per distinct value; a collision (unlikely below billions of
/// values) would merge two values.
pub(crate) fn count_values(array: &ArrayRef, rows: &Selection, keep: usize) -> Result<ValueCounts> {
    // fingerprint -> (count, first row); nulls counted apart.
    let mut counts: hash::Map<u64, (u64, u32)> = hash::Map::default();
    let mut nulls: Option<(u64, u32)> = None;
    match value_bytes(array.as_ref()) {
        Some(bytes) => for_rows!(rows, |it| {
            for r in it {
                let slot = match bytes(r) {
                    Some(b) => counts.entry(hash::fingerprint(b)).or_insert((0, r as u32)),
                    None => nulls.get_or_insert((0, r as u32)),
                };
                slot.0 += 1;
            }
        }),
        None => {
            // Row format, a chunk at a time; nulls get their own bytes.
            let converter = RowConverter::new(vec![SortField::new(array.data_type().clone())])?;
            let mut tally =
                |chunk: &ArrayRef, source: &mut dyn Iterator<Item = usize>| -> Result<()> {
                    let converted = converter.convert_columns(std::slice::from_ref(chunk))?;
                    for (row, r) in converted.iter().zip(source) {
                        counts
                            .entry(hash::fingerprint(row.as_ref()))
                            .or_insert((0, r as u32))
                            .0 += 1;
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
        }
    }

    let mut top: Vec<(u64, u32)> = counts.into_values().chain(nulls).collect();
    let distinct = top.len() as u64;
    top.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    top.truncate(keep);

    // Back into value order, nulls first.
    let cmp = make_comparator(array.as_ref(), array.as_ref(), SortOptions {
        descending: false,
        nulls_first: true,
    });
    if let Ok(cmp) = cmp {
        top.sort_by(|a, b| cmp(a.1 as usize, b.1 as usize));
    } else {
        let converter = RowConverter::new(vec![SortField::new(array.data_type().clone())])?;
        let first_rows = UInt32Array::from(top.iter().map(|&(_, r)| r).collect::<Vec<_>>());
        let converted = converter.convert_columns(&[take(array.as_ref(), &first_rows, None)?])?;
        let mut order: Vec<usize> = (0..top.len()).collect();
        order.sort_by(|&a, &b| converted.row(a).cmp(&converted.row(b)));
        top = order.into_iter().map(|i| top[i]).collect();
    }

    let f = format::formatter(array.as_ref())?;
    let top = top
        .into_iter()
        .enumerate()
        .map(|(key, (count, row))| DistinctValue {
            key: key as u64,
            label: format::cell(array.as_ref(), &f, row as usize),
            count,
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

    /// The direct path must agree with the row-format path on every type
    /// it handles, including sliced arrays and odd floats.
    #[test]
    fn direct_bytes_match_row_format() {
        use arrow::{
            array::{
                Date32Array, DictionaryArray, Float64Array, LargeStringArray, StringViewArray,
                UInt8Array,
            },
            datatypes::Int8Type,
        };
        let floats: ArrayRef = Arc::new(Float64Array::from(vec![
            Some(1.5),
            Some(-0.0),
            None,
            Some(0.0),
            Some(f64::NAN),
            Some(-2.0),
            Some(1.5),
            Some(f64::INFINITY),
        ]));
        let dict: DictionaryArray<Int8Type> =
            vec![Some("b"), None, Some("a"), Some("b"), Some("c")]
                .into_iter()
                .collect();
        // Duplicate values behind different keys must still be one value.
        let dup = DictionaryArray::<Int8Type>::try_new(
            vec![Some(0i8), Some(1), None, Some(2), Some(0)].into(),
            Arc::new(StringArray::from(vec!["x", "y", "x"])),
        )
        .unwrap();
        let arrays: Vec<ArrayRef> = vec![
            floats.clone(),
            floats.slice(2, 5),
            Arc::new(Int64Array::from(vec![Some(-1), Some(3), None, Some(-1)])),
            Arc::new(UInt8Array::from(vec![255, 0, 7, 0])),
            Arc::new(Date32Array::from(vec![Some(19000), None, Some(1)])),
            Arc::new(LargeStringArray::from(vec![
                Some("z"),
                Some(""),
                None,
                Some("z"),
            ])),
            Arc::new(StringViewArray::from(vec![
                Some("a long string beyond twelve bytes"),
                Some("a"),
                Some("a long string beyond twelve bytes"),
            ])),
            Arc::new(dict),
            Arc::new(dup),
            Arc::new(StringArray::from(vec!["q", "p", "q", "r"]).slice(1, 3)),
        ];
        for a in arrays {
            assert!(value_bytes(a.as_ref()).is_some(), "{}", a.data_type());
            let direct = ValueIndex::build(&a, INDEX_LIMIT).unwrap().unwrap();
            let rows = ValueIndex::build_from_rows(&a, INDEX_LIMIT)
                .unwrap()
                .unwrap();
            assert_eq!(direct.labels, rows.labels, "{}", a.data_type());
            assert_eq!(direct.codes, rows.codes, "{}", a.data_type());

            let counted = count_values(&a, &Selection::All(a.len()), usize::MAX).unwrap();
            assert_eq!(counted.distinct as usize, direct.len(), "{}", a.data_type());
            let labels: Vec<_> = counted.top.iter().map(|v| v.label.clone()).collect();
            assert_eq!(labels, direct.labels, "{}", a.data_type());
        }
    }
}
