//! Distinct values of a column as a compact `u8` code space.
//!
//! A [`Dictionary`] gives every source row a code naming its distinct
//! value: codes `0..values.len()` name the non-null values, while row-level
//! nulls are tracked via an Arrow [`NullBuffer`].
//!
//! # Architecture
//!
//! The dictionary is constructed via [`Dictionary::build`]:
//! - Native Arrow dictionary columns reuse their referenced dictionary entries directly.
//! - Canonical primitive and boolean columns scan native values in a single pass via
//!   [`primitive::CanonicalizeArray`].
//! - Text, binary and nested arrays use Arrow's row format to discover distinct byte rows.
//!
//! The column consumes the dictionary through [`Dictionary::filter_choices`] for
//! projected choices and frequency counting, and [`Dictionary::matching_codes`]
//! for filter generation.
//!
//! The dictionary is a pure function of the column data and is lazily computed
//! and cached by [`crate::Column`]. Columns with more than [`MAX_DISTINCT_VALUES`]
//! non-null values are not encoded.

use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::{Array, ArrayRef, AsArray, BooleanBufferBuilder, UInt32Array},
    buffer::NullBuffer,
    compute::take,
    datatypes::*,
    row::{RowConverter, SortField},
};
use rapidhash::RapidHashMap;

use super::{value_text, FilterChoices, ValueChoice};
use crate::{
    predicate::Predicate, scope::Selection, selection::RowSelection, Cancellation, Result,
};

mod primitive;

/// Most non-null values a column may have and still be encoded. Non-null
/// values use codes `0..=254`. Columns with more values are not encoded.
pub(crate) const MAX_DISTINCT_VALUES: usize = 255;

/// Occurrences of each distinct value over a row selection.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Tally {
    pub counts: Vec<u64>,
    pub null_count: u64,
}

pub(crate) struct Dictionary {
    /// Non-null value codes for each source row, naming sorted value ranks `0..values.len()`.
    codes: Vec<u8>,
    /// Row-level null mask. `None` if the column contains no null rows.
    nulls: Option<NullBuffer>,
    /// Unique non-null values in sorted order.
    values: ArrayRef,
    /// Formatted labels matching `values` 1:1, in sorted order.
    labels: Vec<String>,
    /// Precomputed full-table counts, matching `values.len()` 1:1.
    all_counts: Vec<u64>,
}

impl Dictionary {
    pub(crate) fn values(&self) -> &ArrayRef {
        &self.values
    }

    pub(crate) fn matching_codes(
        self: &Arc<Self>,
        matched: &[bool],
        exclude: bool,
        include_nulls: bool,
    ) -> Predicate<'static> {
        let mut allowed = [false; 256];
        for (code, &hit) in matched.iter().enumerate() {
            allowed[code] = hit != exclude;
        }
        self.predicate_from_allowed(allowed, include_nulls)
    }

    pub(crate) fn build(array: &ArrayRef, cancel: &Cancellation) -> Result<Option<Self>> {
        cancel.check()?;
        macro_rules! from_canonical_array {
            ($ty:ident) => {{
                let array = array.as_primitive::<$ty>();
                Self::from_canonical_array(array, cancel)
            }};
        }

        use DataType::*;
        match array.data_type() {
            Dictionary(..) => Self::from_dict_array(array, cancel),
            Boolean => {
                let array = array.as_boolean();
                Self::from_canonical_array(array, cancel)
            }
            Null => Ok(Some(Self::all_null(array))),
            Int8 => from_canonical_array!(Int8Type),
            Int16 => from_canonical_array!(Int16Type),
            Int32 => from_canonical_array!(Int32Type),
            Int64 => from_canonical_array!(Int64Type),
            UInt8 => from_canonical_array!(UInt8Type),
            UInt16 => from_canonical_array!(UInt16Type),
            UInt32 => from_canonical_array!(UInt32Type),
            UInt64 => from_canonical_array!(UInt64Type),
            Float16 => from_canonical_array!(Float16Type),
            Float32 => from_canonical_array!(Float32Type),
            Float64 => from_canonical_array!(Float64Type),
            Date32 => from_canonical_array!(Date32Type),
            Date64 => from_canonical_array!(Date64Type),
            Time32(TimeUnit::Second) => from_canonical_array!(Time32SecondType),
            Time32(TimeUnit::Millisecond) => from_canonical_array!(Time32MillisecondType),
            Time64(TimeUnit::Microsecond) => from_canonical_array!(Time64MicrosecondType),
            Time64(TimeUnit::Nanosecond) => from_canonical_array!(Time64NanosecondType),
            Timestamp(TimeUnit::Second, _) => from_canonical_array!(TimestampSecondType),
            Timestamp(TimeUnit::Millisecond, _) => {
                from_canonical_array!(TimestampMillisecondType)
            }
            Timestamp(TimeUnit::Microsecond, _) => {
                from_canonical_array!(TimestampMicrosecondType)
            }
            Timestamp(TimeUnit::Nanosecond, _) => from_canonical_array!(TimestampNanosecondType),
            Duration(TimeUnit::Second) => from_canonical_array!(DurationSecondType),
            Duration(TimeUnit::Millisecond) => from_canonical_array!(DurationMillisecondType),
            Duration(TimeUnit::Microsecond) => from_canonical_array!(DurationMicrosecondType),
            Duration(TimeUnit::Nanosecond) => from_canonical_array!(DurationNanosecondType),
            Decimal32(..) => from_canonical_array!(Decimal32Type),
            Decimal64(..) => from_canonical_array!(Decimal64Type),
            Decimal128(..) => from_canonical_array!(Decimal128Type),
            Decimal256(..) => from_canonical_array!(Decimal256Type),
            _ => Self::from_row_array(array, cancel),
        }
    }

    /// Build a dictionary from a canonical primitive or boolean array in a single pass.
    /// Values array is assembled directly from canonical native values without sorting or `take`.
    fn from_canonical_array<A>(array: &A, cancel: &Cancellation) -> Result<Option<Self>>
    where
        A: primitive::PrimitiveArray,
    {
        let len = array.len();
        let nulls = array.nulls().cloned();
        let mut seen = RapidHashMap::<A::Key, u8>::default();
        let mut native_of = Vec::new();
        let mut buf = [0u64; 256];

        let mut codes = vec![0u8; len];

        for (i, code) in codes.iter_mut().enumerate() {
            cancel.check_at(i)?;
            if nulls.as_ref().is_some_and(|n| n.is_null(i)) {
                continue;
            }
            let (key, native) = array.canonical_at(i);
            *code = match seen.get(&key) {
                Some(&code) => code,
                None => {
                    if native_of.len() == MAX_DISTINCT_VALUES {
                        return Ok(None);
                    }
                    let code = native_of.len() as u8;
                    seen.insert(key, code);
                    native_of.push(native);
                    code
                }
            };
            buf[*code as usize] += 1;
        }

        let values = array.build_array(native_of.into_iter());
        let all_counts = buf[..values.len()].to_vec();
        Self::sort_and_remap(codes, nulls, values, all_counts, cancel)
    }

    /// Row-format fallback for text, binary and nested arrays: discover
    /// distinct values by hashing rows, then write one `u8` code per row.
    fn from_row_array(array: &ArrayRef, cancel: &Cancellation) -> Result<Option<Self>> {
        /// Rows converted to row format at a time, to bound memory.
        const CHUNK: usize = 64 * 1024;

        let Ok(converter) = RowConverter::new(vec![SortField::new(array.data_type().clone())])
        else {
            return Ok(None);
        };
        let nulls = array.nulls().cloned();
        let mut seen: HashMap<Box<[u8]>, u8> = HashMap::new();
        let mut representative_row: Vec<usize> = Vec::new();
        let mut codes: Vec<u8> = Vec::with_capacity(array.len());
        let mut buf = [0u64; 256];

        for start in (0..array.len()).step_by(CHUNK) {
            cancel.check()?;
            let len = CHUNK.min(array.len() - start);
            let rows = converter.convert_columns(std::slice::from_ref(&array.slice(start, len)))?;
            for (i, row) in rows.iter().enumerate() {
                cancel.check_at(i)?;
                let source_row: usize = start + i;
                if array.is_null(source_row) {
                    codes.push(0);
                    continue;
                }
                let bytes: &[u8] = row.as_ref();
                let code = match seen.get(bytes) {
                    Some(&code) => code,
                    None => {
                        if representative_row.len() == MAX_DISTINCT_VALUES {
                            return Ok(None);
                        }
                        let code = representative_row.len() as u8;
                        seen.insert(bytes.into(), code);
                        representative_row.push(source_row);
                        code
                    }
                };
                buf[code as usize] += 1;
                codes.push(code);
            }
        }

        let rows =
            UInt32Array::from_iter_values(representative_row.into_iter().map(|row| row as u32));
        let values = take(array.as_ref(), &rows, None)?;
        let all_counts = buf[..values.len()].to_vec();
        Self::sort_and_remap(codes, nulls, values, all_counts, cancel)
    }

    /// Encode a native Dictionary. Arrow already encodes the values into
    /// `keys`/`values`, so no row hashing is needed: collect the referenced
    /// dictionary values, reuse [`Self::build`] on that small array, then
    /// walk the keys once to write the shared per-row code array.
    fn from_dict_array(array: &ArrayRef, cancel: &Cancellation) -> Result<Option<Self>> {
        let dictionary = array.as_any_dictionary();
        let keys = dictionary.keys();
        let dictionary_values = dictionary.values().clone();
        if dictionary_values.len() > MAX_DISTINCT_VALUES + 1 {
            return Ok(None);
        }

        let value_nulls = dictionary_values.logical_nulls();
        let mut referenced: Vec<u32> = Vec::new();
        let mut entry_slot = vec![u32::MAX; dictionary_values.len()];
        if !scan_keys(keys, cancel, |key| {
            let Some(entry) = key else {
                return;
            };
            if value_nulls
                .as_ref()
                .is_some_and(|nulls| nulls.is_null(entry))
            {
                return;
            }
            let Some(slot) = entry_slot.get_mut(entry) else {
                return;
            };
            if *slot == u32::MAX {
                *slot = referenced.len() as u32;
                referenced.push(entry as u32);
            }
        })? {
            return Ok(None);
        }

        let referenced_values = take(
            dictionary_values.as_ref(),
            &UInt32Array::from(referenced),
            None,
        )?;
        let Some(encoded) = Self::build(&referenced_values, cancel)? else {
            return Ok(None);
        };

        let mut key_to_code = vec![u8::MAX; dictionary_values.len()];
        for (entry, &slot) in entry_slot.iter().enumerate() {
            if slot != u32::MAX {
                key_to_code[entry] = encoded.codes[slot as usize];
            }
        }

        let mut codes = Vec::with_capacity(array.len());
        let mut buf = [0u64; 256];
        let mut null_bits = BooleanBufferBuilder::new(array.len());
        let mut has_null = false;

        if !scan_keys(keys, cancel, |key| {
            let code =
                key.and_then(|entry| key_to_code.get(entry).copied().filter(|&c| c != u8::MAX));
            match code {
                Some(code) => {
                    null_bits.append(true);
                    buf[code as usize] += 1;
                    codes.push(code);
                }
                None => {
                    null_bits.append(false);
                    has_null = true;
                    codes.push(0);
                }
            }
        })? {
            return Ok(None);
        }

        let nulls = if has_null {
            Some(NullBuffer::new(null_bits.finish()))
        } else {
            None
        };
        let all_counts = buf[..encoded.values.len()].to_vec();
        Ok(Some(Self {
            codes,
            nulls,
            values: encoded.values,
            labels: encoded.labels,
            all_counts,
        }))
    }

    /// A column with no values: every row is null and there is nothing to
    /// enumerate or label.
    fn all_null(array: &ArrayRef) -> Self {
        let len = array.len();
        Self {
            codes: vec![0; len],
            nulls: (len > 0).then(|| NullBuffer::new_null(len)),
            values: array.slice(0, 0),
            labels: Vec::new(),
            all_counts: Vec::new(),
        }
    }

    /// Remap discovered codes and values into Arrow native sorted order,
    /// generating precomputed string labels 1:1 with `values`.
    fn sort_and_remap(
        mut codes: Vec<u8>,
        nulls: Option<NullBuffer>,
        values: ArrayRef,
        all_counts: Vec<u64>,
        cancel: &Cancellation,
    ) -> Result<Option<Self>> {
        cancel.check()?;
        let len = values.len();
        if len <= 1 {
            let labels = if len == 0 {
                Vec::new()
            } else {
                let formatter = crate::format::formatter(values.as_ref())?;
                vec![crate::format::cell(values.as_ref(), &formatter, 0).unwrap_or_default()]
            };
            return Ok(Some(Self {
                codes,
                nulls,
                values,
                labels,
                all_counts,
            }));
        }

        // Some row-encodable nested types cannot use Arrow's single-array sort.
        // They have no compact encoding; this is a completed negative result.
        let Ok(sort_indices) = arrow::compute::sort_to_indices(&values, None, None) else {
            return Ok(None);
        };
        let sorted_values = arrow::compute::take(&values, &sort_indices, None)?;
        let labels = {
            let formatter = crate::format::formatter(sorted_values.as_ref())?;
            (0..sorted_values.len())
                .map(|i| {
                    crate::format::cell(sorted_values.as_ref(), &formatter, i).unwrap_or_default()
                })
                .collect()
        };

        let mut remap = [0u8; 256];
        for (sorted_rank, &orig_code) in sort_indices.values().iter().enumerate() {
            remap[orig_code as usize] = sorted_rank as u8;
        }

        if let Some(nulls) = &nulls {
            for (row, c) in codes.iter_mut().enumerate() {
                cancel.check_at(row)?;
                if !nulls.is_null(row) {
                    *c = remap[*c as usize];
                }
            }
        } else {
            for (row, c) in codes.iter_mut().enumerate() {
                cancel.check_at(row)?;
                *c = remap[*c as usize];
            }
        }

        let sorted_all_counts = (0..len)
            .map(|sorted_rank| all_counts[sort_indices.value(sorted_rank) as usize])
            .collect();

        Ok(Some(Self {
            codes,
            nulls,
            values: sorted_values,
            labels,
            all_counts: sorted_all_counts,
        }))
    }
}

impl Dictionary {
    #[cfg(test)]
    #[inline]
    pub(crate) fn labels(&self) -> &[String] {
        &self.labels
    }

    #[inline]
    pub(crate) fn null_count(&self) -> u64 {
        self.nulls.as_ref().map_or(0, |n| n.null_count() as u64)
    }

    #[cfg(test)]
    #[inline]
    pub(crate) fn is_null(&self, row: usize) -> bool {
        self.nulls.as_ref().is_some_and(|n| n.is_null(row))
    }

    #[cfg(test)]
    pub(crate) fn n_values(&self) -> usize {
        self.values.len()
    }

    #[cfg(test)]
    pub(crate) fn code_at(&self, row: usize) -> u8 {
        self.codes[row]
    }

    /// Project exact filter candidates directly from the dictionary's native values.
    pub(crate) fn filter_choices(
        &self,
        rows: &Selection,
        cancel: &Cancellation,
    ) -> Result<FilterChoices> {
        cancel.check()?;
        let tally = self.tally(rows, cancel)?;
        let values = self
            .labels
            .iter()
            .enumerate()
            .map(|(code, label)| {
                Ok(ValueChoice {
                    label: label.clone(),
                    value: value_text::encode(&self.values, code)?,
                    count: tally.counts[code],
                })
            })
            .collect::<Result<Vec<_>>>()?;
        cancel.check()?;
        Ok(FilterChoices {
            values: Some(values),
            null_count: tally.null_count,
        })
    }

    fn tally(&self, rows: &Selection, cancel: &Cancellation) -> Result<Tally> {
        Ok(match rows {
            Selection::All(_) => Tally {
                counts: self.all_counts.clone(),
                null_count: self.null_count(),
            },
            Selection::Rows(rows) => {
                let mut buf = [0u64; 256];
                let mut null_count = 0u64;
                if let Some(nulls) = &self.nulls {
                    for (index, &row) in rows.iter().enumerate() {
                        cancel.check_at(index)?;
                        let r = row as usize;
                        if nulls.is_null(r) {
                            null_count += 1;
                        } else {
                            buf[self.codes[r] as usize] += 1;
                        }
                    }
                } else {
                    for (index, &row) in rows.iter().enumerate() {
                        cancel.check_at(index)?;
                        buf[self.codes[row as usize] as usize] += 1;
                    }
                }
                Tally {
                    counts: buf[..self.values.len()].to_vec(),
                    null_count,
                }
            }
        })
    }

    fn fill_matches(
        &self,
        rows: RowSelection<'_>,
        incoming: Option<&[bool]>,
        output: &mut [bool],
        allowed: &[bool; 256],
        null_allowed: bool,
    ) {
        match (rows, incoming, &self.nulls) {
            (RowSelection::Range(range), None, None) => {
                for (i, &code) in self.codes[range].iter().enumerate() {
                    output[i] = allowed[code as usize];
                }
            }
            (RowSelection::Range(range), Some(mask), None) => {
                for (i, &code) in self.codes[range].iter().enumerate() {
                    output[i] = allowed[code as usize] & mask[i];
                }
            }
            (RowSelection::Range(range), None, Some(nulls)) => {
                for (i, (&code, r)) in self.codes[range.clone()].iter().zip(range).enumerate() {
                    output[i] = if nulls.is_null(r) {
                        null_allowed
                    } else {
                        allowed[code as usize]
                    };
                }
            }
            (RowSelection::Range(range), Some(mask), Some(nulls)) => {
                for (i, (&code, r)) in self.codes[range.clone()].iter().zip(range).enumerate() {
                    let hit = if nulls.is_null(r) {
                        null_allowed
                    } else {
                        allowed[code as usize]
                    };
                    output[i] = hit & mask[i];
                }
            }
            (RowSelection::Indices(indices), None, None) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = allowed[self.codes[row] as usize];
                }
            }
            (RowSelection::Indices(indices), Some(mask), None) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = allowed[self.codes[row] as usize] & mask[i];
                }
            }
            (RowSelection::Indices(indices), None, Some(nulls)) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = if nulls.is_null(row) {
                        null_allowed
                    } else {
                        allowed[self.codes[row] as usize]
                    };
                }
            }
            (RowSelection::Indices(indices), Some(mask), Some(nulls)) => {
                for (i, &row) in indices.iter().enumerate() {
                    let hit = if nulls.is_null(row) {
                        null_allowed
                    } else {
                        allowed[self.codes[row] as usize]
                    };
                    output[i] = hit & mask[i];
                }
            }
        }
    }

    fn predicate_from_allowed(
        self: &Arc<Self>,
        allowed: [bool; 256],
        null_allowed: bool,
    ) -> Predicate<'static> {
        let dictionary = Arc::clone(self);
        Predicate::new(Box::new(move |rows, incoming, output, _cancel| {
            dictionary.fill_matches(rows, incoming, output, &allowed, null_allowed);
            Ok(())
        }))
    }
}

/// Visit every key of a native Dictionary array; `None` for a null key.
/// Returns false if the key type is not an Arrow dictionary index type.
fn scan_keys(
    keys: &dyn Array,
    cancel: &Cancellation,
    mut f: impl FnMut(Option<usize>),
) -> Result<bool> {
    macro_rules! visit {
        ($array:expr) => {{
            for row in 0..$array.len() {
                cancel.check_at(row)?;
                f(if $array.is_null(row) {
                    None
                } else {
                    Some($array.value(row) as usize)
                });
            }
        }};
    }
    match keys.data_type() {
        DataType::Int8 => visit!(keys.as_primitive::<Int8Type>()),
        DataType::Int16 => visit!(keys.as_primitive::<Int16Type>()),
        DataType::Int32 => visit!(keys.as_primitive::<Int32Type>()),
        DataType::Int64 => visit!(keys.as_primitive::<Int64Type>()),
        DataType::UInt8 => visit!(keys.as_primitive::<UInt8Type>()),
        DataType::UInt16 => visit!(keys.as_primitive::<UInt16Type>()),
        DataType::UInt32 => visit!(keys.as_primitive::<UInt32Type>()),
        DataType::UInt64 => visit!(keys.as_primitive::<UInt64Type>()),
        _ => return Ok(false),
    }
    cancel.check()?;
    Ok(true)
}

#[cfg(test)]
mod tests;
