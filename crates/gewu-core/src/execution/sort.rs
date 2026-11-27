//! Columnar multi-key sort of source row IDs.
//!
//! The input is a filtered view's source-ordered row IDs plus one source column per
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

use crate::column::Dictionary;

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

/// Sort a filtered view's ascending source IDs in place. Stable runs give tied
/// values their original source order, including when refining later keys.
pub(crate) fn sort_source_rows(rows: &mut [u32], keys: &[SortKey<'_>]) -> Result<(), ArrowError> {
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
            sort_source_rows(&mut rows[run], rest)?;
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
    let converter = match RowConverter::new(fields) {
        Ok(converter) => converter,
        Err(ArrowError::NotYetImplemented(_)) => return sort_comparators(rows, keys),
        Err(error) => return Err(error),
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
#[path = "sort_tests.rs"]
mod tests;
