//! Column metadata for presentation. Execution uses the original Arrow type.

use arrow::datatypes::DataType;

use super::{predicate, Column, ColumnCapabilities};
use crate::{
    stats,
    types::{display_type, value_type},
};

impl ColumnCapabilities {
    pub(super) fn from_arrow(data_type: &DataType) -> Self {
        Self {
            text_match: predicate::supports_text_match(data_type),
            range: predicate::range::supports(data_type),
            exact: predicate::supports_equals(data_type),
            scatter: stats::scatter::supports(data_type),
            logarithmic: stats::supports_logarithmic(data_type),
        }
    }
}

/// Coarse value semantics for choosing cell presentation, not operation support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueCategory {
    Number,
    Temporal,
    Text,
}

impl ValueCategory {
    fn from_arrow(data_type: &DataType) -> Self {
        use DataType::*;
        match value_type(data_type) {
            Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64 | Float16 | Float32
            | Float64 | Decimal32(..) | Decimal64(..) | Decimal128(..) | Decimal256(..) => {
                Self::Number
            }
            Date32 | Date64 | Time32(_) | Time64(_) | Timestamp(..) | Duration(_) | Interval(_) => {
                Self::Temporal
            }
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
            capabilities: ColumnCapabilities::from_arrow(data_type),
        }
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
