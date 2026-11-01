//! Turning a physical Arrow array into the codebook: native dictionaries reuse
//! their keys and values, and plain arrays use native primitive scans or
//! Arrow's row format.

use std::collections::HashMap;

use arrow::{
    array::{Array, ArrayRef, AsArray, UInt32Array},
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
    primitive::{build_boolean, build_null, build_primitive},
    ValueDictionary, CHUNK, MAX_DISTINCT_VALUES, NULL_CODE,
};

impl ValueDictionary {
    /// Encode `array`, or `None` for unsupported types and columns with more
    /// than [`MAX_DISTINCT_VALUES`] non-null values.
    pub(crate) fn build(array: &ArrayRef) -> Option<Self> {
        use DataType::*;
        match array.data_type() {
            Dictionary(..) => Self::build_native(array),
            Boolean => build_boolean(array),
            Null => build_null(array),
            Int8 => build_primitive(array.as_primitive::<Int8Type>()),
            Int16 => build_primitive(array.as_primitive::<Int16Type>()),
            Int32 => build_primitive(array.as_primitive::<Int32Type>()),
            Int64 => build_primitive(array.as_primitive::<Int64Type>()),
            UInt8 => build_primitive(array.as_primitive::<UInt8Type>()),
            UInt16 => build_primitive(array.as_primitive::<UInt16Type>()),
            UInt32 => build_primitive(array.as_primitive::<UInt32Type>()),
            UInt64 => build_primitive(array.as_primitive::<UInt64Type>()),
            Float16 => build_primitive(array.as_primitive::<Float16Type>()),
            Float32 => build_primitive(array.as_primitive::<Float32Type>()),
            Float64 => build_primitive(array.as_primitive::<Float64Type>()),
            Date32 => build_primitive(array.as_primitive::<Date32Type>()),
            Date64 => build_primitive(array.as_primitive::<Date64Type>()),
            Time32(TimeUnit::Second) => build_primitive(array.as_primitive::<Time32SecondType>()),
            Time32(TimeUnit::Millisecond) => {
                build_primitive(array.as_primitive::<Time32MillisecondType>())
            }
            Time64(TimeUnit::Microsecond) => {
                build_primitive(array.as_primitive::<Time64MicrosecondType>())
            }
            Time64(TimeUnit::Nanosecond) => {
                build_primitive(array.as_primitive::<Time64NanosecondType>())
            }
            Timestamp(TimeUnit::Second, _) => {
                build_primitive(array.as_primitive::<TimestampSecondType>())
            }
            Timestamp(TimeUnit::Millisecond, _) => {
                build_primitive(array.as_primitive::<TimestampMillisecondType>())
            }
            Timestamp(TimeUnit::Microsecond, _) => {
                build_primitive(array.as_primitive::<TimestampMicrosecondType>())
            }
            Timestamp(TimeUnit::Nanosecond, _) => {
                build_primitive(array.as_primitive::<TimestampNanosecondType>())
            }
            Duration(TimeUnit::Second) => {
                build_primitive(array.as_primitive::<DurationSecondType>())
            }
            Duration(TimeUnit::Millisecond) => {
                build_primitive(array.as_primitive::<DurationMillisecondType>())
            }
            Duration(TimeUnit::Microsecond) => {
                build_primitive(array.as_primitive::<DurationMicrosecondType>())
            }
            Duration(TimeUnit::Nanosecond) => {
                build_primitive(array.as_primitive::<DurationNanosecondType>())
            }
            Decimal32(..) => build_primitive(array.as_primitive::<Decimal32Type>()),
            Decimal64(..) => build_primitive(array.as_primitive::<Decimal64Type>()),
            Decimal128(..) => build_primitive(array.as_primitive::<Decimal128Type>()),
            Decimal256(..) => build_primitive(array.as_primitive::<Decimal256Type>()),
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
}

/// Build the `values` array in code order from the first row of each value.
pub(super) fn value_rows(array: &dyn Array, first_row: &[usize], order: &[u8]) -> Option<ArrayRef> {
    let rows = UInt32Array::from(
        order
            .iter()
            .map(|&old| first_row[old as usize] as u32)
            .collect::<Vec<_>>(),
    );
    take(array, &rows, None).ok()
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
