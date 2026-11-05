//! Dataset and source metadata exposed to callers.

use crate::{ColumnInfo, FileMetadata, FileType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetInfo {
    pub path: String,
    pub row_count: u64,
    pub columns: Vec<ColumnInfo>,
    pub file_size: u64,
    pub format: FileType,
    pub metadata: FileMetadata,
}
