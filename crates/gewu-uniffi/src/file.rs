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
    pub format: String,
    pub row_groups: Option<u32>,
    pub file_size: u64,
    pub compression: Vec<String>,
    pub created_by: Option<String>,
    /// Footer key-value metadata (Arrow's own schema left out).
    pub metadata: Vec<KeyValue>,
    pub sheet: Option<String>,
    pub sheets: Vec<String>,
}

#[derive(uniffi::Record)]
pub struct KeyValue {
    pub key: String,
    pub value: String,
}

#[derive(uniffi::Enum)]
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

impl From<core::FileInfo> for FileInfo {
    fn from(i: core::FileInfo) -> Self {
        let mut info = FileInfo {
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
            format: i.format.to_string(),
            row_groups: None,
            file_size: i.file_size,
            compression: Vec::new(),
            created_by: None,
            metadata: Vec::new(),
            sheet: None,
            sheets: Vec::new(),
        };
        match i.metadata {
            core::FileMetadata::Parquet {
                row_groups,
                compression,
                created_by,
                key_values,
            } => {
                info.row_groups = Some(row_groups);
                info.compression = compression;
                info.created_by = created_by;
                info.metadata = key_values
                    .into_iter()
                    .map(|(key, value)| KeyValue { key, value })
                    .collect();
            }
            core::FileMetadata::Delimited => {}
            core::FileMetadata::Xlsx { sheet, sheets } => {
                info.sheet = Some(sheet);
                info.sheets = sheets;
            }
        }
        info
    }
}
