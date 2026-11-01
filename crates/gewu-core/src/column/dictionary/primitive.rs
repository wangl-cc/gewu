//! Native primitive encoders. One `PrimitiveKey` key function per physical
//! type, plus the floating-point value-identity rule those keys share.

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

/// Canonical bits of a floating-point value: `-0.0` groups with `0.0`, and
/// every NaN payload groups into one NaN.
pub(crate) fn normalized_bits(value: f64) -> u64 {
    if value == 0.0 {
        0
    } else if value.is_nan() {
        f64::NAN.to_bits()
    } else {
        value.to_bits()
    }
}

/// Order key of a normalized value. Unsigned comparison matches IEEE total
/// order, with the shared zero between negatives and positives and NaN last.
fn order_key(value: f64) -> u64 {
    let bits = normalized_bits(value);
    if bits & (1 << 63) != 0 {
        !bits
    } else {
        bits | (1 << 63)
    }
}

impl PrimitiveKey for Float16Type {
    type Key = u64;

    fn key(value: Self::Native) -> u64 {
        order_key(f64::from(value))
    }

    fn prefers_representative(value: Self::Native) -> bool {
        f64::from(value) == 0.0 && value.is_sign_positive()
    }
}

impl PrimitiveKey for Float32Type {
    type Key = u64;

    fn key(value: Self::Native) -> u64 {
        order_key(f64::from(value))
    }

    fn prefers_representative(value: Self::Native) -> bool {
        f64::from(value) == 0.0 && value.is_sign_positive()
    }
}

impl PrimitiveKey for Float64Type {
    type Key = u64;

    fn key(value: Self::Native) -> u64 {
        order_key(value)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_zeros_group_together() {
        assert_eq!(normalized_bits(0.0), normalized_bits(-0.0));
        assert_eq!(order_key(0.0), order_key(-0.0));
        assert_eq!(order_key(0.0), 1 << 63);
    }

    #[test]
    fn nan_payloads_group_together() {
        let payloads = [
            f64::NAN,
            f64::from_bits(f64::NAN.to_bits() | 1),
            f64::from_bits(0x7ff8_0000_0000_0001),
            f64::from_bits(0xfff8_0000_0000_0000),
        ];
        for value in payloads {
            assert_eq!(normalized_bits(value), f64::NAN.to_bits());
            assert_eq!(order_key(value), order_key(f64::NAN));
        }
    }

    #[test]
    fn order_key_orders_like_the_values() {
        let values = [
            f64::NEG_INFINITY,
            -1.0,
            -0.0,
            0.0,
            1.0,
            f64::INFINITY,
            f64::NAN,
        ];
        let keys: Vec<u64> = values.iter().copied().map(order_key).collect();
        // Signed zeros share one option, so the sequence is non-decreasing
        // rather than strictly increasing.
        assert!(keys.windows(2).all(|pair| pair[0] <= pair[1]), "{keys:?}");
        assert_eq!(keys[2], keys[3]);
        assert!(keys[0] < keys[1] && keys[4] < keys[5] && keys[5] < keys[6]);
    }
}
