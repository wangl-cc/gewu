use std::{hash::Hash, sync::Arc};

use arrow::{
    array::{Array, ArrayRef, BooleanArray, PrimitiveArray},
    datatypes::*,
};

pub(super) trait Canonicalize: ArrowPrimitiveType {
    type Key: Copy + Eq + Ord + Hash;

    fn canonical(native: Self::Native) -> (Self::Key, Self::Native);
}

macro_rules! impl_canonicalize {
    ($($int_like:ident),*, $(,)?) => {
        $(
            impl Canonicalize for $int_like {
                type Key = Self::Native;

                #[inline]
                fn canonical(value: Self::Native) -> (Self::Key, Self::Native) {
                    (value, value)
                }
            }
        )*
    };
    ($($float_like:ident => $int:ident => ($zero:expr, $nan:expr, $shift:literal)),*, $(,)?) => {
        $(
            impl Canonicalize for $float_like {
                type Key = $int;

                #[inline]
                fn canonical(value: Self::Native) -> (Self::Key, Self::Native) {
                    let (bits, canonical_native) = if value == $zero {
                        (0, $zero)
                    } else if value.is_nan() {
                        ($nan.to_bits(), $nan)
                    } else {
                        (value.to_bits(), value)
                    };
                    let key = if bits & (1 << $shift) != 0 {
                        !bits
                    } else {
                        bits | (1 << $shift)
                    };
                    (key, canonical_native)
                }
            }
        )*
    };
}

impl_canonicalize!(
    Int8Type,
    Int16Type,
    Int32Type,
    Int64Type,
    UInt8Type,
    UInt16Type,
    UInt32Type,
    UInt64Type,
    Date32Type,
    Date64Type,
    Time32SecondType,
    Time32MillisecondType,
    Time64MicrosecondType,
    Time64NanosecondType,
    TimestampSecondType,
    TimestampMillisecondType,
    TimestampMicrosecondType,
    TimestampNanosecondType,
    DurationSecondType,
    DurationMillisecondType,
    DurationMicrosecondType,
    DurationNanosecondType,
    Decimal32Type,
    Decimal64Type,
    Decimal128Type,
    Decimal256Type,
);

impl_canonicalize!(
    Float16Type => u16 => (half::f16::ZERO, half::f16::NAN, 15),
    Float32Type => u32 => (0.0f32, f32::NAN, 31),
    Float64Type => u64 => (0.0f64, f64::NAN, 63),
);

pub(super) trait CanonicalizeArray: Array {
    type Key: Copy + Eq + Ord + Hash;
    type Native: Copy;

    fn canonical_at(&self, index: usize) -> (Self::Key, Self::Native);

    fn build_array(&self, sorted: impl ExactSizeIterator<Item = Self::Native>) -> ArrayRef;
}

impl<T: Canonicalize> CanonicalizeArray for PrimitiveArray<T> {
    type Key = T::Key;
    type Native = T::Native;

    #[inline]
    fn canonical_at(&self, index: usize) -> (Self::Key, Self::Native) {
        T::canonical(self.value(index))
    }

    fn build_array(&self, sorted: impl ExactSizeIterator<Item = Self::Native>) -> ArrayRef {
        Arc::new(
            PrimitiveArray::<T>::from_iter_values(sorted).with_data_type(self.data_type().clone()),
        )
    }
}

impl CanonicalizeArray for BooleanArray {
    type Key = bool;
    type Native = bool;

    #[inline]
    fn canonical_at(&self, index: usize) -> (Self::Key, Self::Native) {
        let v = self.value(index);
        (v, v)
    }

    fn build_array(&self, sorted: impl ExactSizeIterator<Item = bool>) -> ArrayRef {
        Arc::new(BooleanArray::from(sorted.collect::<Vec<bool>>()))
    }
}

pub(crate) fn normalized_bits(value: f64) -> u64 {
    if value == 0.0 {
        0
    } else if value.is_nan() {
        f64::NAN.to_bits()
    } else {
        value.to_bits()
    }
}
