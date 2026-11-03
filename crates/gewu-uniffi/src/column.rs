//! Column metadata exposed to the app.

use gewu_core as core;

/// Coarse value semantics for cell presentation. Use capabilities for operations.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum ValueCategory {
    Number,
    Temporal,
    Text,
}

impl From<core::ValueCategory> for ValueCategory {
    fn from(category: core::ValueCategory) -> Self {
        match category {
            core::ValueCategory::Number => Self::Number,
            core::ValueCategory::Temporal => Self::Temporal,
            core::ValueCategory::Text => Self::Text,
        }
    }
}

#[derive(uniffi::Record)]
pub struct ColumnCapabilities {
    /// Text filters: contains and whole-value equality.
    pub text_match: bool,
    pub range: bool,
    pub exact: bool,
    pub scatter: bool,
    pub logarithmic: bool,
}

impl From<core::ColumnCapabilities> for ColumnCapabilities {
    fn from(capabilities: core::ColumnCapabilities) -> Self {
        Self {
            text_match: capabilities.text_match,
            range: capabilities.range,
            exact: capabilities.exact,
            scatter: capabilities.scatter,
            logarithmic: capabilities.logarithmic,
        }
    }
}

#[derive(uniffi::Record)]
pub struct ColumnInfo {
    pub name: String,
    pub value_category: ValueCategory,
    pub display_type: String,
    pub technical_type: String,
    pub capabilities: ColumnCapabilities,
    pub nullable: bool,
}

impl From<core::ColumnInfo> for ColumnInfo {
    fn from(column: core::ColumnInfo) -> Self {
        Self {
            name: column.name,
            value_category: column.value_category.into(),
            display_type: column.display_type,
            technical_type: column.technical_type,
            capabilities: column.capabilities.into(),
            nullable: column.nullable,
        }
    }
}
