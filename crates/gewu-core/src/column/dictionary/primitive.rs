//! Native primitive encoders. One `PrimitiveKey` key function per physical
//! type; floats normalize signed zeros and NaN payloads.

use std::{collections::HashMap, hash::Hash};

use arrow::{
    array::Array,
    datatypes::{
        ArrowPrimitiveType, Date32Type, Date64Type, Decimal128Type, Decimal256Type, Decimal32Type,
        Decimal64Type, DurationMicrosecondType, DurationMillisecondType, DurationNanosecondType,
        DurationSecondType, Float16Type, Float32Type, Float64Type, Int16Type, Int32Type, Int64Type,
        Int8Type, Time32MillisecondType, Time32SecondType, Time64MicrosecondType,
        Time64NanosecondType, TimestampMicrosecondType, TimestampMillisecondType,
        TimestampNanosecondType, TimestampSecondType, UInt16Type, UInt32Type, UInt64Type,
        UInt8Type,
    },
};

use super::{value_rows, ValueDictionary, MAX_DISTINCT_VALUES, NULL_CODE};
use crate::column::float;

/// How a primitive value becomes a codebook key.
///
/// Contract: equal keys must mean "the same displayed value", and key order
/// must equal value order, because codes are assigned in key order.
pub(super) trait PrimitiveKey: ArrowPrimitiveType {
    type Key: Copy + Eq + Ord + Hash;

    fn key(value: Self::Native) -> Self::Key;

    /// Whether this value should replace the recorded representative of its
    /// key. Floats prefer positive zero so the option reads `0.0`.
    fn prefers_representative(_: Self::Native) -> bool {
        false
    }
}

macro_rules! identity_keys {
    ($($array:ty => $native:ty),* $(,)?) => {$(
        impl PrimitiveKey for $array {
            type Key = $native;

            fn key(value: $native) -> $native {
                value
            }
        }
    )*};
}

identity_keys!(
    Int8Type => i8,
    Int16Type => i16,
    Int32Type => i32,
    Int64Type => i64,
    UInt8Type => u8,
    UInt16Type => u16,
    UInt32Type => u32,
    UInt64Type => u64,
    Date32Type => i32,
    Date64Type => i64,
    Time32SecondType => i32,
    Time32MillisecondType => i32,
    Time64MicrosecondType => i64,
    Time64NanosecondType => i64,
    TimestampSecondType => i64,
    TimestampMillisecondType => i64,
    TimestampMicrosecondType => i64,
    TimestampNanosecondType => i64,
    DurationSecondType => i64,
    DurationMillisecondType => i64,
    DurationMicrosecondType => i64,
    DurationNanosecondType => i64,
    Decimal32Type => i32,
    Decimal64Type => i64,
    Decimal128Type => i128,
    Decimal256Type => arrow::datatypes::i256,
);

impl PrimitiveKey for Float16Type {
    type Key = u64;

    fn key(value: Self::Native) -> u64 {
        float::order_key(f64::from(value))
    }

    fn prefers_representative(value: Self::Native) -> bool {
        f64::from(value) == 0.0 && value.is_sign_positive()
    }
}

impl PrimitiveKey for Float32Type {
    type Key = u64;

    fn key(value: Self::Native) -> u64 {
        float::order_key(f64::from(value))
    }

    fn prefers_representative(value: Self::Native) -> bool {
        f64::from(value) == 0.0 && value.is_sign_positive()
    }
}

impl PrimitiveKey for Float64Type {
    type Key = u64;

    fn key(value: Self::Native) -> u64 {
        float::order_key(value)
    }

    fn prefers_representative(value: Self::Native) -> bool {
        value == 0.0 && value.is_sign_positive()
    }
}

/// Shared codebook algorithm: one native pass, one code per distinct key, then
/// codes renumbered into key order.
pub(super) fn build_distinct<K: Copy + Eq + Ord + Hash>(
    len: usize,
    is_null: impl Fn(usize) -> bool,
    key_at: impl Fn(usize) -> K,
    prefer_at: impl Fn(usize) -> bool,
    array: &dyn Array,
) -> Option<ValueDictionary> {
    let mut seen: HashMap<K, u8> = HashMap::new();
    let mut key_of: Vec<K> = Vec::new();
    let mut first_row: Vec<usize> = Vec::new();
    let mut codes: Vec<u8> = Vec::with_capacity(len);
    let mut has_null = false;
    for row in 0..len {
        if is_null(row) {
            has_null = true;
            codes.push(NULL_CODE);
            continue;
        }
        let key = key_at(row);
        let code = match seen.get(&key) {
            Some(&code) => code,
            None => {
                if key_of.len() == MAX_DISTINCT_VALUES {
                    return None;
                }
                let code = key_of.len() as u8;
                seen.insert(key, code);
                key_of.push(key);
                first_row.push(row);
                code
            }
        };
        if prefer_at(row) {
            first_row[code as usize] = row;
        }
        codes.push(code);
    }
    let mut order: Vec<u8> = (0..key_of.len() as u8).collect();
    order.sort_unstable_by(|&a, &b| key_of[a as usize].cmp(&key_of[b as usize]));
    let mut remap = [0u8; MAX_DISTINCT_VALUES];
    for (new, &old) in order.iter().enumerate() {
        remap[old as usize] = new as u8;
    }
    for code in codes.iter_mut().filter(|code| **code != NULL_CODE) {
        *code = remap[*code as usize];
    }
    Some(ValueDictionary {
        codes,
        values: value_rows(array, &first_row, &order)?,
        has_null,
    })
}
