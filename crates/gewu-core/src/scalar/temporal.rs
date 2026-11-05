//! Temporal text parsing into physical Arrow values.

use std::sync::Arc;

use arrow::{
    array::{downcast_temporal_array, Array, ArrayRef, PrimitiveArray, StringArray},
    compute::cast,
    datatypes::*,
};

use super::bound::Result;

pub(crate) fn parse_value(data_type: &DataType, text: &str) -> Result<i64> {
    parse_physical(data_type, text)?.ok_or_else(|| crate::BoundError::InvalidValue {
        value: text.into(),
        expected: data_type.clone(),
        reason: crate::ValueParseError::InvalidSyntax,
    })
}

/// Cast only the input scalar. Column data always stays in its native array.
pub(crate) fn parse_physical(data_type: &DataType, text: &str) -> Result<Option<i64>> {
    let input: ArrayRef = Arc::new(StringArray::from(vec![text]));
    let typed = cast(&input, data_type).map_err(|_| crate::BoundError::InvalidValue {
        value: text.into(),
        expected: data_type.clone(),
        reason: crate::ValueParseError::InvalidSyntax,
    })?;
    let typed = typed.as_ref();
    Ok(downcast_temporal_array!(typed => physical_scalar(typed),
        _ => unreachable!("temporal scalar dispatch")))
}

fn physical_scalar<T: ArrowPrimitiveType>(array: &PrimitiveArray<T>) -> Option<i64>
where
    T::Native: Into<i64>,
{
    array.is_valid(0).then(|| array.value(0).into())
}
