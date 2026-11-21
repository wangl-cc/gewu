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
        let id = ColumnId::new(3);
        let info = column.info(id);
        assert_eq!(info.id, id);
        assert_eq!(info.name, "value");
        assert_eq!(info.value_category, category);
        assert_eq!(info.display_type, label);
        assert_eq!(info.technical_type, data_type.to_string());
        assert!(info.nullable);
        assert_eq!(
            info.capabilities.text_match,
            matches!(data_type, DataType::Utf8)
        );
        assert_eq!(info.analysis, crate::stats::capabilities(&data_type));
    }
}
