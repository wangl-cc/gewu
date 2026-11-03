//! Shared Arrow type descriptions and scalar classifications.
//!
//! Arrow's [`DataType`] is the source of truth. This module describes values
//! and storage; each operation owns its support rules and native dispatch.

use arrow::datatypes::DataType;

/// How Arrow stores a type. Dictionary encoding is lossless and does not
/// change the value domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Encoding {
    Direct,
    Dictionary,
}

/// The parameter-free scalar class that policy code matches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Domain {
    Text,
    Boolean,
    Integer,
    Float,
    Decimal,
    Date,
    Time,
    Timestamp,
    Duration,
    Interval,
    Binary,
}

/// Storage encoding of the outermost Arrow type.
pub(crate) fn encoding(data_type: &DataType) -> Encoding {
    match data_type {
        DataType::Dictionary(..) => Encoding::Dictionary,
        _ => Encoding::Direct,
    }
}

/// Borrow the value type beneath any dictionary wrappers.
pub(crate) fn value_type(mut data_type: &DataType) -> &DataType {
    while let DataType::Dictionary(_, values) = data_type {
        data_type = values;
    }
    data_type
}

/// Scalar semantics, independent of dictionary encoding.
pub(crate) fn domain(data_type: &DataType) -> Option<Domain> {
    direct_domain(value_type(data_type))
}

/// Scalar class of a native array. Dictionary keys are not scalar values.
pub(crate) fn direct_domain(data_type: &DataType) -> Option<Domain> {
    use DataType::*;
    Some(match data_type {
        Utf8 | LargeUtf8 | Utf8View => Domain::Text,
        Boolean => Domain::Boolean,
        Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64 => Domain::Integer,
        Float16 | Float32 | Float64 => Domain::Float,
        Decimal32(..) | Decimal64(..) | Decimal128(..) | Decimal256(..) => Domain::Decimal,
        Date32 | Date64 => Domain::Date,
        Time32(_) | Time64(_) => Domain::Time,
        Timestamp(..) => Domain::Timestamp,
        Duration(_) => Domain::Duration,
        Interval(_) => Domain::Interval,
        Binary | LargeBinary | BinaryView | FixedSizeBinary(_) => Domain::Binary,
        _ => return None,
    })
}

/// Whether the entire schema has a display description, including children.
/// This is not a claim that any particular operation supports the type.
pub(crate) fn is_describable(data_type: &DataType) -> bool {
    use DataType::*;
    match data_type {
        Null => true,
        Dictionary(_, values) => is_describable(values),
        List(field)
        | LargeList(field)
        | ListView(field)
        | LargeListView(field)
        | FixedSizeList(field, _) => is_describable(field.data_type()),
        Struct(fields) => fields.iter().all(|field| is_describable(field.data_type())),
        Map(field, _) => match field.data_type() {
            Struct(fields) if fields.len() == 2 => {
                fields.iter().all(|field| is_describable(field.data_type()))
            }
            _ => false,
        },
        _ => direct_domain(data_type).is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_preserves_domain_but_not_native_dispatch() {
        let encoded = DataType::Dictionary(
            Box::new(DataType::Int8),
            Box::new(DataType::Dictionary(
                Box::new(DataType::Int32),
                Box::new(DataType::Int64),
            )),
        );
        assert_eq!(domain(&encoded), Some(Domain::Integer));
        assert_eq!(direct_domain(&encoded), None);
        assert_eq!(encoding(&encoded), Encoding::Dictionary);
        assert_eq!(value_type(&encoded), &DataType::Int64);
    }
}
