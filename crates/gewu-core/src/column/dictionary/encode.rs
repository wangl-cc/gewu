//! Turning a physical Arrow array into the dictionary: native dictionaries reuse
//! their keys and values, and plain arrays use native primitive scans or
//! Arrow's row format.

use std::collections::HashMap;

use arrow::{
    array::{Array, ArrayRef, AsArray, BooleanArray, PrimitiveArray, UInt32Array},
    compute::take,
    datatypes::{
        DataType, Date32Type, Date64Type, Decimal128Type, Decimal256Type, Decimal32Type,
        Decimal64Type, DurationMicrosecondType, DurationMillisecondType, DurationNanosecondType,
        DurationSecondType, Float16Type, Float32Type, Float64Type, Int16Type, Int32Type, Int64Type,
        Int8Type, Time32MillisecondType, Time32SecondType, Time64MicrosecondType,
        Time64NanosecondType, TimeUnit, TimestampMicrosecondType, TimestampMillisecondType,
        TimestampNanosecondType, TimestampSecondType, UInt16Type, UInt32Type, UInt64Type,
        UInt8Type,
    },
    row::{RowConverter, SortField},
};

use super::{
    primitive::{build_distinct, DictionaryKey},
    value_rows, ValueDictionary, CHUNK, MAX_DISTINCT_VALUES, NULL_CODE,
};

/// An array kind that can enumerate its own values into a dictionary.
pub(super) trait BuildDictionary {
    fn build_dictionary(&self) -> Option<ValueDictionary>;
}

impl<T: DictionaryKey> BuildDictionary for PrimitiveArray<T> {
    fn build_dictionary(&self) -> Option<ValueDictionary> {
        build_distinct(
            self.len(),
            |row| self.is_null(row),
            |row| T::key(self.value(row)),
            |row| T::prefers_representative(self.value(row)),
            self,
        )
    }
}

impl BuildDictionary for BooleanArray {
    fn build_dictionary(&self) -> Option<ValueDictionary> {
        build_distinct(
            self.len(),
            |row| self.is_null(row),
            |row| u8::from(self.value(row)),
            |_| false,
            self,
        )
    }
}

impl ValueDictionary {
    /// Encode `array`, or `None` for unsupported types and columns with more
    /// than [`MAX_DISTINCT_VALUES`] non-null values.
    pub(crate) fn build(array: &ArrayRef) -> Option<Self> {
        use DataType::*;
        match array.data_type() {
            Dictionary(..) => Self::build_native(array),
            Boolean => array.as_boolean().build_dictionary(),
            Null => Some(ValueDictionary::all_null(array)),
            Int8 => array.as_primitive::<Int8Type>().build_dictionary(),
            Int16 => array.as_primitive::<Int16Type>().build_dictionary(),
            Int32 => array.as_primitive::<Int32Type>().build_dictionary(),
            Int64 => array.as_primitive::<Int64Type>().build_dictionary(),
            UInt8 => array.as_primitive::<UInt8Type>().build_dictionary(),
            UInt16 => array.as_primitive::<UInt16Type>().build_dictionary(),
            UInt32 => array.as_primitive::<UInt32Type>().build_dictionary(),
            UInt64 => array.as_primitive::<UInt64Type>().build_dictionary(),
            Float16 => array.as_primitive::<Float16Type>().build_dictionary(),
            Float32 => array.as_primitive::<Float32Type>().build_dictionary(),
            Float64 => array.as_primitive::<Float64Type>().build_dictionary(),
            Date32 => array.as_primitive::<Date32Type>().build_dictionary(),
            Date64 => array.as_primitive::<Date64Type>().build_dictionary(),
            Time32(TimeUnit::Second) => array.as_primitive::<Time32SecondType>().build_dictionary(),
            Time32(TimeUnit::Millisecond) => array
                .as_primitive::<Time32MillisecondType>()
                .build_dictionary(),
            Time64(TimeUnit::Microsecond) => array
                .as_primitive::<Time64MicrosecondType>()
                .build_dictionary(),
            Time64(TimeUnit::Nanosecond) => array
                .as_primitive::<Time64NanosecondType>()
                .build_dictionary(),
            Timestamp(TimeUnit::Second, _) => array
                .as_primitive::<TimestampSecondType>()
                .build_dictionary(),
            Timestamp(TimeUnit::Millisecond, _) => array
                .as_primitive::<TimestampMillisecondType>()
                .build_dictionary(),
            Timestamp(TimeUnit::Microsecond, _) => array
                .as_primitive::<TimestampMicrosecondType>()
                .build_dictionary(),
            Timestamp(TimeUnit::Nanosecond, _) => array
                .as_primitive::<TimestampNanosecondType>()
                .build_dictionary(),
            Duration(TimeUnit::Second) => array
                .as_primitive::<DurationSecondType>()
                .build_dictionary(),
            Duration(TimeUnit::Millisecond) => array
                .as_primitive::<DurationMillisecondType>()
                .build_dictionary(),
            Duration(TimeUnit::Microsecond) => array
                .as_primitive::<DurationMicrosecondType>()
                .build_dictionary(),
            Duration(TimeUnit::Nanosecond) => array
                .as_primitive::<DurationNanosecondType>()
                .build_dictionary(),
            Decimal32(..) => array.as_primitive::<Decimal32Type>().build_dictionary(),
            Decimal64(..) => array.as_primitive::<Decimal64Type>().build_dictionary(),
            Decimal128(..) => array.as_primitive::<Decimal128Type>().build_dictionary(),
            Decimal256(..) => array.as_primitive::<Decimal256Type>().build_dictionary(),
            _ => Self::build_row_format(array),
        }
    }

    /// Row-format fallback for text, binary and nested arrays: discover
    /// distinct values by hashing rows, then write one `u8` code per row.
    /// Row-format bytes give both equality and value order.
    fn build_row_format(array: &ArrayRef) -> Option<Self> {
        let converter = RowConverter::new(vec![SortField::new(array.data_type().clone())]).ok()?;

        // Temporary codes in order of first appearance, and each value's first
        // source row. Row-format bytes sort like the values themselves.
        let mut seen: HashMap<Box<[u8]>, u8> = HashMap::new();
        let mut first_row: Vec<usize> = Vec::new();
        let mut codes: Vec<u8> = Vec::with_capacity(array.len());
        let mut has_null = false;
        for start in (0..array.len()).step_by(CHUNK) {
            let len = CHUNK.min(array.len() - start);
            let rows = converter
                .convert_columns(std::slice::from_ref(&array.slice(start, len)))
                .ok()?;
            for (i, row) in rows.iter().enumerate() {
                let source_row = start + i;
                if array.is_null(source_row) {
                    has_null = true;
                    codes.push(NULL_CODE);
                    continue;
                }
                let bytes: &[u8] = row.as_ref();
                let code = match seen.get(bytes) {
                    Some(&code) => code,
                    None => {
                        if first_row.len() == MAX_DISTINCT_VALUES {
                            return None;
                        }
                        let code = first_row.len() as u8;
                        seen.insert(bytes.into(), code);
                        first_row.push(source_row);
                        code
                    }
                };
                codes.push(code);
            }
        }

        // Renumber codes into value order; null keeps its reserved code.
        let mut key_of: Vec<&[u8]> = vec![&[]; first_row.len()];
        for (bytes, &code) in &seen {
            key_of[code as usize] = bytes;
        }
        let mut order: Vec<u8> = (0..first_row.len() as u8).collect();
        order.sort_unstable_by(|&a, &b| key_of[a as usize].cmp(key_of[b as usize]));
        let mut remap = [0u8; MAX_DISTINCT_VALUES];
        for (new, &old) in order.iter().enumerate() {
            remap[old as usize] = new as u8;
        }
        for code in codes.iter_mut().filter(|code| **code != NULL_CODE) {
            *code = remap[*code as usize];
        }

        Some(Self {
            codes,
            values: value_rows(array.as_ref(), &first_row, &order)?,
            has_null,
        })
    }

    /// Encode a native Dictionary. Arrow already encodes the values into
    /// `keys`/`values`, so no row hashing is needed: collect the referenced
    /// dictionary values, reuse [`Self::build`] on that small array, then
    /// walk the keys once to write the shared per-row code array.
    fn build_native(array: &ArrayRef) -> Option<Self> {
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
        let encoded = Self::build(&referenced_values)?;

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
