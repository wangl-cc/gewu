//! Column metadata exposed to the app.

use gewu_core::{AnalysisCapabilities, ColumnCapabilities, ColumnId, ColumnInfo, ValueCategory};

uniffi::custom_type!(ColumnId, u32, {
    remote,
    lower: |id| id.into_raw(),
    try_lift: |raw| Ok(ColumnId::from_raw(raw)),
});

/// Coarse value semantics for cell presentation. Use capabilities for operations.
#[uniffi::remote(Enum)]
pub enum ValueCategory {
    Number,
    Temporal,
    Text,
}

#[uniffi::remote(Record)]
pub struct ColumnCapabilities {
    /// Text filters: contains and whole-value equality.
    pub text_match: bool,
    pub range: bool,
    pub exact: bool,
    pub scatter: bool,
    pub logarithmic: bool,
}

#[uniffi::remote(Record)]
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
    pub analysis: AnalysisCapabilities,
}
