//! Turning a physical Arrow array into the dictionary: native dictionaries reuse
//! their keys and values, ordered primitives scan their native values, floats
//! scan a normalized key, and text, binary and nested arrays use Arrow's row
//! format.

use std::collections::HashMap;

use arrow::{
    array::{Array, ArrayRef, AsArray, UInt32Array},
    compute::take,
    datatypes::*,
    row::{RowConverter, SortField},
};

use super::{
    kernel::{build_distinct, finish_dictionary},
    primitive::{build_float_primitive, build_ordered_primitive},
    ValueDictionary, CHUNK, MAX_DISTINCT_VALUES, NULL_CODE,
};

impl ValueDictionary {
    /// Encode `array`, or `None` for unsupported types and columns with more
    /// than [`MAX_DISTINCT_VALUES`] non-null values.
    pub(crate) fn build(array: &ArrayRef) -> Option<Self> {
        macro_rules! ordered {
            ($ty:ident) => {
                build_ordered_primitive(array.as_primitive::<$ty>())
            };
        }
        macro_rules! float {
            ($ty:ident) => {
                build_float_primitive(array.as_primitive::<$ty>())
            };
        }

        use DataType::*;
        match array.data_type() {
            Dictionary(..) => Self::build_native(array),
            Boolean => build_boolean(array),
            Null => Some(Self::all_null(array)),
            Int8 => ordered!(Int8Type),
            Int16 => ordered!(Int16Type),
            Int32 => ordered!(Int32Type),
            Int64 => ordered!(Int64Type),
            UInt8 => ordered!(UInt8Type),
            UInt16 => ordered!(UInt16Type),
            UInt32 => ordered!(UInt32Type),
            UInt64 => ordered!(UInt64Type),
            Float16 => float!(Float16Type),
            Float32 => float!(Float32Type),
            Float64 => float!(Float64Type),
            Date32 => ordered!(Date32Type),
            Date64 => ordered!(Date64Type),
            Time32(TimeUnit::Second) => ordered!(Time32SecondType),
            Time32(TimeUnit::Millisecond) => ordered!(Time32MillisecondType),
            Time64(TimeUnit::Microsecond) => ordered!(Time64MicrosecondType),
            Time64(TimeUnit::Nanosecond) => ordered!(Time64NanosecondType),
            Timestamp(TimeUnit::Second, _) => ordered!(TimestampSecondType),
            Timestamp(TimeUnit::Millisecond, _) => ordered!(TimestampMillisecondType),
            Timestamp(TimeUnit::Microsecond, _) => ordered!(TimestampMicrosecondType),
            Timestamp(TimeUnit::Nanosecond, _) => ordered!(TimestampNanosecondType),
            Duration(TimeUnit::Second) => ordered!(DurationSecondType),
            Duration(TimeUnit::Millisecond) => ordered!(DurationMillisecondType),
            Duration(TimeUnit::Microsecond) => ordered!(DurationMicrosecondType),
            Duration(TimeUnit::Nanosecond) => ordered!(DurationNanosecondType),
            Decimal32(..) => ordered!(Decimal32Type),
            Decimal64(..) => ordered!(Decimal64Type),
            Decimal128(..) => ordered!(Decimal128Type),
            Decimal256(..) => ordered!(Decimal256Type),
            _ => Self::build_row_format(array),
        }
    }

    /// Row-format fallback for text, binary and nested arrays: discover
    /// distinct values by hashing rows, then write one `u8` code per row.
    /// Row-format bytes give both equality and value order.
    fn build_row_format(array: &ArrayRef) -> Option<Self> {
        let converter = RowConverter::new(vec![SortField::new(array.data_type().clone())]).ok()?;

        // Temporary codes in order of first appearance, and each value's representative
        // source row. Row-format bytes sort like the values themselves.
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

        // Renumber codes into value order; null keeps its reserved code.
        let mut key_of: Vec<&[u8]> = vec![&[]; representative_row.len()];
        for (bytes, &code) in &seen {
            key_of[code as usize] = bytes;
        }
        let mut order: Vec<u8> = (0..representative_row.len() as u8).collect();
        order.sort_unstable_by(|&a, &b| key_of[a as usize].cmp(key_of[b as usize]));
        finish_dictionary(codes, &representative_row, &order, has_null, array.as_ref())
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

/// Encode a boolean array. `false` sorts before `true`.
fn build_boolean(array: &ArrayRef) -> Option<ValueDictionary> {
    let source = array.as_boolean();
    build_distinct(
        source.len(),
        |row| source.is_null(row),
        |row| u8::from(source.value(row)),
        |_| false,
        source,
    )
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
