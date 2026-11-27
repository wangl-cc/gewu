//! Range predicate compilation over Arrow arrays.

use arrow::{array::ArrayRef, datatypes::DataType};

use super::{
    scalar::{decimal::Endpoint, float, integer, temporal},
    Result,
};
use crate::{
    execution::predicate::Predicate,
    scalar::{bound::invalid_type, Bound},
};

pub(crate) fn supports(data_type: &DataType) -> bool {
    use DataType::*;
    matches!(
        crate::types::value_type(data_type),
        Int8 | Int16
            | Int32
            | Int64
            | UInt8
            | UInt16
            | UInt32
            | UInt64
            | Float16
            | Float32
            | Float64
            | Decimal32(..)
            | Decimal64(..)
            | Decimal128(..)
            | Decimal256(..)
            | Date32
            | Date64
            | Time32(_)
            | Time64(_)
            | Timestamp(..)
            | Duration(_)
    )
}

pub(crate) fn compile_range<'a>(
    array: &'a ArrayRef,
    min: Option<&Bound>,
    max: Option<&Bound>,
) -> Result<Predicate<'a>> {
    let data_type = array.data_type();
    for bound in [min, max].into_iter().flatten() {
        if bound.data_type() != data_type {
            return Err(crate::FilterError::RangeTypeMismatch {
                expected: data_type.clone(),
                actual: bound.data_type().clone(),
            });
        }
    }
    if let Some((lo, hi)) = min.zip(max).filter(|(lo, hi)| lo.compare(hi).is_gt()) {
        return Err(crate::FilterError::ReversedRange {
            min: lo.display().into(),
            max: hi.display().into(),
        });
    }
    if data_type.is_floating() {
        let lower = min.map_or(f64::NEG_INFINITY, |bound| bound.float_lower());
        let upper = max.map_or(f64::INFINITY, |bound| bound.float_upper());
        return Ok(float::bound_range(array, lower, upper));
    }
    let lower = min.map_or(Endpoint::Below, |bound| bound.exact_lower());
    let upper = max.map_or(Endpoint::Above, |bound| bound.exact_upper());
    use DataType::*;
    Ok(match data_type {
        Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64 => {
            integer::bind(array, lower, upper)
        }
        Decimal32(..) | Decimal64(..) | Decimal128(..) | Decimal256(..) => {
            super::scalar::decimal::bind(array, lower, upper)
        }
        Date32 | Date64 | Time32(_) | Time64(_) | Timestamp(..) | Duration(_) => {
            temporal::bind(array, lower, upper)
        }
        _ => return Err(invalid_type(data_type).into()),
    })
}

#[cfg(test)]
#[path = "range_tests.rs"]
mod tests;
