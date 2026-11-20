//! Column metadata exposed to the app.

use gewu_core as core;

#[derive(uniffi::Record, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColumnId {
    pub raw: u32,
}

impl From<core::ColumnId> for ColumnId {
    fn from(id: core::ColumnId) -> Self {
        Self { raw: id.into_raw() }
    }
}

impl From<ColumnId> for core::ColumnId {
    fn from(id: ColumnId) -> Self {
        core::ColumnId::from_raw(id.raw)
    }
}

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
    pub id: ColumnId,
    pub name: String,
    pub value_category: ValueCategory,
    pub display_type: String,
    pub technical_type: String,
    pub nullable: bool,
    /// Lists, structs and maps; only Parquet exports them structurally.
    pub nested: bool,
    pub capabilities: ColumnCapabilities,
    pub analysis: crate::AnalysisCapabilities,
}

impl From<core::ColumnInfo> for ColumnInfo {
    fn from(column: core::ColumnInfo) -> Self {
        Self {
            id: column.id.into(),
            name: column.name,
            value_category: column.value_category.into(),
            display_type: column.display_type,
            technical_type: column.technical_type,
            nullable: column.nullable,
            nested: column.nested,
            capabilities: column.capabilities.into(),
            analysis: column.analysis.into(),
        }
    }
}
