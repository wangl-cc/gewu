//! Distinct values of a column as a compact `u8` code space.
//!
//! A [`Dictionary`] gives every source row a code naming its distinct
//! value: codes `0..=254` name the non-null values, and [`NULL_CODE`] names null.
//!
//! # Architecture
//!
//! The dictionary is constructed via [`Dictionary::from_array`]:
//! - Native Arrow dictionary columns reuse their referenced dictionary entries directly.
//! - Canonical primitive and boolean columns scan native values in a single pass via
//!   [`primitive::CanonicalizeArray`].
//! - Text, binary and nested arrays use Arrow's row format to discover distinct byte rows.
//!
//! Other modules consume the dictionary through [`Dictionary::tally`] for
//! frequency counting, [`Dictionary::values`] for distinct value arrays,
//! and [`Dictionary::prepare_predicate`] for filter generation.
//!
//! The dictionary is a pure function of the column data and is lazily computed
//! and cached by [`crate::Column`]. Columns with more than [`MAX_DISTINCT_VALUES`]
//! non-null values are not encoded.

use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::{Array, ArrayRef, AsArray, UInt32Array},
    compute::take,
    datatypes::*,
    row::{RowConverter, SortField},
};

use crate::{predicate::Predicate, scope::Selection, selection::RowSelection};

mod primitive;

pub(crate) use primitive::normalized_bits;

/// Most non-null values a column may have and still be encoded. Non-null
/// values use codes `0..=254`; [`NULL_CODE`] is reserved for null, so one
/// `u8` code space covers 255 values plus null. Columns with more values are
/// not encoded.
pub(crate) const MAX_DISTINCT_VALUES: usize = 255;

/// Code reserved for null. Non-null values use codes `0..=254`.
pub(crate) const NULL_CODE: u8 = 255;

/// Rows converted to row format at a time, to bound memory.
const CHUNK: usize = 64 * 1024;

pub(crate) struct Dictionary {
    /// Code of each source row; [`NULL_CODE`] for null.
    codes: Vec<u8>,
    /// Non-null values in code order.
    values: ArrayRef,
    /// Whether the source column contains at least one null.
    has_null: bool,
}

impl Dictionary {
    /// Encode `array`, or `None` for unsupported types and columns with more
    /// than [`MAX_DISTINCT_VALUES`] non-null values.
    pub(crate) fn from_array(array: &ArrayRef) -> Option<Self> {
        macro_rules! from_canonical_array {
            ($ty:ident) => {{
                let array = array.as_primitive::<$ty>();
                Self::from_canonical_array(array)
            }};
        }

        use DataType::*;
        match array.data_type() {
            Dictionary(..) => Self::from_dict_array(array),
            Boolean => {
                let array = array.as_boolean();
                Self::from_canonical_array(array)
            }
            Null => Some(Self::all_null(array)),
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
    fn from_canonical_array<A>(array: &A) -> Option<Self>
    where
        A: primitive::CanonicalizeArray,
    {
        let len = array.len();
        let nulls = array.nulls();
        let mut seen: HashMap<A::Key, u8> = HashMap::new();
        let mut native_of: Vec<A::Native> = Vec::new();
        let mut codes: Vec<u8> = Vec::with_capacity(len);
        let mut has_null = false;

        for row in 0..len {
            if nulls.as_ref().is_some_and(|n| n.is_null(row)) {
                has_null = true;
                codes.push(NULL_CODE);
                continue;
            }
            let (key, native) = array.canonical_at(row);
            let code = match seen.get(&key) {
                Some(&code) => code,
                None => {
                    if native_of.len() == MAX_DISTINCT_VALUES {
                        return None;
                    }
                    let code = native_of.len() as u8;
                    seen.insert(key, code);
                    native_of.push(native);
                    code
                }
            };
            codes.push(code);
        }

        let values = array.build_array(native_of.into_iter());
        Some(Self {
            codes,
            values,
            has_null,
        })
    }

    /// Row-format fallback for text, binary and nested arrays: discover
    /// distinct values by hashing rows, then write one `u8` code per row.
    fn from_row_array(array: &ArrayRef) -> Option<Self> {
        let converter = RowConverter::new(vec![SortField::new(array.data_type().clone())]).ok()?;

        let mut seen: HashMap<Box<[u8]>, u8> = HashMap::new();
        let mut representative_row: Vec<usize> = Vec::new();
        let mut codes: Vec<u8> = Vec::with_capacity(array.len());
        let mut has_null = false;
        for start in (0..array.len()).step_by(CHUNK) {
            let len = CHUNK.min(array.len() - start);
            let rows = converter
                .convert_columns(std::slice::from_ref(&array.slice(start, len)))
                .ok()?;
            for (i, row) in rows.iter().enumerate() {
                let source_row: usize = start + i;
                if array.is_null(source_row) {
                    has_null = true;
                    codes.push(NULL_CODE);
                    continue;
                }
                let bytes: &[u8] = row.as_ref();
                let code = match seen.get(bytes) {
                    Some(&code) => code,
                    None => {
                        if representative_row.len() == MAX_DISTINCT_VALUES {
                            return None;
                        }
                        let code = representative_row.len() as u8;
                        seen.insert(bytes.into(), code);
                        representative_row.push(source_row);
                        code
                    }
                };
                codes.push(code);
            }
        }

        let rows =
            UInt32Array::from_iter_values(representative_row.into_iter().map(|row| row as u32));
        let values = take(array.as_ref(), &rows, None).ok()?;
        Some(Self {
            codes,
            values,
            has_null,
        })
    }

    /// Encode a native Dictionary. Arrow already encodes the values into
    /// `keys`/`values`, so no row hashing is needed: collect the referenced
    /// dictionary values, reuse [`Self::from_array`] on that small array, then
    /// walk the keys once to write the shared per-row code array.
    fn from_dict_array(array: &ArrayRef) -> Option<Self> {
        let dictionary = array.as_any_dictionary();
        let keys = dictionary.keys();
        let dictionary_values = dictionary.values().clone();
        // A dictionary with more entries than the code space cannot be compact.
        if dictionary_values.len() > MAX_DISTINCT_VALUES + 1 {
            return None;
        }

        // Which dictionary entries are referenced, and are there logical nulls
        // (a null key, or a key pointing at a null value)?
        let value_nulls = dictionary_values.logical_nulls();
        let mut has_null = false;
        let mut referenced: Vec<u32> = Vec::new();
        let mut entry_slot = vec![u32::MAX; dictionary_values.len()];
        if !scan_keys(keys, |key| {
            let Some(entry) = key else {
                has_null = true;
                return;
            };
            if value_nulls
                .as_ref()
                .is_some_and(|nulls| nulls.is_null(entry))
            {
                has_null = true;
                return;
            }
            let Some(slot) = entry_slot.get_mut(entry) else {
                return;
            };
            if *slot == u32::MAX {
                *slot = referenced.len() as u32;
                referenced.push(entry as u32);
            }
        }) {
            return None;
        }

        // Encode the referenced values with the same path as a plain column.
        let referenced_values = take(
            dictionary_values.as_ref(),
            &UInt32Array::from(referenced),
            None,
        )
        .ok()?;
        let encoded = Self::from_array(&referenced_values)?;

        let mut key_to_code = vec![NULL_CODE; dictionary_values.len()];
        for (entry, &slot) in entry_slot.iter().enumerate() {
            if slot != u32::MAX {
                key_to_code[entry] = encoded.codes[slot as usize];
            }
        }

        // One key walk writes the shared per-row code array.
        let mut codes = Vec::with_capacity(array.len());
        if !scan_keys(keys, |key| {
            codes.push(match key {
                None => NULL_CODE,
                Some(entry) => key_to_code.get(entry).copied().unwrap_or(NULL_CODE),
            });
        }) {
            return None;
        }

        Some(Self {
            codes,
            values: encoded.values,
            has_null,
        })
    }

    /// A column with no values: every row is null and there is nothing to
    /// enumerate or label.
    fn all_null(array: &ArrayRef) -> Self {
        Self {
            codes: vec![NULL_CODE; array.len()],
            values: array.slice(0, 0),
            has_null: !array.is_empty(),
        }
    }
}

impl Dictionary {
    #[inline]
    pub(crate) fn values(&self) -> &ArrayRef {
        &self.values
    }

    #[inline]
    pub(crate) fn has_null(&self) -> bool {
        self.has_null
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    #[cfg(test)]
    pub(crate) fn n_values(&self) -> usize {
        self.values.len()
    }

    #[cfg(test)]
    pub(crate) fn code_at(&self, row: usize) -> u8 {
        self.codes[row]
    }

    /// Tally occurrences of each code over `rows`.
    /// `counts[0..self.values.len()]` correspond to non-null values, and
    /// `counts[NULL_CODE as usize]` is the count of null rows.
    pub(crate) fn tally(&self, rows: &Selection) -> [u64; 256] {
        let mut counts = [0u64; 256];
        match rows {
            Selection::All(_) => self
                .codes
                .iter()
                .for_each(|&code| counts[code as usize] += 1),
            Selection::Rows(rows) => rows
                .iter()
                .for_each(|&row| counts[self.codes[row as usize] as usize] += 1),
        }
        counts
    }

    fn fill_matches(
        &self,
        rows: RowSelection<'_>,
        incoming: Option<&[bool]>,
        output: &mut [bool],
        allowed: &[bool; 256],
    ) {
        match (rows, incoming) {
            (RowSelection::Range(range), None) => {
                for (i, &code) in self.codes[range].iter().enumerate() {
                    output[i] = allowed[code as usize];
                }
            }
            (RowSelection::Range(range), Some(mask)) => {
                for (i, &code) in self.codes[range].iter().enumerate() {
                    output[i] = allowed[code as usize] & mask[i];
                }
            }
            (RowSelection::Indices(indices), None) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = allowed[self.codes[row] as usize];
                }
            }
            (RowSelection::Indices(indices), Some(mask)) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = allowed[self.codes[row] as usize] & mask[i];
                }
            }
        }
    }

    pub(crate) fn predicate_from_allowed(
        self: &Arc<Self>,
        allowed: [bool; 256],
    ) -> Predicate<'static> {
        let dictionary = Arc::clone(self);
        Predicate::new(Box::new(move |rows, incoming, output, _cancel| {
            dictionary.fill_matches(rows, incoming, output, &allowed);
            Ok(())
        }))
    }

    /// Prepare a row-based predicate for filtering by key codes.
    pub(crate) fn prepare_predicate(self: &Arc<Self>, keys: &[u64]) -> Predicate<'static> {
        let mut allowed = [false; 256];
        for &key in keys {
            if let Ok(key) = usize::try_from(key) {
                if let Some(slot) = allowed.get_mut(key) {
                    *slot = true;
                }
            }
        }
        self.predicate_from_allowed(allowed)
    }
}

/// Visit every key of a native Dictionary array; `None` for a null key.
/// Returns false if the key type is not an Arrow dictionary index type.
fn scan_keys(keys: &dyn Array, mut f: impl FnMut(Option<usize>)) -> bool {
    macro_rules! visit {
        ($array:expr) => {{
            for row in 0..$array.len() {
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
        _ => return false,
    }
    true
}

#[cfg(test)]
mod primitive_tests;
#[cfg(test)]
mod tests;
