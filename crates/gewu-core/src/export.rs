//! Export an immutable view snapshot using bounded working buffers. An export
//! never changes its document and only replaces its destination after success.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use arrow::{array::ArrayRef, datatypes::SchemaRef};

use crate::{
    file::{self, ExportData, WriteProgress},
    Error, FileType, Result,
};

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
    rows: Arc<Vec<u32>>,
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
        rows: Arc<Vec<u32>>,
        format: FileType,
    ) -> Self {
        Self {
            source,
            schema,
            columns,
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
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Write on a background thread. Headers are included for CSV/TSV, nulls
    /// are empty fields, and scalar values use Arrow's full display precision.
    /// CSV/TSV and XLSX reject nested columns. Empty strings and nulls both
    /// use empty CSV/TSV fields under the standard Arrow CSV encoding.
    /// Parquet preserves Arrow types and nulls. A sibling temporary file is
    /// atomically renamed only after the complete output has been flushed.
    ///
    /// The source file (including aliases), an invalid destination, reuse of
    /// this job, formatting failures, and cancellation return errors without
    /// replacing the destination. A failed job cannot be retried.
    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        if self.started.swap(true, Ordering::Relaxed) {
            return Err(Error::InvalidArgument(
                "this export has already been started".into(),
            ));
        }
        self.check_cancelled()?;
        let path = path.as_ref();
        self.check_destination(path)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temporary = tempfile::Builder::new()
            .prefix(".gewu-export-")
            .tempfile_in(parent)?;
        file::write(
            temporary.as_file_mut(),
            self.format,
            self.data(),
            self.write_progress(),
        )?;
        self.check_cancelled()?;
        temporary.as_file().sync_all()?;
        let mut committed = self.committed.lock().unwrap();
        self.check_cancelled()?;
        self.check_destination(path)?;
        temporary.persist(path).map_err(|error| error.error)?;
        *committed = true;
        Ok(())
    }

    fn data(&self) -> ExportData<'_> {
        ExportData {
            schema: &self.schema,
            columns: &self.columns,
            rows: &self.rows,
        }
    }

    fn write_progress(&self) -> WriteProgress<'_> {
        WriteProgress {
            cancelled: &self.cancelled,
            rows_written: &self.rows_written,
        }
    }

    fn check_destination(&self, path: &Path) -> Result<()> {
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
            return Err(Error::InvalidArgument(
                "choose a different path from the open source file".into(),
            ));
        }
        Ok(())
    }
}
