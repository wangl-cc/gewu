//! Export an immutable view snapshot using bounded working buffers. An export
//! never changes its document and only replaces its destination after success.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
};

use arrow::{
    array::ArrayRef,
    datatypes::{DataType, Field, Schema, SchemaRef},
};

use super::{ExportData, WriteProgress};
use crate::{row_ids::RowIds, ColumnId, ExportOptions, FileType, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportProgress {
    /// Rows processed so far. Reaching `total_rows` does not imply the final
    /// file is committed; wait for `write` to return successfully.
    pub rows_written: u64,
    pub total_rows: u64,
}

/// A single-use export with its own cancellation and progress. The snapshot
/// shares immutable Arrow buffers and row indices with its source document.
pub struct Export {
    source: PathBuf,
    schema: SchemaRef,
    columns: Vec<ArrayRef>,
    /// Source column identities, in export order.
    column_ids: Vec<ColumnId>,
    rows: RowIds,
    format: FileType,
    started: AtomicBool,
    cancelled: AtomicBool,
    rows_written: AtomicU64,
    /// Serializes cancellation with the final rename. Cancellation after that
    /// commit is a no-op; before it, the destination is left untouched.
    committed: Mutex<bool>,
}

impl Export {
    pub(crate) fn new(
        source: PathBuf,
        schema: SchemaRef,
        columns: Vec<ArrayRef>,
        column_ids: Vec<ColumnId>,
        rows: RowIds,
        format: FileType,
    ) -> Self {
        Self {
            source,
            schema,
            columns,
            column_ids,
            rows,
            format,
            started: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            rows_written: AtomicU64::new(0),
            committed: Mutex::new(false),
        }
    }

    pub fn progress(&self) -> ExportProgress {
        ExportProgress {
            rows_written: self.rows_written.load(Ordering::Relaxed),
            total_rows: self.rows.len() as u64,
        }
    }

    pub fn cancel(&self) {
        let committed = self.committed.lock().unwrap();
        if !*committed {
            self.cancelled.store(true, Ordering::Relaxed);
        }
    }

    fn check_cancelled(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(crate::ExportError::Cancelled.into())
        } else {
            Ok(())
        }
    }

    /// Write on a background thread. Headers are included for CSV/TSV, nulls
    /// are empty fields, and scalar values use Arrow's full display precision.
    /// CSV/TSV and XLSX reject nested columns unless
    /// [`ExportOptions::nested_as_text`] is set. Empty strings and nulls both
    /// use empty CSV/TSV fields under the standard Arrow CSV encoding.
    /// Parquet preserves Arrow types and nulls. A sibling temporary file is
    /// atomically renamed only after the complete output has been flushed.
    ///
    /// The source file (including aliases), an invalid destination, reuse of
    /// this job, formatting failures, and cancellation return errors without
    /// replacing the destination. A failed job cannot be retried.
    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        self.write_with_options(path, &ExportOptions::default())
    }

    /// Write with explicit encoding options, with the same single-use and
    /// atomic destination semantics as [`Self::write`]. Defaults use Zstd for Parquet.
    pub fn write_with_options(
        &self,
        path: impl AsRef<Path>,
        options: &ExportOptions,
    ) -> Result<()> {
        self.start(options)?;
        let path = path.as_ref();
        self.check_destination(path)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temporary = tempfile::Builder::new()
            .prefix(".gewu-export-")
            .tempfile_in(parent)
            .map_err(|error| {
                crate::IoError::new(crate::IoOperation::CreateTemporaryFile, parent, error)
            })?;
        let as_text = self.as_text(options);
        super::write(
            temporary.as_file_mut(),
            self.format,
            options,
            self.data(&as_text),
            self.write_progress(),
        )?;
        self.check_cancelled()?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| crate::IoError::new(crate::IoOperation::Sync, path, error))?;
        let mut committed = self.committed.lock().unwrap();
        self.check_cancelled()?;
        self.check_destination(path)?;
        temporary
            .persist(path)
            .map_err(|error| crate::IoError::new(crate::IoOperation::Commit, path, error.error))?;
        *committed = true;
        Ok(())
    }

    /// Write a CSV or TSV export into memory, for example for the clipboard.
    /// Fails with [`crate::ExportError::SizeLimit`] instead of building more
    /// than `limit` bytes. Single-use and cancellable like a file write.
    pub fn write_to_memory(&self, options: &ExportOptions, limit: usize) -> Result<Vec<u8>> {
        self.start(options)?;
        let as_text = self.as_text(options);
        let bytes = super::write_delimited_to_memory(
            self.format,
            options,
            self.data(&as_text),
            self.write_progress(),
            limit,
        )?;
        self.check_cancelled()?;
        Ok(bytes)
    }

    /// Claim this single-use job and check that `options` can write its columns.
    fn start(&self, options: &ExportOptions) -> Result<()> {
        if self.started.swap(true, Ordering::Relaxed) {
            return Err(crate::ExportError::AlreadyStarted.into());
        }
        self.check_cancelled()?;
        if self.format != FileType::Parquet && !options.nested_as_text {
            let nested: Vec<ColumnId> = self
                .column_ids
                .iter()
                .zip(&self.columns)
                .filter(|(_, array)| array.data_type().is_nested())
                .map(|(&id, _)| id)
                .collect();
            if !nested.is_empty() {
                return Err(crate::ExportError::NestedColumns {
                    format: self.format,
                    columns: nested,
                }
                .into());
            }
        }
        Ok(())
    }

    /// Which columns to write as display text under `options`.
    fn as_text(&self, options: &ExportOptions) -> Vec<bool> {
        self.columns
            .iter()
            .map(|array| {
                options.nested_as_text
                    && self.format != FileType::Parquet
                    && array.data_type().is_nested()
            })
            .collect()
    }

    fn data<'a>(&'a self, as_text: &'a [bool]) -> ExportData<'a> {
        let schema = if as_text.contains(&true) {
            let fields: Vec<Field> = self
                .schema
                .fields()
                .iter()
                .zip(as_text)
                .map(|(field, &as_text)| {
                    if as_text {
                        Field::new(field.name(), DataType::Utf8, field.is_nullable())
                    } else {
                        field.as_ref().clone()
                    }
                })
                .collect();
            std::sync::Arc::new(Schema::new_with_metadata(
                fields,
                self.schema.metadata().clone(),
            ))
        } else {
            self.schema.clone()
        };
        ExportData {
            schema,
            columns: &self.columns,
            rows: self.rows.as_slice(),
            as_text,
        }
    }

    fn write_progress(&self) -> WriteProgress<'_> {
        WriteProgress {
            cancelled: &self.cancelled,
            rows_written: &self.rows_written,
        }
    }

    fn check_destination(&self, path: &Path) -> Result<(), crate::ExportError> {
        let same_path = match (self.source.canonicalize(), path.canonicalize()) {
            (Ok(source), Ok(destination)) => source == destination,
            _ => false,
        };
        // Hard links have different canonical paths but share the same file.
        #[cfg(unix)]
        let same_file = {
            use std::os::unix::fs::MetadataExt;
            match (std::fs::metadata(&self.source), std::fs::metadata(path)) {
                (Ok(source), Ok(destination)) => {
                    source.dev() == destination.dev() && source.ino() == destination.ino()
                }
                _ => false,
            }
        };
        #[cfg(not(unix))]
        let same_file = false;
        if same_path || same_file {
            return Err(crate::ExportError::SourceIsDestination {
                source_path: self.source.clone(),
                destination: path.to_owned(),
            });
        }
        Ok(())
    }
}
