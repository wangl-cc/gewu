//! Column metadata for presentation. Execution uses the original Arrow type.

use arrow::datatypes::{DataType, Field};

use super::{predicate, ColumnCapabilities, ColumnId};
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
    pub id: ColumnId,
    pub name: String,
    pub value_category: ValueCategory,
    /// Floating statistics may follow the user's display precision; integer and decimal labels are
    /// exact.
    pub floating_point: bool,
    pub display_type: String,
    pub technical_type: String,
    /// Whether the source schema permits null, regardless of this dataset’s missing count.
    pub nullable: bool,
    /// Lists, structs and maps. Only Parquet exports them structurally; other
    /// formats need [`crate::ExportOptions::nested_as_text`].
    pub nested: bool,
    pub capabilities: ColumnCapabilities,
    pub analysis: crate::AnalysisCapabilities,
}

impl ColumnInfo {
    pub(crate) fn from_field(id: ColumnId, field: &Field) -> Self {
        let data_type = field.data_type();
        Self {
            id,
            name: field.name().clone(),
            value_category: ValueCategory::from_arrow(data_type),
            floating_point: value_type(data_type).is_floating(),
            display_type: display_type(data_type),
            technical_type: data_type.to_string(),
            nullable: field.is_nullable(),
            nested: data_type.is_nested(),
            capabilities: ColumnCapabilities::from_arrow(data_type),
            analysis: crate::stats::capabilities(data_type),
        }
    }
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
