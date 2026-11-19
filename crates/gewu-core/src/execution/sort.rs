//! Columnar multi-key sort of source row IDs.
//!
//! The input is a set of selected source row IDs plus one source column per
//! key; the output is the same IDs in display order. Equal keys always keep
//! ascending source-row order, so the result is deterministic and matches
//! Arrow's `lexsort_to_indices` with the row ID appended as a final key.
//!
//! Each key is sorted by a kernel chosen for its column, never by a
//! per-pair dynamic comparator over every key:
//! - Columns with a compact [`Dictionary`] counting-sort its `u8` rank codes.
//! - Primitive columns sort native `(value, row)` pairs.
//! - Text and binary columns sort inline `(16-byte prefix, length, row)` keys and read the full
//!   value only when long values share a prefix.
//!
//! The leading key is sorted first. Each run of equal leading values is then
//! sorted by the remaining keys, so later keys only touch tied rows. Types
//! without a dedicated kernel encode all remaining keys once with Arrow's
//! row format and compare bytes.

use std::{cmp::Ordering, ops::Range};

use arrow::{
    array::{
        downcast_primitive_array, make_comparator, Array, ArrayRef, AsArray, GenericByteArray,
        GenericByteViewArray, PrimitiveArray, UInt32Array,
    },
    buffer::NullBuffer,
    compute::{take, SortOptions},
    datatypes::{ArrowNativeTypeOp, ArrowPrimitiveType, ByteArrayType, ByteViewType, DataType},
    error::ArrowError,
    row::{RowConverter, SortField},
};

use crate::{column::Dictionary, row_ids::RowSlice};

/// One sort key: a source column and its direction. Nulls always sort last.
#[derive(Clone, Copy)]
pub(crate) struct SortKey<'a> {
    pub(crate) array: &'a ArrayRef,
    pub(crate) descending: bool,
    /// Rank codes for the column, set only when equal codes mean equal Arrow
    /// comparisons (see [`codes_match_arrow_order`]).
    pub(crate) codes: Option<&'a Dictionary>,
}

impl SortKey<'_> {
    fn options(&self) -> SortOptions {
        SortOptions {
            descending: self.descending,
            nulls_first: false,
        }
    }
}

/// Whether a column's [`Dictionary`] codes order rows exactly as Arrow's
/// comparator does. Float dictionaries merge `-0.0`/`0.0` and NaN payloads,
/// which Arrow's total order keeps distinct, so they are excluded.
pub(crate) fn codes_match_arrow_order(data_type: &DataType) -> bool {
    use DataType::*;
    match data_type {
        Dictionary(_, values) => codes_match_arrow_order(values),
        Float16 | Float32 | Float64 => false,
        Null | Boolean | Utf8 | LargeUtf8 | Utf8View | Binary | LargeBinary | BinaryView => true,
        other => other.is_primitive(),
    }
}

pub(crate) fn sort_rows(rows: RowSlice<'_>, keys: &[SortKey<'_>]) -> Result<Vec<u32>, ArrowError> {
    let mut sorted: Vec<u32> = rows.iter_u32().collect();
    // Every kernel is stable over its input, so starting from ascending IDs
    // gives the source-row tie-break without a separate pass.
    if !sorted.is_sorted() {
        sorted.sort_unstable();
    }
    sort_ascending(&mut sorted, keys)?;
    Ok(sorted)
}

/// Sort `rows`, which hold distinct source IDs in ascending order, by `keys`.
fn sort_ascending(rows: &mut [u32], keys: &[SortKey<'_>]) -> Result<(), ArrowError> {
    let Some((key, rest)) = keys.split_first() else {
        return Ok(());
    };
    if rows.len() <= 1 {
        return Ok(());
    }
    if let Some(codes) = key.codes {
        let runs = sort_codes(rows, codes, key.descending);
        return refine(rows, runs, rest);
    }
    let array = key.array.as_ref();
    let runs = downcast_primitive_array!(
        array => sort_primitive(rows, array, key.descending),
        DataType::Utf8 => sort_bytes(rows, array.as_string::<i32>(), key.descending),
        DataType::LargeUtf8 => sort_bytes(rows, array.as_string::<i64>(), key.descending),
        DataType::Binary => sort_bytes(rows, array.as_binary::<i32>(), key.descending),
        DataType::LargeBinary => sort_bytes(rows, array.as_binary::<i64>(), key.descending),
        DataType::Utf8View => sort_bytes(rows, array.as_string_view(), key.descending),
        DataType::BinaryView => sort_bytes(rows, array.as_binary_view(), key.descending),
        _ => return sort_row_format(rows, keys),
    );
    refine(rows, runs, rest)
}

/// Sort each run of rows tied on the previous keys by the remaining keys.
fn refine(
    rows: &mut [u32],
    runs: Vec<Range<usize>>,
    rest: &[SortKey<'_>],
) -> Result<(), ArrowError> {
    if rest.is_empty() {
        return Ok(());
    }
    for run in runs {
        if run.len() > 1 {
            sort_ascending(&mut rows[run], rest)?;
        }
    }
    Ok(())
}

/// Collect `[start, end)` ranges of adjacent equal items.
fn equal_runs<T>(items: &[T], equal: impl Fn(&T, &T) -> bool) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut start = 0;
    for end in 1..=items.len() {
        if end == items.len() || !equal(&items[start], &items[end]) {
            if end - start > 1 {
                runs.push(start..end);
            }
            start = end;
        }
    }
    runs
}

/// Stable counting sort on dictionary rank codes, nulls last.
fn sort_codes(rows: &mut [u32], codes: &Dictionary, descending: bool) -> Vec<Range<usize>> {
    let n_values = codes.values().len();
    let null_bucket = n_values;
    let bucket = |row: u32| -> usize {
        let row = row as usize;
        if codes.is_null(row) {
            null_bucket
        } else if descending {
            n_values - 1 - codes.code_at(row) as usize
        } else {
            codes.code_at(row) as usize
        }
    };
    let mut offsets = vec![0usize; n_values + 2];
    for &row in rows.iter() {
        offsets[bucket(row) + 1] += 1;
    }
    for i in 1..offsets.len() {
        offsets[i] += offsets[i - 1];
    }
    let runs = offsets
        .windows(2)
        .filter(|w| w[1] - w[0] > 1)
        .map(|w| w[0]..w[1])
        .collect();
    let mut out = vec![0u32; rows.len()];
    let mut next = offsets;
    for &row in rows.iter() {
        let slot = &mut next[bucket(row)];
        out[*slot] = row;
        *slot += 1;
    }
    rows.copy_from_slice(&out);
    runs
}

/// Move null rows to the end, keeping each part in input order. Returns the
/// number of non-null rows.
fn partition_nulls(rows: &mut [u32], nulls: Option<&NullBuffer>) -> usize {
    let Some(nulls) = nulls.filter(|nulls| nulls.null_count() > 0) else {
        return rows.len();
    };
    let mut null_rows = Vec::new();
    let mut valid = 0;
    for i in 0..rows.len() {
        let row = rows[i];
        if nulls.is_null(row as usize) {
            null_rows.push(row);
        } else {
            rows[valid] = row;
            valid += 1;
        }
    }
    rows[valid..].copy_from_slice(&null_rows);
    valid
}

/// Runs over the sorted non-null prefix plus the trailing null run.
fn with_null_run(mut runs: Vec<Range<usize>>, valid: usize, len: usize) -> Vec<Range<usize>> {
    if len - valid > 1 {
        runs.push(valid..len);
    }
    runs
}

/// Sort native `(value, row)` pairs with Arrow's total order for the type.
fn sort_primitive<T: ArrowPrimitiveType>(
    rows: &mut [u32],
    array: &PrimitiveArray<T>,
    descending: bool,
) -> Vec<Range<usize>> {
    let valid = partition_nulls(rows, array.nulls());
    let values = array.values();
    let mut pairs: Vec<(T::Native, u32)> = rows[..valid]
        .iter()
        .map(|&row| (values[row as usize], row))
        .collect();
    if descending {
        pairs.sort_unstable_by(|a, b| b.0.compare(a.0).then(a.1.cmp(&b.1)));
    } else {
        pairs.sort_unstable_by(|a, b| a.0.compare(b.0).then(a.1.cmp(&b.1)));
    }
    for (slot, &(_, row)) in rows.iter_mut().zip(&pairs) {
        *slot = row;
    }
    let runs = equal_runs(&pairs, |a, b| a.0.is_eq(b.0));
    with_null_run(runs, valid, rows.len())
}

/// Byte-addressable arrays: offset-based and view-based text and binary.
trait Bytes: Array {
    fn bytes(&self, row: usize) -> &[u8];
}

impl<T: ByteArrayType> Bytes for GenericByteArray<T> {
    #[inline]
    fn bytes(&self, row: usize) -> &[u8] {
        self.value(row).as_ref()
    }
}

impl<T: ByteViewType + ?Sized> Bytes for GenericByteViewArray<T> {
    #[inline]
    fn bytes(&self, row: usize) -> &[u8] {
        self.value(row).as_ref()
    }
}

/// Values up to this many bytes are compared without reading the array.
const INLINE: usize = 16;

/// Inline sort key for a byte value: its big-endian first [`INLINE`] bytes,
/// zero padded, and its length. Comparing `(prefix, len)` agrees with byte
/// order whenever both values fit inline; otherwise a prefix tie needs the
/// full values.
#[derive(Clone, Copy)]
struct ByteKey {
    /// `[u64; 2]` rather than `u128` keeps the key at 24 bytes.
    prefix: [u64; 2],
    len: u32,
    row: u32,
}

impl ByteKey {
    #[inline]
    fn new(bytes: &[u8], row: u32) -> Self {
        let mut buf = [0u8; INLINE];
        let n = bytes.len().min(INLINE);
        buf[..n].copy_from_slice(&bytes[..n]);
        Self {
            prefix: [
                u64::from_be_bytes(buf[..8].try_into().unwrap()),
                u64::from_be_bytes(buf[8..].try_into().unwrap()),
            ],
            len: bytes.len().min(u32::MAX as usize) as u32,
            row,
        }
    }

    #[inline]
    fn inline(&self) -> bool {
        self.len as usize <= INLINE
    }

    /// Byte order of the two values, reading `array` only on long prefix ties.
    #[inline]
    fn value_cmp<A: Bytes>(&self, other: &Self, array: &A) -> Ordering {
        match self.prefix.cmp(&other.prefix) {
            Ordering::Equal if self.inline() && other.inline() => self.len.cmp(&other.len),
            Ordering::Equal => array
                .bytes(self.row as usize)
                .cmp(array.bytes(other.row as usize)),
            order => order,
        }
    }
}

/// Stable sort of inline byte keys in byte order; rows arrive ascending, so
/// equal values keep source order. Arrow compares strings by their UTF-8
/// bytes too.
fn sort_bytes<A: Bytes>(rows: &mut [u32], array: &A, descending: bool) -> Vec<Range<usize>> {
    let valid = partition_nulls(rows, array.nulls());
    // Distinct leading words are common (IDs), so sort compact
    // `(first eight bytes, row)` pairs, then order each tied run by the full
    // value. Runs arrive in source order, so the stable second pass keeps the
    // tie-break. Wider keys exist only for one tied run at a time.
    let mut pairs: Vec<(u64, u32)> = rows[..valid]
        .iter()
        .map(|&row| (prefix_word(array.bytes(row as usize)), row))
        .collect();
    if descending {
        pairs.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    } else {
        pairs.sort_unstable();
    }
    for (slot, &(_, row)) in rows.iter_mut().zip(&pairs) {
        *slot = row;
    }
    let tied = equal_runs(&pairs, |a, b| a.0 == b.0);
    drop(pairs);
    let mut runs = Vec::new();
    let mut keys = Vec::new();
    for Range { start, end } in tied {
        keys.clear();
        keys.extend(
            rows[start..end]
                .iter()
                .map(|&row| ByteKey::new(array.bytes(row as usize), row)),
        );
        if descending {
            keys.sort_by(|a, b| b.value_cmp(a, array));
        } else {
            keys.sort_by(|a, b| a.value_cmp(b, array));
        }
        for (slot, key) in rows[start..end].iter_mut().zip(&keys) {
            *slot = key.row;
        }
        runs.extend(
            equal_runs(&keys, |a, b| a.value_cmp(b, array).is_eq())
                .into_iter()
                .map(|run| run.start + start..run.end + start),
        );
    }
    with_null_run(runs, valid, rows.len())
}

/// Big-endian first eight bytes, zero padded: the leading word of [`ByteKey`].
#[inline]
fn prefix_word(bytes: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    let n = bytes.len().min(8);
    buf[..n].copy_from_slice(&bytes[..n]);
    u64::from_be_bytes(buf)
}

/// Encode every key once with Arrow's row format and sort by bytes. Falls
/// back to Arrow's comparators for types the row format cannot encode.
fn sort_row_format(rows: &mut [u32], keys: &[SortKey<'_>]) -> Result<(), ArrowError> {
    let fields = keys
        .iter()
        .map(|key| SortField::new_with_options(key.array.data_type().clone(), key.options()))
        .collect();
    let Ok(converter) = RowConverter::new(fields) else {
        return sort_comparators(rows, keys);
    };
    let ids = UInt32Array::from(rows.to_vec());
    let columns = keys
        .iter()
        .map(|key| take(key.array.as_ref(), &ids, None))
        .collect::<Result<Vec<_>, _>>()?;
    let encoded = converter.convert_columns(&columns)?;
    let mut order: Vec<u32> = (0..rows.len() as u32).collect();
    // Positions follow ascending source IDs, so they double as the tie-break.
    order.sort_unstable_by(|&a, &b| {
        encoded
            .row(a as usize)
            .cmp(&encoded.row(b as usize))
            .then(a.cmp(&b))
    });
    for (slot, position) in rows.iter_mut().zip(order) {
        *slot = ids.value(position as usize);
    }
    Ok(())
}

fn sort_comparators(rows: &mut [u32], keys: &[SortKey<'_>]) -> Result<(), ArrowError> {
    let comparators = keys
        .iter()
        .map(|key| make_comparator(key.array.as_ref(), key.array.as_ref(), key.options()))
        .collect::<Result<Vec<_>, _>>()?;
    rows.sort_unstable_by(|&left, &right| {
        comparators
            .iter()
            .map(|compare| compare(left as usize, right as usize))
            .find(|order| *order != Ordering::Equal)
            .unwrap_or_else(|| left.cmp(&right))
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{
            BooleanArray, DictionaryArray, Float64Array, Int64Array, Int8Array, ListArray,
            StringArray, StringViewArray, UInt32Array,
        },
        compute::{lexsort_to_indices, take, SortColumn},
        datatypes::{Int32Type, Int8Type},
    };

    use super::*;

    /// Sort with every applicable kernel and compare against Arrow lexsort
    /// with the source row appended as the final key.
    fn agrees_with_arrow(rows: RowSlice<'_>, columns: &[(&ArrayRef, bool)]) {
        let row_vec: Vec<u32> = rows.iter_u32().collect();
        let selected = UInt32Array::from(row_vec.clone());
        let mut sort_columns: Vec<_> = columns
            .iter()
            .map(|&(array, descending)| SortColumn {
                values: take(array.as_ref(), &selected, None).unwrap(),
                options: Some(SortOptions {
                    descending,
                    nulls_first: false,
                }),
            })
            .collect();
        sort_columns.push(SortColumn {
            values: Arc::new(selected),
            options: None,
        });
        let order = lexsort_to_indices(&sort_columns, None).unwrap();
        let expected: Vec<_> = order
            .values()
            .iter()
            .map(|&index| row_vec[index as usize])
            .collect();

        let dictionaries: Vec<_> = columns
            .iter()
            .map(|(array, _)| {
                codes_match_arrow_order(array.data_type())
                    .then(|| Dictionary::build(array).unwrap())
                    .flatten()
            })
            .collect();
        let keys = |use_codes: bool| -> Vec<SortKey<'_>> {
            columns
                .iter()
                .zip(&dictionaries)
                .map(|(&(array, descending), dictionary)| SortKey {
                    array,
                    descending,
                    codes: dictionary.as_ref().filter(|_| use_codes),
                })
                .collect()
        };
        assert_eq!(sort_rows(rows, &keys(true)).unwrap(), expected, "codes");
        assert_eq!(sort_rows(rows, &keys(false)).unwrap(), expected, "native");
        let mut fallback: Vec<u32> = row_vec.clone();
        fallback.sort_unstable();
        sort_row_format(&mut fallback, &keys(false)).unwrap();
        assert_eq!(fallback, expected, "row format");
        let mut fallback: Vec<u32> = row_vec;
        fallback.sort_unstable();
        sort_comparators(&mut fallback, &keys(false)).unwrap();
        assert_eq!(fallback, expected, "comparators");
    }

    #[test]
    fn kernels_preserve_arrow_native_and_multikey_order() {
        let len = 12_411;
        let numbers: ArrayRef = Arc::new(Int64Array::from_iter((0..len).map(|row| {
            [
                Some(i64::MAX),
                Some(9_007_199_254_740_993),
                None,
                Some(-1),
                Some(i64::MIN),
                Some(0),
            ][row % 6]
        })));
        let floats: ArrayRef = Arc::new(Float64Array::from_iter((0..len).map(|row| {
            [
                Some(f64::NAN),
                Some(-f64::NAN),
                Some(f64::INFINITY),
                Some(f64::NEG_INFINITY),
                Some(-0.0),
                Some(0.0),
                None,
                Some(1.5),
            ][row % 8]
        })));
        let strings: ArrayRef = Arc::new(StringArray::from_iter((0..len).map(|row| {
            [
                Some("z"),
                Some(""),
                None,
                Some("é"),
                Some("e\u{301}"),
                Some("A"),
                Some("abcdefgh"),
                Some("abcdefgh\0"),
                Some("0123456789abcdef"),
                Some("0123456789abcdef\0"),
                Some("0123456789abcdefg"),
                Some("0123456789abcde"),
            ][row % 12]
        })));
        // Distinct values beyond the dictionary limit exercise prefix ties.
        let wide: ArrayRef = Arc::new(StringViewArray::from_iter((0..len).map(|row| {
            (row % 13 != 0).then(|| format!("shared-prefix-{:05}", (row * 7919) % 3_001))
        })));
        let dictionary: ArrayRef = Arc::new(
            DictionaryArray::<Int8Type>::try_new(
                Int8Array::from_iter(
                    (0..len).map(|row| [Some(0), Some(1), Some(2), None, Some(3)][row % 5]),
                ),
                Arc::new(StringArray::from(vec![
                    Some("z"),
                    None,
                    Some("a"),
                    Some("a"),
                ])),
            )
            .unwrap(),
        );
        let booleans: ArrayRef = Arc::new(BooleanArray::from_iter(
            (0..len).map(|row| [Some(true), None, Some(false)][row % 3]),
        ));
        let lists: ArrayRef = Arc::new(ListArray::from_iter_primitive::<Int32Type, _, _>(
            (0..len).map(|row| (row % 4 != 0).then(|| vec![Some((row % 7) as i32), None])),
        ));
        // A sparse, reversed selection verifies comparisons use source IDs,
        // rather than positions in the selected view.
        let rows: Vec<_> = (0..len as u32).rev().filter(|row| row % 3 != 0).collect();
        for array in [
            &numbers,
            &floats,
            &strings,
            &wide,
            &dictionary,
            &booleans,
            &lists,
        ] {
            for descending in [false, true] {
                agrees_with_arrow(RowSlice::Selected(&rows), &[(array, descending)]);
                agrees_with_arrow(RowSlice::All(len), &[(array, descending)]);
            }
        }
        for rows in [RowSlice::Selected(&rows), RowSlice::All(len)] {
            agrees_with_arrow(rows, &[
                (&dictionary, true),
                (&floats, false),
                (&numbers, true),
            ]);
            agrees_with_arrow(rows, &[(&floats, true), (&strings, false)]);
            agrees_with_arrow(rows, &[(&wide, false), (&booleans, true)]);
            agrees_with_arrow(rows, &[
                (&strings, true),
                (&lists, false),
                (&numbers, false),
            ]);
            agrees_with_arrow(rows, &[
                (&booleans, false),
                (&dictionary, false),
                (&wide, true),
            ]);
        }
    }

    #[test]
    fn primitive_sort_keeps_random_repeated_sorted_and_sparse_selections_exact() {
        let len = 70_000usize;
        for pattern in 0..4 {
            let array: ArrayRef = Arc::new(Int64Array::from_iter((0..len).map(|row| {
                if pattern == 1 && row % 97 == 0 {
                    None
                } else {
                    Some(match pattern {
                        1 => (row % 17) as i64,
                        2 => row as i64,
                        _ => ((row * 104729) % len) as i64,
                    })
                }
            })));
            if pattern == 3 {
                let rows: Vec<u32> = (0..len as u32).rev().filter(|row| row % 3 != 0).collect();
                agrees_with_arrow(RowSlice::Selected(&rows), &[(&array, false)]);
            } else {
                agrees_with_arrow(RowSlice::All(len), &[(&array, false)]);
            }
        }
    }

    #[test]
    fn empty_single_and_repeated_selections_preserve_every_row_once() {
        let array: ArrayRef = Arc::new(Int64Array::from_iter_values(
            (0..5_000).map(|row| (row % 17) as i64),
        ));
        for len in [0, 1, 2, 17, 5_000] {
            let rev: Vec<u32> = (0..len as u32).rev().collect();
            agrees_with_arrow(RowSlice::Selected(&rev), &[(&array, false)]);
            agrees_with_arrow(RowSlice::All(len), &[(&array, false)]);
        }
    }
}
