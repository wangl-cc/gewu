//! Statistics borrow the original Arrow arrays. Type dispatch happens
//! once per operation; only individual values needed for floating-point results
//! are converted. No widened column or numeric cache is built.

use std::hash::Hash;

use arrow::{
    array::{downcast_integer_array, Array, ArrayRef, AsArray, PrimitiveArray},
    compute::single_decimal_to_float_lossy,
    datatypes::*,
};
use num_traits::ToPrimitive;

use crate::{ColumnSummary, DataKind};

/// A validated, borrowed numeric column.
#[derive(Clone, Copy)]
pub(crate) struct Numeric<'a>(&'a ArrayRef);

// Keep the supported physical types in one dispatch table. Each branch calls
// a monomorphized operation over the native array, not a per-row type switch.
macro_rules! dispatch {
    ($array:expr, $method:ident($($arg:expr),* $(,)?)) => {{
        let a = $array;
        use DataType::*;
        match a.data_type() {
            Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64 => {
                downcast_integer_array!(a => IntegerColumn(a).$method($($arg),*),
                    _ => unreachable!("integer type checked above"))
            }
            Date32 => IntegerColumn(a.as_primitive::<Date32Type>()).$method($($arg),*),
            Date64 => IntegerColumn(a.as_primitive::<Date64Type>()).$method($($arg),*),
            Time32(TimeUnit::Second) => IntegerColumn(a.as_primitive::<Time32SecondType>()).$method($($arg),*),
            Time32(TimeUnit::Millisecond) => IntegerColumn(a.as_primitive::<Time32MillisecondType>()).$method($($arg),*),
            Time64(TimeUnit::Microsecond) => IntegerColumn(a.as_primitive::<Time64MicrosecondType>()).$method($($arg),*),
            Time64(TimeUnit::Nanosecond) => IntegerColumn(a.as_primitive::<Time64NanosecondType>()).$method($($arg),*),
            Timestamp(TimeUnit::Second, _) => IntegerColumn(a.as_primitive::<TimestampSecondType>()).$method($($arg),*),
            Duration(TimeUnit::Second) => IntegerColumn(a.as_primitive::<DurationSecondType>()).$method($($arg),*),
            Timestamp(TimeUnit::Millisecond, _) => IntegerColumn(a.as_primitive::<TimestampMillisecondType>()).$method($($arg),*),
            Duration(TimeUnit::Millisecond) => IntegerColumn(a.as_primitive::<DurationMillisecondType>()).$method($($arg),*),
            Timestamp(TimeUnit::Microsecond, _) => IntegerColumn(a.as_primitive::<TimestampMicrosecondType>()).$method($($arg),*),
            Duration(TimeUnit::Microsecond) => IntegerColumn(a.as_primitive::<DurationMicrosecondType>()).$method($($arg),*),
            Timestamp(TimeUnit::Nanosecond, _) => IntegerColumn(a.as_primitive::<TimestampNanosecondType>()).$method($($arg),*),
            Duration(TimeUnit::Nanosecond) => IntegerColumn(a.as_primitive::<DurationNanosecondType>()).$method($($arg),*),
            Float16 => RealColumn::new(a.as_primitive::<Float16Type>()).$method($($arg),*),
            Float32 => RealColumn::new(a.as_primitive::<Float32Type>()).$method($($arg),*),
            Float64 => RealColumn::new(a.as_primitive::<Float64Type>()).$method($($arg),*),
            Decimal32(..) => RealColumn::new(a.as_primitive::<Decimal32Type>()).$method($($arg),*),
            Decimal64(..) => RealColumn::new(a.as_primitive::<Decimal64Type>()).$method($($arg),*),
            Decimal128(..) => RealColumn::new(a.as_primitive::<Decimal128Type>()).$method($($arg),*),
            Decimal256(..) => RealColumn::new(a.as_primitive::<Decimal256Type>()).$method($($arg),*),
            _ => unreachable!("Numeric validates supported types"),
        }
    }};
}

mod summary;

impl<'a> Numeric<'a> {
    pub(crate) fn new(array: &'a ArrayRef) -> Option<Self> {
        // `DataKind` decides; the dispatch table above must cover every
        // ordered kind (checked by a test in kind.rs).
        DataKind::of(array.data_type())
            .is_ordered()
            .then_some(Self(array))
    }

    /// Fill in the numeric parts of `summary`; returns the rows holding
    /// the smallest and largest values.
    pub(crate) fn summarize(
        self,
        rows: impl Iterator<Item = usize>,
        summary: &mut ColumnSummary,
    ) -> (Option<usize>, Option<usize>) {
        dispatch!(self.0, summarize(rows, summary))
    }
}

struct IntegerColumn<'a, T: ArrowPrimitiveType>(&'a PrimitiveArray<T>);

/// Operations that differ between native floating values and decimal coefficients.
trait RealType: ArrowPrimitiveType {
    type Key: Eq + Hash;
    fn as_float(value: Self::Native, scale: i32) -> f64;
    fn key(value: Self::Native) -> Self::Key;
}

macro_rules! float_type {
    ($($ty:ty),* $(,)?) => {$(
        impl RealType for $ty {
            type Key = u64;
            fn as_float(value: Self::Native, _: i32) -> f64 { f64::from(value) }
            fn key(value: Self::Native) -> u64 { (f64::from(value) + 0.0).to_bits() }
        }
    )*};
}
float_type!(Float16Type, Float32Type, Float64Type);

macro_rules! decimal_type {
    ($($ty:ty),* $(,)?) => {$(
        impl RealType for $ty {
            type Key = Self::Native;
            fn as_float(value: Self::Native, scale: i32) -> f64 {
                single_decimal_to_float_lossy::<Self, _>(
                    &|v| v.to_f64().expect("decimal coefficients fit in f64's exponent range"),
                    value, scale,
                )
            }
            fn key(value: Self::Native) -> Self::Native { value }
        }
    )*};
}
decimal_type!(Decimal32Type, Decimal64Type, Decimal128Type, Decimal256Type);

/// Borrowed native values plus the decimal scale, with no owned array buffers.
struct RealColumn<'a, T: RealType> {
    array: &'a PrimitiveArray<T>,
    scale: i32,
}

impl<'a, T: RealType> RealColumn<'a, T> {
    fn new(array: &'a PrimitiveArray<T>) -> Self {
        let scale = match array.data_type() {
            DataType::Decimal32(_, scale)
            | DataType::Decimal64(_, scale)
            | DataType::Decimal128(_, scale)
            | DataType::Decimal256(_, scale) => i32::from(*scale),
            _ => 0,
        };
        Self { array, scale }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{Decimal128Array, Decimal256Array},
        compute::cast,
    };

    use super::*;

    #[test]
    fn decimal_scalar_conversion_matches_arrows_scale_and_rounding() {
        for scale in [-38, -2, 0, 2, 38] {
            let array: ArrayRef = Arc::new(
                Decimal128Array::from(vec![
                    Some(1),
                    Some(-1),
                    Some(9_007_199_254_740_993),
                    Some(10_i128.pow(38) - 1),
                    None,
                ])
                .with_precision_and_scale(38, scale)
                .unwrap(),
            );
            check_decimal_conversion(array);
        }
        for scale in [-76, -2, 0, 2, 76] {
            let large: i256 = "123456789012345678901234567890123456789012345678901234567890"
                .parse()
                .unwrap();
            let array: ArrayRef = Arc::new(
                Decimal256Array::from(vec![
                    Some(i256::from(1)),
                    Some(i256::from(-1)),
                    Some(large),
                    Some(-large),
                    None,
                ])
                .with_precision_and_scale(76, scale)
                .unwrap(),
            );
            check_decimal_conversion(array);
        }
    }

    fn check_decimal_conversion(array: ArrayRef) {
        let converted = cast(&array, &DataType::Float64).unwrap();
        let expected = converted.as_primitive::<Float64Type>();
        for row in 0..array.len() {
            if array.is_null(row) {
                continue;
            }
            let actual = match array.data_type() {
                DataType::Decimal128(_, scale) => Decimal128Type::as_float(
                    array.as_primitive::<Decimal128Type>().value(row),
                    i32::from(*scale),
                ),
                DataType::Decimal256(_, scale) => Decimal256Type::as_float(
                    array.as_primitive::<Decimal256Type>().value(row),
                    i32::from(*scale),
                ),
                _ => unreachable!("test decimal types"),
            };
            assert_eq!(
                actual.to_bits(),
                expected.value(row).to_bits(),
                "{:?} row {row}",
                array.data_type()
            );
        }
    }
}
