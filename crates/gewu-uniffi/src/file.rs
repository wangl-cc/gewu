//! File and column metadata exposed to the app.

use gewu_core as core;

/// List XLSX worksheets in workbook order, including hidden and empty sheets,
/// without decoding their cells. Chart sheets are excluded. Reads workbook
/// metadata and shared strings; call off the main thread. Returns an empty
/// list for a workbook with no worksheets. Does not retain a workbook handle.
#[uniffi::export]
pub fn xlsx_sheet_names(path: String) -> Result<Vec<String>, crate::GewuError> {
    Ok(core::xlsx_sheet_names(path)?)
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
    pub name: String,
    pub value_category: ValueCategory,
    pub display_type: String,
    pub technical_type: String,
    pub capabilities: ColumnCapabilities,
    pub nullable: bool,
}

#[derive(uniffi::Record)]
pub struct FileInfo {
    pub path: String,
    pub row_count: u64,
    pub columns: Vec<ColumnInfo>,
    pub format: FileType,
    pub file_size: u64,
    pub metadata: FileMetadata,
}

#[derive(uniffi::Record)]
pub struct KeyValue {
    pub key: String,
    pub value: String,
}

#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Csv,
    Tsv,
    Parquet,
    Xlsx,
}

impl From<FileType> for core::FileType {
    fn from(value: FileType) -> Self {
        match value {
            FileType::Csv => Self::Csv,
            FileType::Tsv => Self::Tsv,
            FileType::Parquet => Self::Parquet,
            FileType::Xlsx => Self::Xlsx,
        }
    }
}

impl From<core::FileType> for FileType {
    fn from(value: core::FileType) -> Self {
        match value {
            core::FileType::Csv => Self::Csv,
            core::FileType::Tsv => Self::Tsv,
            core::FileType::Parquet => Self::Parquet,
            core::FileType::Xlsx => Self::Xlsx,
        }
    }
}

#[derive(uniffi::Enum)]
pub enum FileMetadata {
    Parquet {
        row_groups: u32,
        compression: Vec<String>,
        created_by: Option<String>,
        /// Footer key-value metadata (Arrow's own schema left out).
        key_values: Vec<KeyValue>,
    },
    Delimited,
    Xlsx {
        sheet: String,
        sheets: Vec<String>,
    },
}

impl From<core::FileMetadata> for FileMetadata {
    fn from(value: core::FileMetadata) -> Self {
        match value {
            core::FileMetadata::Parquet {
                row_groups,
                compression,
                created_by,
                key_values,
            } => Self::Parquet {
                row_groups,
                compression,
                created_by,
                key_values: key_values
                    .into_iter()
                    .map(|(key, value)| KeyValue { key, value })
                    .collect(),
            },
            core::FileMetadata::Delimited => Self::Delimited,
            core::FileMetadata::Xlsx { sheet, sheets } => Self::Xlsx { sheet, sheets },
        }
    }
}

impl From<core::FileInfo> for FileInfo {
    fn from(i: core::FileInfo) -> Self {
        Self {
            path: i.path,
            row_count: i.row_count,
            columns: i
                .columns
                .into_iter()
                .map(|c| ColumnInfo {
                    name: c.name,
                    value_category: c.value_category.into(),
                    display_type: c.display_type,
                    technical_type: c.technical_type,
                    capabilities: c.capabilities.into(),
                    nullable: c.nullable,
                })
                .collect(),
            format: i.format.into(),
            file_size: i.file_size,
            metadata: i.metadata.into(),
        }
    }
}
