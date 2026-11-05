//! Shared Arrow type structure and display projections.
//!
//! Arrow's [`DataType`] is the source of truth. Each operation owns its
//! support rules and native dispatch.

use arrow::datatypes::DataType;

/// Borrow the value type beneath any dictionary wrappers.
pub(crate) fn value_type(mut data_type: &DataType) -> &DataType {
    while let DataType::Dictionary(_, values) = data_type {
        data_type = values;
    }
    data_type
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
        Boolean | Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64 | Float16
        | Float32 | Float64 | Decimal32(..) | Decimal64(..) | Decimal128(..) | Decimal256(..)
        | Date32 | Date64 | Time32(_) | Time64(_) | Timestamp(..) | Duration(_) | Interval(_)
        | Binary | LargeBinary | BinaryView | FixedSizeBinary(_) | Utf8 | LargeUtf8 | Utf8View => {
            true
        }
        _ => false,
    }
}

pub(crate) fn display_type(data_type: &DataType) -> String {
    use DataType::*;
    match value_type(data_type) {
        Null => "null".into(),
        Boolean => "bool".into(),
        Int8 => "int8".into(),
        Int16 => "int16".into(),
        Int32 => "int32".into(),
        Int64 => "int64".into(),
        UInt8 => "uint8".into(),
        UInt16 => "uint16".into(),
        UInt32 => "uint32".into(),
        UInt64 => "uint64".into(),
        Float16 => "float16".into(),
        Float32 => "float32".into(),
        Float64 => "float64".into(),
        Decimal32(precision, scale)
        | Decimal64(precision, scale)
        | Decimal128(precision, scale)
        | Decimal256(precision, scale) => format!("decimal({precision},{scale})"),
        Date32 | Date64 => "date".into(),
        Time32(_) | Time64(_) => "time".into(),
        Timestamp(_, Some(timezone)) => format!("timestamp({timezone})"),
        Timestamp(_, None) => "timestamp".into(),
        Duration(_) => "duration".into(),
        Interval(_) => "interval".into(),
        Binary | LargeBinary | BinaryView | FixedSizeBinary(_) => "binary".into(),
        Utf8 | LargeUtf8 | Utf8View => "string".into(),
        List(_) | LargeList(_) | ListView(_) | LargeListView(_) | FixedSizeList(..) => {
            "list".into()
        }
        Struct(_) => "struct".into(),
        Map(..) => "map".into(),
        _ => unreachable!("Column validates supported types at construction"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_dictionaries_expose_their_value_type() {
        let encoded = DataType::Dictionary(
            Box::new(DataType::Int8),
            Box::new(DataType::Dictionary(
                Box::new(DataType::Int32),
                Box::new(DataType::Int64),
            )),
        );
        assert_eq!(value_type(&encoded), &DataType::Int64);
        assert!(is_describable(&encoded));
    }
}
