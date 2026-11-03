//! File format and source metadata exposed to the app.

use gewu_core as core;

/// List XLSX worksheets in workbook order, including hidden and empty sheets,
/// without decoding their cells. Chart sheets are excluded. Reads workbook
/// metadata and shared strings; call off the main thread. Returns an empty
/// list for a workbook with no worksheets. Does not retain a workbook handle.
#[uniffi::export]
pub fn xlsx_sheet_names(path: String) -> Result<Vec<String>, crate::GewuError> {
    Ok(core::xlsx_sheet_names(path)?)
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
