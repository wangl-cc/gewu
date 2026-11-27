//! Lossless text values for the filter protocol, never presentation labels.
use arrow::{
    array::{
        downcast_dictionary_array, downcast_integer_array, downcast_temporal_array, Array,
        ArrayRef, AsArray,
    },
    datatypes::*,
};

type Result<T> = std::result::Result<T, crate::InternalError>;

pub(crate) fn encode(array: &ArrayRef, row: usize) -> Result<String> {
    use DataType::*;
    if array.is_null(row) {
        return Err(crate::InternalError::Invariant(
            "null has no exact filter representation".into(),
        ));
    }
    match array.data_type() {
        Dictionary(..) => downcast_dictionary_array!(array => {
            let key = array.key(row).ok_or_else(|| crate::InternalError::Invariant("null dictionary key has no exact filter representation".into()))?;
            encode(array.values(), key)
        }, _ => unreachable!()),
        Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64 => {
            downcast_integer_array!(array => Ok(array.value(row).to_string()), _ => unreachable!())
        }
        Float16 => Ok(array.as_primitive::<Float16Type>().value(row).to_string()),
        Float32 => Ok(array.as_primitive::<Float32Type>().value(row).to_string()),
        Float64 => Ok(array.as_primitive::<Float64Type>().value(row).to_string()),
        Decimal32(_, scale) => Ok(format!(
            "{}e{}",
            array.as_primitive::<Decimal32Type>().value(row),
            -i16::from(*scale)
        )),
        Decimal64(_, scale) => Ok(format!(
            "{}e{}",
            array.as_primitive::<Decimal64Type>().value(row),
            -i16::from(*scale)
        )),
        Decimal128(_, scale) => Ok(format!(
            "{}e{}",
            array.as_primitive::<Decimal128Type>().value(row),
            -i16::from(*scale)
        )),
        Decimal256(_, scale) => Ok(format!(
            "{}e{}",
            array.as_primitive::<Decimal256Type>().value(row),
            -i16::from(*scale)
        )),
        Duration(TimeUnit::Second) => Ok(array
            .as_primitive::<DurationSecondType>()
            .value(row)
            .to_string()),
        Duration(TimeUnit::Millisecond) => Ok(array
            .as_primitive::<DurationMillisecondType>()
            .value(row)
            .to_string()),
        Duration(TimeUnit::Microsecond) => Ok(array
            .as_primitive::<DurationMicrosecondType>()
            .value(row)
            .to_string()),
        Duration(TimeUnit::Nanosecond) => Ok(array
            .as_primitive::<DurationNanosecondType>()
            .value(row)
            .to_string()),
        Date32 | Date64 | Time32(_) | Time64(_) | Timestamp(..) => {
            downcast_temporal_array!(array => Ok(format!("@{}", array.value(row))), _ => Err(crate::InternalError::Invariant("unsupported temporal value".into())))
        }
        Boolean => Ok(array.as_boolean().value(row).to_string()),
        Utf8 => Ok(array.as_string::<i32>().value(row).into()),
        LargeUtf8 => Ok(array.as_string::<i64>().value(row).into()),
        Utf8View => Ok(array.as_string_view().value(row).into()),
        _ => Err(crate::InternalError::Invariant(format!(
            "{} has no exact filter representation",
            array.data_type()
        ))),
    }
}

#[cfg(test)]
#[path = "value_text_tests.rs"]
mod tests;
