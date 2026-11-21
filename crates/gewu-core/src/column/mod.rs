//! First-class persistent Column model and column-level operations.
//!
//! Type semantics live in [`crate::types`]; this module owns the column's data,
//! its lazy distinct-value dictionary, and the per-column kernels:
//! - [`dictionary`]: compact distinct-value dictionary encoding
//! - [`predicate`]: Single-column filter and Find predicate compilation

pub(crate) mod dictionary;
mod error;
mod id;
mod metadata;
pub(crate) mod predicate;
pub(crate) mod value_text;

use std::sync::{Arc, Mutex, OnceLock};

use arrow::array::ArrayRef;
pub(crate) use dictionary::Dictionary;
pub use error::{ColumnError, ColumnOperation, FilterError, FilterKind, ValueParseError};
pub use id::ColumnId;
pub use metadata::{ColumnInfo, ValueCategory};

use crate::{execution::predicate::Selector, types, FileError, Filter};

pub(crate) type Result<T> = std::result::Result<T, ColumnError>;

/// Operations supported by a column's type. Value-list availability
/// additionally depends on cardinality and is reported by `crate::FilterChoices`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnCapabilities {
    /// Text filters: contains and whole-value equality.
    pub text_match: bool,
    pub range: bool,
    pub exact: bool,
    /// Direct numeric and temporal arrays support approximate plot coordinates.
    pub scatter: bool,
    /// Direct numeric values support positive-only logarithmic charts.
    pub logarithmic: bool,
}

/// Complete bounded non-null candidates, or None when no bounded dictionary is available.
#[derive(Debug, Clone, PartialEq)]
pub struct FilterChoices {
    pub values: Option<Vec<ValueChoice>>,
    pub null_count: u64,
}

/// A filter candidate with separate display and lossless input text.
#[derive(Debug, Clone, PartialEq)]
pub struct ValueChoice {
    pub label: String,
    pub value: String,
    pub count: u64,
}

/// An immutable, first-class column owned by a [`crate::Dataset`].
pub(crate) struct Column {
    name: String,
    nullable: bool,
    array: ArrayRef,
    /// Lazy compact encoding of the column's distinct values. `None` is
    /// cached when the type cannot be encoded or has too many values.
    dictionary: OnceLock<Option<Arc<Dictionary>>>,
    dictionary_lock: Mutex<()>,
}

impl Column {
    /// Build a column. Fails for Arrow types without Gewu semantics rather
    /// than exposing a column no operation can describe.
    pub(crate) fn new(
        name: impl Into<String>,
        array: ArrayRef,
    ) -> std::result::Result<Self, FileError> {
        let name = name.into();
        if !types::is_describable(array.data_type()) {
            return Err(crate::FileError::UnsupportedColumnType {
                column: name,
                data_type: array.data_type().clone(),
            });
        }
        let nullable = array.is_nullable();
        Ok(Self {
            name,
            nullable,
            array,
            dictionary: OnceLock::new(),
            dictionary_lock: Mutex::new(()),
        })
    }

    pub(crate) fn dictionary(&self) -> Result<Option<&Arc<Dictionary>>> {
        if let Some(value) = self.dictionary.get() {
            return Ok(value.as_ref());
        }
        let _guard = self
            .dictionary_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(value) = self.dictionary.get() {
            return Ok(value.as_ref());
        }
        let value = Dictionary::build(&self.array)?.map(Arc::new);
        Ok(self.dictionary.get_or_init(|| value).as_ref())
    }

    pub(crate) fn array(&self) -> &ArrayRef {
        &self.array
    }

    pub(crate) fn info(&self, id: ColumnId) -> ColumnInfo {
        ColumnInfo::from_column(id, self)
    }

    pub(crate) fn compile_filter<'a>(&'a self, filter: &Filter) -> predicate::Result<Selector<'a>> {
        predicate::compile_filter(self, filter)
    }
}

#[cfg(test)]
mod tests;
