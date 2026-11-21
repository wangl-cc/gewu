//! File format and source metadata exposed to the app.

use gewu_core::{self as core, FileType};

/// List XLSX worksheets in workbook order, including hidden and empty sheets,
/// without decoding their cells. Chart sheets are excluded. Reads workbook
/// metadata and shared strings; call off the main thread. Returns an empty
/// list for a workbook with no worksheets. Does not retain a workbook handle.
#[uniffi::export]
pub fn xlsx_sheet_names(path: String) -> Result<Vec<String>, crate::GewuError> {
    Ok(core::xlsx_sheet_names(path)?)
}

/// One footer metadata entry.
#[derive(uniffi::Record)]
pub struct KeyValue {
    pub key: String,
    pub value: String,
}

#[uniffi::remote(Enum)]
pub enum FileType {
    Csv,
    Tsv,
    Parquet,
    Xlsx,
}

/// Format-specific source details. Mirrors Core's, with key-value pairs as
/// records because UniFFI has no tuples.
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
