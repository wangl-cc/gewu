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

/// Time unit for timestamps and durations; mirrors `arrow::datatypes::TimeUnit`.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum TimeUnit {
    Second,
    Millisecond,
    Microsecond,
    Nanosecond,
}

impl From<core::TimeUnit> for TimeUnit {
    fn from(u: core::TimeUnit) -> Self {
        match u {
            core::TimeUnit::Second => Self::Second,
            core::TimeUnit::Millisecond => Self::Millisecond,
            core::TimeUnit::Microsecond => Self::Microsecond,
            core::TimeUnit::Nanosecond => Self::Nanosecond,
        }
    }
}

/// A logical data type with semantic parameters; mirrors `gewu_core::LogicalType`.
#[derive(uniffi::Enum, Clone, PartialEq, Eq)]
pub enum LogicalType {
    Text,
    Integer {
        bits: u8,
        signed: bool,
    },
    Float {
        bits: u8,
    },
    Decimal {
        precision: u8,
        scale: i8,
    },
    Boolean,
    Date,
    Time,
    Timestamp {
        unit: TimeUnit,
        timezone: Option<String>,
    },
    Duration {
        unit: TimeUnit,
    },
    Interval,
    Binary,
    List,
    Struct,
    Dictionary {
        value_type: Box<LogicalType>,
    },
    Other {
        raw_name: String,
    },
}

impl From<core::LogicalType> for LogicalType {
    fn from(t: core::LogicalType) -> Self {
        match t {
            core::LogicalType::Text => Self::Text,
            core::LogicalType::Integer { bits, signed } => Self::Integer { bits, signed },
            core::LogicalType::Float { bits } => Self::Float { bits },
            core::LogicalType::Decimal { precision, scale } => Self::Decimal { precision, scale },
            core::LogicalType::Boolean => Self::Boolean,
            core::LogicalType::Date => Self::Date,
            core::LogicalType::Time => Self::Time,
            core::LogicalType::Timestamp { unit, timezone } => Self::Timestamp {
                unit: unit.into(),
                timezone,
            },
            core::LogicalType::Duration { unit } => Self::Duration { unit: unit.into() },
            core::LogicalType::Interval => Self::Interval,
            core::LogicalType::Binary => Self::Binary,
            core::LogicalType::List => Self::List,
            core::LogicalType::Struct => Self::Struct,
            core::LogicalType::Dictionary { value_type } => Self::Dictionary {
                value_type: Box::new((*value_type).into()),
            },
            core::LogicalType::Other { raw_name } => Self::Other { raw_name },
        }
    }
}

#[derive(uniffi::Record)]
pub struct ColumnCapabilities {
    pub contains: bool,
    pub range: bool,
    pub equals: bool,
    pub histogram: bool,
    pub scatter: bool,
    pub logarithmic: bool,
}

impl From<core::ColumnCapabilities> for ColumnCapabilities {
    fn from(capabilities: core::ColumnCapabilities) -> Self {
        Self {
            contains: capabilities.contains,
            range: capabilities.range,
            equals: capabilities.equals,
            histogram: capabilities.histogram,
            scatter: capabilities.scatter,
            logarithmic: capabilities.logarithmic,
        }
    }
}

#[derive(uniffi::Record)]
pub struct ColumnInfo {
    pub name: String,
    pub logical_type: LogicalType,
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

impl From<&core::Session> for FileInfo {
    fn from(document: &core::Session) -> Self {
        let i = document.info();
        let mut info = FileInfo {
            path: i.path.clone(),
            row_count: i.row_count,
            columns: i
                .columns
                .iter()
                .enumerate()
                .map(|(column, c)| {
                    let capabilities = document
                        .column_capabilities(column as u32)
                        .expect("column came from this document's schema");
                    ColumnInfo {
                        name: c.name.clone(),
                        logical_type: c.logical_type.clone().into(),
                        display_type: c.display_type.clone(),
                        technical_type: c.technical_type.clone(),
                        capabilities: capabilities.into(),
                        nullable: c.nullable,
                    }
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
        match &i.metadata {
            core::FileMetadata::Parquet {
                row_groups,
                compression,
                created_by,
                key_values,
            } => {
                info.row_groups = Some(*row_groups);
                info.compression = compression.clone();
                info.created_by = created_by.clone();
                info.metadata = key_values
                    .iter()
                    .map(|(key, value)| KeyValue {
                        key: key.clone(),
                        value: value.clone(),
                    })
                    .collect();
            }
            core::FileMetadata::Delimited => {}
            core::FileMetadata::Xlsx { sheet, sheets } => {
                info.sheet = Some(sheet.clone());
                info.sheets = sheets.clone();
            }
        }
        info
    }
}
