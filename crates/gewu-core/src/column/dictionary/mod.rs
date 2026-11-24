//! Distinct values of a column as a compact `u8` code space.
//!
//! A `Dictionary` gives every source row a code naming its distinct
//! value: codes `0..values.len()` name the non-null values, while row-level
//! nulls are tracked via an Arrow [`NullBuffer`].
//!
//! # Architecture
//!
//! The dictionary is constructed via `Dictionary::build`:
//! - Native Arrow dictionary columns reuse their referenced dictionary entries directly.
//! - Canonical primitive and boolean columns scan native values in a single pass via
//!   `primitive::PrimitiveArray`.
//! - Text, binary and nested arrays use Arrow's row format to discover distinct byte rows.
//!
//! Filter candidates are computed by the View layer. `Dictionary::matching_codes`
//! supports filter generation without exposing the code space in the protocol.
//!
//! The dictionary is a pure function of the column data and is lazily computed
//! and cached by [`crate::column::Column`]. Columns with more than `MAX_DISTINCT_VALUES`
//! non-null values are not encoded.

use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::{
        downcast_dictionary_array, Array, ArrayRef, AsArray, DictionaryArray, NullBufferBuilder,
        UInt32Array,
    },
    buffer::{BooleanBuffer, NullBuffer},
    compute::take,
    datatypes::*,
    row::{RowConverter, SortField},
};
use rapidhash::RapidHashMap;

use crate::execution::predicate::{with_nulls, Selector};

type Result<T> = std::result::Result<T, crate::ColumnError>;

mod primitive;

/// Most non-null values a column may have and still be encoded. Non-null
/// values use codes `0..=254`. Columns with more values are not encoded.
pub(crate) const MAX_DISTINCT_VALUES: usize = 255;

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

    /// Select rows whose value code is matched (or, with `exclude`, unmatched);
    /// null rows are selected by `include_nulls`.
    pub(crate) fn matching_codes(
        self: &Arc<Self>,
        matched: &BooleanBuffer,
        exclude: bool,
        include_nulls: bool,
    ) -> Selector<'static> {
        let mut allowed = [false; 256];
        for (code, hit) in matched.iter().enumerate() {
            allowed[code] = hit != exclude;
        }
        let dictionary = Arc::clone(self);
        Selector::new(Box::new(move |rows| {
            let codes = &dictionary.codes[rows.clone()];
            let bits = BooleanBuffer::collect_bool(codes.len(), |i| allowed[codes[i] as usize]);
            with_nulls(bits, dictionary.nulls.as_ref(), &rows, include_nulls)
        }))
    }

    pub(crate) fn build(array: &ArrayRef) -> Result<Option<Self>> {
        macro_rules! from_canonical_array {
            ($ty:ident) => {{
                let array = array.as_primitive::<$ty>();
                Self::from_canonical_array(array)
            }};
        }

        use DataType::*;
        match array.data_type() {
            Dictionary(..) => downcast_dictionary_array!(
                array => Self::from_dict_array(array),
                _ => unreachable!("matched Arrow dictionary")
            ),
            Boolean => {
                let array = array.as_boolean();
                Self::from_canonical_array(array)
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
            _ => Self::from_row_array(array),
        }
    }

    /// Build a dictionary from a canonical primitive or boolean array in a single pass.
    /// Values array is assembled directly from canonical native values without sorting or `take`.
    fn from_canonical_array<A>(array: &A) -> Result<Option<Self>>
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
        Self::sort_and_remap(codes, nulls, values, all_counts)
    }

    /// Row-format fallback for text, binary and nested arrays: discover
    /// distinct values by hashing rows, then write one `u8` code per row.
    fn from_row_array(array: &ArrayRef) -> Result<Option<Self>> {
        /// Rows converted to row format at a time, to bound memory.
        const CHUNK: usize = 64 * 1024;

        let converter = match RowConverter::new(vec![SortField::new(array.data_type().clone())]) {
            Ok(converter) => converter,
            Err(arrow::error::ArrowError::NotYetImplemented(_)) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let nulls = array.nulls().cloned();
        let mut seen: HashMap<Box<[u8]>, u8> = HashMap::new();
        let mut representative_row: Vec<usize> = Vec::new();
        let mut codes: Vec<u8> = Vec::with_capacity(array.len());
        let mut buf = [0u64; 256];

        for start in (0..array.len()).step_by(CHUNK) {
            let len = CHUNK.min(array.len() - start);
            let rows = converter.convert_columns(std::slice::from_ref(&array.slice(start, len)))?;
            for (i, row) in rows.iter().enumerate() {
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
        Self::sort_and_remap(codes, nulls, values, all_counts)
    }

    /// Encode a native Dictionary. Arrow already encodes the values into
    /// `keys`/`values`, so no row hashing is needed: collect the referenced
    /// dictionary values, reuse [`Self::build`] on that small array, then
    /// walk the keys once to write the shared per-row code array.
    fn from_dict_array<K: ArrowDictionaryKeyType>(
        array: &DictionaryArray<K>,
    ) -> Result<Option<Self>> {
        let values = array.values();
        if values.len() > MAX_DISTINCT_VALUES + 1 {
            return Ok(None);
        }

        let mut used = vec![false; values.len()];
        for entry in array.keys_iter().flatten() {
            used[entry] = true;
        }
        let referenced = UInt32Array::from_iter_values(
            used.into_iter()
                .enumerate()
                .filter(|&(entry, used)| used && !crate::column::is_null_at(values.as_ref(), entry))
                .map(|(entry, _)| entry as u32),
        );
        let referenced_values = take(values.as_ref(), &referenced, None)?;
        let Some(encoded) = Self::build(&referenced_values)? else {
            return Ok(None);
        };

        let mut entry_codes = vec![None; values.len()];
        for (&entry, code) in referenced.values().iter().zip(encoded.codes) {
            entry_codes[entry as usize] = Some(code);
        }

        let mut counts = vec![0; encoded.values.len()];
        let mut nulls = NullBufferBuilder::new(array.len());
        let codes = array
            .keys_iter()
            .map(|key| {
                let code = key.and_then(|entry| entry_codes[entry]);
                nulls.append(code.is_some());
                if let Some(code) = code {
                    counts[code as usize] += 1;
                }
                // Null rows carry a placeholder code; only the validity bit is read.
                code.unwrap_or(0)
            })
            .collect();
        Ok(Some(Self {
            codes,
            nulls: nulls.finish(),
            values: encoded.values,
            labels: encoded.labels,
            all_counts: counts,
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
    ) -> Result<Option<Self>> {
        let len = values.len();
        if len <= 1 {
            let labels = if len == 0 {
                Vec::new()
            } else {
                let formatter = crate::format::formatter(values.as_ref())?;
                vec![formatter.value(0).to_string()]
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
                .map(|i| formatter.value(i).to_string())
                .collect()
        };

        let mut remap = [0u8; 256];
        for (sorted_rank, &orig_code) in sort_indices.values().iter().enumerate() {
            remap[orig_code as usize] = sorted_rank as u8;
        }

        if let Some(nulls) = &nulls {
            for (row, c) in codes.iter_mut().enumerate() {
                if !nulls.is_null(row) {
                    *c = remap[*c as usize];
                }
            }
        } else {
            for c in &mut codes {
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
    #[inline]
    pub(crate) fn labels(&self) -> &[String] {
        &self.labels
    }

    #[inline]
    pub(crate) fn null_count(&self) -> u64 {
        self.nulls.as_ref().map_or(0, |n| n.null_count() as u64)
    }

    #[inline]
    pub(crate) fn is_null(&self, row: usize) -> bool {
        self.nulls.as_ref().is_some_and(|n| n.is_null(row))
    }

    #[cfg(test)]
    pub(crate) fn n_values(&self) -> usize {
        self.values.len()
    }

    #[inline]
    pub(crate) fn code_at(&self, row: usize) -> u8 {
        self.codes[row]
    }

    /// Per-row value codes; meaningful only where [`Self::nulls`] is valid.
    pub(crate) fn codes(&self) -> &[u8] {
        &self.codes
    }

    pub(crate) fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    pub(crate) fn all_counts(&self) -> &[u64] {
        &self.all_counts
    }
}

#[cfg(test)]
mod tests;
