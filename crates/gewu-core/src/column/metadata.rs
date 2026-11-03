//! Column metadata for presentation. Execution uses the original Arrow type.

use arrow::datatypes::DataType;

use super::{Column, ColumnCapabilities};
use crate::types::{domain, value_type, Domain};

/// Coarse value semantics for choosing cell presentation, not operation support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueCategory {
    Number,
    Temporal,
    Text,
}

impl ValueCategory {
    fn from_arrow(data_type: &DataType) -> Self {
        match domain(data_type) {
            Some(Domain::Integer | Domain::Float | Domain::Decimal) => Self::Number,
            Some(
                Domain::Date
                | Domain::Time
                | Domain::Timestamp
                | Domain::Duration
                | Domain::Interval,
            ) => Self::Temporal,
            _ => Self::Text,
        }
    }
}

/// Display information and supported operations for one column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub value_category: ValueCategory,
    pub display_type: String,
    pub technical_type: String,
    pub nullable: bool,
    pub capabilities: ColumnCapabilities,
}

impl ColumnInfo {
    pub(super) fn from_column(column: &Column) -> Self {
        let data_type = column.array.data_type();
        Self {
            name: column.name.clone(),
            value_category: ValueCategory::from_arrow(data_type),
            display_type: display_type(data_type),
            technical_type: data_type.to_string(),
            nullable: column.nullable,
            capabilities: column.capabilities(),
        }
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
    use std::sync::Arc;

    use arrow::{
        array::new_null_array,
        datatypes::{Field, TimeUnit},
    };

    use super::*;

    #[test]
    fn metadata_preserves_labels_and_projects_cell_categories() {
        for (data_type, category, label) in [
            (DataType::UInt64, ValueCategory::Number, "uint64"),
            (
                DataType::Decimal256(76, -4),
                ValueCategory::Number,
                "decimal(76,-4)",
            ),
            (
                DataType::Timestamp(TimeUnit::Nanosecond, Some("Europe/London".into())),
                ValueCategory::Temporal,
                "timestamp(Europe/London)",
            ),
            (DataType::Boolean, ValueCategory::Text, "bool"),
            (DataType::Utf8, ValueCategory::Text, "string"),
            (DataType::Binary, ValueCategory::Text, "binary"),
            (
                DataType::List(Arc::new(Field::new("item", DataType::Int32, true))),
                ValueCategory::Text,
                "list",
            ),
            (
                DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::Int64)),
                ValueCategory::Number,
                "int64",
            ),
        ] {
            let column = Column::new("value", new_null_array(&data_type, 1)).unwrap();
            let info = column.info();
            assert_eq!(info.name, "value");
            assert_eq!(info.value_category, category);
            assert_eq!(info.display_type, label);
            assert_eq!(info.technical_type, data_type.to_string());
            assert!(info.nullable);
            assert_eq!(
                info.capabilities.text_match,
                matches!(data_type, DataType::Utf8)
            );
        }
    }
}
