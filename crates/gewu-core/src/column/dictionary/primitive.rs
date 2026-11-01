//! Native primitive encoders. A primitive whose native values are already
//! exactly comparable and totally ordered uses its native value as the
//! dictionary key; a floating-point array projects to a normalized total-order
//! key so signed zeros and NaN payloads group the way value selection expects.

use std::{collections::HashMap, hash::Hash};

use arrow::{
    array::{Array, PrimitiveArray},
    datatypes::ArrowPrimitiveType,
};

use super::{value_rows, ValueDictionary, MAX_DISTINCT_VALUES, NULL_CODE};

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

/// Encode an array whose native values are already the dictionary keys.
pub(super) fn build_ordered_primitive<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
) -> Option<ValueDictionary>
where
    T::Native: Copy + Eq + Ord + Hash,
{
    build_distinct(
        array.len(),
        |row| array.is_null(row),
        |row| array.value(row),
        |_| false,
        array,
    )
}

/// Encode a floating-point array. The key normalizes signed zeros and NaN
/// payloads into one total order, and the zero group keeps positive zero as its
/// representative so the option reads `0.0`.
pub(super) fn build_float_primitive<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
) -> Option<ValueDictionary>
where
    T::Native: Into<f64>,
{
    build_distinct(
        array.len(),
        |row| array.is_null(row),
        |row| order_key(array.value(row).into()),
        |row| {
            let value: f64 = array.value(row).into();
            value == 0.0 && value.is_sign_positive()
        },
        array,
    )
}

/// Shared dictionary algorithm: one native pass, one code per distinct key, then
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
    let mut representative_row: Vec<usize> = Vec::new();
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
                representative_row.push(row);
                code
            }
        };
        if prefer_at(row) {
            representative_row[code as usize] = row;
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
        values: value_rows(array, &representative_row, &order)?,
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
