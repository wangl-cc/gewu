//! Exact decimal predicates over native arrays.

use arrow::{
    array::{Array, ArrayRef, AsArray, PrimitiveArray},
    datatypes::*,
};

pub(in crate::column) use crate::scalar::decimal::{Endpoint, Literal};
use crate::{predicate::Predicate, selection::RowSelection};

pub(in crate::column) trait Coefficient:
    Copy + PartialOrd + Send + Sync + 'static
{
    const MIN: i256;
    const MAX: i256;
    fn from_wide(value: i256) -> Self;
}

macro_rules! coefficient {
    ($($t:ty),*) => {$ (
        impl Coefficient for $t {
            const MIN: i256 = i256::from_i128(<$t>::MIN as i128);
            const MAX: i256 = i256::from_i128(<$t>::MAX as i128);
            fn from_wide(value: i256) -> Self { value.to_i128().expect("clamped to native range") as Self }
        }
    )*};
}
coefficient!(i8, i16, i32, i64, i128, u8, u16, u32, u64);
impl Coefficient for i256 {
    const MAX: i256 = i256::MAX;
    const MIN: i256 = i256::MIN;

    fn from_wide(value: i256) -> Self {
        value
    }
}

pub(in crate::column) fn native_range<'a, T: ArrowPrimitiveType>(
    array: &'a PrimitiveArray<T>,
    lo: Endpoint,
    hi: Endpoint,
) -> Predicate<'a>
where
    T::Native: Coefficient,
{
    let lo = lo.max(Endpoint::Finite(<T::Native as Coefficient>::MIN));
    let hi = hi.min(Endpoint::Finite(<T::Native as Coefficient>::MAX));
    if lo > hi {
        return Predicate::empty();
    }
    let (Endpoint::Finite(lo), Endpoint::Finite(hi)) = (lo, hi) else {
        unreachable!("native domain intersection");
    };
    let (lo, hi) = (T::Native::from_wide(lo), T::Native::from_wide(hi));
    let values = array.values();
    let has_nulls = array.null_count() > 0;

    Predicate::new(Box::new(move |rows, incoming, output, _cancel| {
        match (rows, incoming) {
            (RowSelection::Range(range), None) => {
                let slice = &values[range.clone()];
                for (i, &v) in slice.iter().enumerate() {
                    output[i] = v >= lo && v <= hi;
                }
                if has_nulls {
                    for (i, row) in range.enumerate() {
                        if array.is_null(row) {
                            output[i] = false;
                        }
                    }
                }
            }
            (RowSelection::Range(range), Some(mask)) => {
                let slice = &values[range.clone()];
                for (i, &v) in slice.iter().enumerate() {
                    output[i] = (v >= lo && v <= hi) & mask[i];
                }
                if has_nulls {
                    for (i, row) in range.enumerate() {
                        if array.is_null(row) {
                            output[i] = false;
                        }
                    }
                }
            }
            (RowSelection::Indices(indices), None) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = (!has_nulls || array.is_valid(row))
                        && values[row] >= lo
                        && values[row] <= hi;
                }
            }
            (RowSelection::Indices(indices), Some(mask)) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = mask[i]
                        && (!has_nulls || array.is_valid(row))
                        && values[row] >= lo
                        && values[row] <= hi;
                }
            }
        }
        Ok(())
    }))
}

fn scale(array: &ArrayRef) -> i8 {
    match array.data_type() {
        DataType::Decimal32(_, s)
        | DataType::Decimal64(_, s)
        | DataType::Decimal128(_, s)
        | DataType::Decimal256(_, s) => *s,
        _ => unreachable!("decimal dispatch"),
    }
}

pub(in crate::column) fn bind(array: &ArrayRef, lo: Endpoint, hi: Endpoint) -> Predicate<'_> {
    match array.data_type() {
        DataType::Decimal32(..) => native_range(array.as_primitive::<Decimal32Type>(), lo, hi),
        DataType::Decimal64(..) => native_range(array.as_primitive::<Decimal64Type>(), lo, hi),
        DataType::Decimal128(..) => native_range(array.as_primitive::<Decimal128Type>(), lo, hi),
        DataType::Decimal256(..) => native_range(array.as_primitive::<Decimal256Type>(), lo, hi),
        _ => unreachable!("decimal dispatch"),
    }
}

pub(in crate::column) fn search<'a>(array: &'a ArrayRef, text: &str) -> Option<Predicate<'a>> {
    let (lo, hi) = Literal::parse(text)?.search_interval()?;
    let scale = scale(array);
    Some(bind(
        array,
        lo.coefficient(scale, true),
        hi.coefficient(scale, true).preceding(),
    ))
}
