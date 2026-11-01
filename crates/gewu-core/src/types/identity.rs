//! Value identity shared by every component that groups or orders values.
//!
//! Floating-point value selection cannot distinguish signed zeros or NaN
//! payloads meaningfully, so `-0.0`/`0.0` share one value and all NaNs share
//! another. The distinct-value dictionary, streaming counts and the values
//! analysis all use these functions instead of repeating the normalization.

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
pub(crate) fn order_key(value: f64) -> u64 {
    let bits = normalized_bits(value);
    if bits & (1 << 63) != 0 {
        !bits
    } else {
        bits | (1 << 63)
    }
}

/// Order key shared by `-0.0` and `0.0`; the representative value reads as
/// `0.0` when both signed zeros occur.
pub(crate) const ZERO_ORDER_KEY: u64 = 1 << 63;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_zeros_group_together() {
        assert_eq!(normalized_bits(0.0), normalized_bits(-0.0));
        assert_eq!(order_key(0.0), order_key(-0.0));
        assert_eq!(order_key(0.0), ZERO_ORDER_KEY);
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
