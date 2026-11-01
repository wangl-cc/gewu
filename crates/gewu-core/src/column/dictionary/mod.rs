//! Distinct values of a column as a compact `u8` codebook.
//!
//! A [`ValueDictionary`] gives every source row a code naming its distinct
//! value: codes `0..=254` name the non-null values in value order, and
//! [`NULL_CODE`] names null. Everything about how a physical Arrow array
//! becomes that codebook lives here: native dictionaries reuse their keys and
//! values, primitive arrays scan natively, and only text, binary and nested
//! arrays fall back to Arrow's row format. Other modules consume the result
//! through the methods below; they never see codes, values or encoding
//! strategy.
//!
//! The codebook is a pure function of the column data and is cached by
//! [`crate::Column`]. Columns with more than [`MAX_DISTINCT_VALUES`] non-null
//! values are not encoded; callers fall back to streaming counts, which this
//! module also provides through [`counted`].

use std::{collections::HashSet, sync::Arc};

use arrow::{
    array::{Array, ArrayRef, UInt32Array},
    compute::take,
};
use streaming::{count_values, ValueCounts};

use crate::{
    column::format, predicate::Predicate, scope::Selection, selection::RowSelection, Result,
};

mod encode;
mod primitive;
mod streaming;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistinctValue {
    /// Stable for the lifetime of the document; pass it to [`crate::Filter::InValues`].
    /// Codes `0..=254` name non-null values and [`NULL_CODE`] names null.
    pub key: u64,
    /// Displayed text; `None` is the null value.
    pub label: Option<String>,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistinctResult {
    /// Sorted by value, nulls first.
    pub values: Vec<DistinctValue>,
    /// Number of distinct values before `limit` was applied.
    pub total: u64,
    pub truncated: bool,
    /// Whether the keys name values for [`crate::Filter::InValues`]. False for
    /// columns with too many distinct values to encode; `values` then holds
    /// the most common ones.
    pub keyed: bool,
}

/// Most non-null values a column may have and still be encoded. Non-null
/// values use codes `0..=254`; [`NULL_CODE`] is reserved for null, so one
/// `u8` code space covers 255 values plus null. Columns with more values are
/// counted without an encoding and use streaming counts/searches.
pub(crate) const MAX_DISTINCT_VALUES: usize = 255;

/// Code reserved for null. Non-null values use codes `0..=254`.
pub(crate) const NULL_CODE: u8 = 255;

/// Rows converted to row format at a time, to bound memory.
const CHUNK: usize = 64 * 1024;

pub(crate) struct ValueDictionary {
    /// Code of each source row; [`NULL_CODE`] for null.
    codes: Vec<u8>,
    /// Non-null values in code order; index `code` formats that value.
    values: ArrayRef,
    /// Whether the source column contains at least one null.
    has_null: bool,
}

impl ValueDictionary {
    #[cfg(test)]
    fn n_values(&self) -> usize {
        self.values.len()
    }

    #[cfg(test)]
    fn has_null(&self) -> bool {
        self.has_null
    }

    #[cfg(test)]
    fn code_at(&self, row: usize) -> u8 {
        self.codes[row]
    }

    /// Each distinct value with its count over `rows`, in value order with
    /// null first, leaving out values with no rows.
    pub(crate) fn counts(&self, rows: &Selection) -> Result<Vec<DistinctValue>> {
        self.counts_with_zero(rows, false)
    }

    /// Every source-column value in value order with null first, including
    /// those absent from `rows`. Stable options remain selectable as other
    /// filters change.
    pub(crate) fn counts_all(&self, rows: &Selection) -> Result<Vec<DistinctValue>> {
        self.counts_with_zero(rows, true)
    }

    fn counts_with_zero(&self, rows: &Selection, include_zero: bool) -> Result<Vec<DistinctValue>> {
        let mut counts = [0u64; 256];
        match rows {
            Selection::All(_) => self
                .codes
                .iter()
                .for_each(|&code| counts[code as usize] += 1),
            Selection::Rows(rows) => rows
                .iter()
                .for_each(|&row| counts[self.codes[row as usize] as usize] += 1),
        }

        let formatter = format::formatter(self.values.as_ref())?;
        let mut values = Vec::with_capacity(self.values.len() + 1);
        if self.has_null && (include_zero || counts[NULL_CODE as usize] > 0) {
            values.push(DistinctValue {
                key: u64::from(NULL_CODE),
                label: None,
                count: counts[NULL_CODE as usize],
            });
        }
        for (code, &count) in counts.iter().enumerate().take(self.values.len()) {
            if include_zero || count > 0 {
                values.push(DistinctValue {
                    key: code as u64,
                    label: format::cell(self.values.as_ref(), &formatter, code),
                    count,
                });
            }
        }
        Ok(values)
    }

    /// Resolve matching keys for a list of displayed labels (`None` = null).
    pub(crate) fn keys_for_labels(&self, labels: &[Option<String>]) -> Result<Vec<u64>> {
        let target: HashSet<&Option<String>> = labels.iter().collect();
        let mut keys = Vec::new();
        if self.has_null && labels.iter().any(Option::is_none) {
            keys.push(u64::from(NULL_CODE));
        }
        let formatter = format::formatter(self.values.as_ref())?;
        for code in 0..self.values.len() {
            let label = format::cell(self.values.as_ref(), &formatter, code);
            if target.contains(&label) {
                keys.push(code as u64);
            }
        }
        Ok(keys)
    }

    fn fill_matches(
        &self,
        rows: RowSelection<'_>,
        incoming: Option<&[bool]>,
        output: &mut [bool],
        allowed: &[bool; 256],
    ) {
        match (rows, incoming) {
            (RowSelection::Range(range), None) => {
                for (i, &code) in self.codes[range].iter().enumerate() {
                    output[i] = allowed[code as usize];
                }
            }
            (RowSelection::Range(range), Some(mask)) => {
                for (i, &code) in self.codes[range].iter().enumerate() {
                    output[i] = allowed[code as usize] & mask[i];
                }
            }
            (RowSelection::Indices(indices), None) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = allowed[self.codes[row] as usize];
                }
            }
            (RowSelection::Indices(indices), Some(mask)) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = allowed[self.codes[row] as usize] & mask[i];
                }
            }
        }
    }

    fn predicate_from_allowed(self: &Arc<Self>, allowed: [bool; 256]) -> Predicate<'static> {
        let dictionary = Arc::clone(self);
        Predicate::new(Box::new(move |rows, incoming, output, _cancel| {
            dictionary.fill_matches(rows, incoming, output, &allowed);
            Ok(())
        }))
    }

    /// Prepare a row-based predicate for filtering by key codes.
    pub(crate) fn prepare_predicate(self: &Arc<Self>, keys: &[u64]) -> Predicate<'static> {
        let mut allowed = [false; 256];
        for &key in keys {
            if let Ok(key) = usize::try_from(key) {
                if let Some(slot) = allowed.get_mut(key) {
                    *slot = true;
                }
            }
        }
        self.predicate_from_allowed(allowed)
    }

    /// Prepare a row-based predicate for filtering by text values (`None` for null).
    pub(crate) fn prepare_text_predicate(
        self: &Arc<Self>,
        values: &[Option<String>],
        exclude: bool,
    ) -> Result<Predicate<'static>> {
        let keys = self.keys_for_labels(values)?;
        let mut allowed = [exclude; 256];
        for key in keys {
            if let Some(slot) = allowed.get_mut(key as usize) {
                *slot = !exclude;
            }
        }
        Ok(self.predicate_from_allowed(allowed))
    }
}

/// Build the codebook's `values` array in code order from the first row of
/// each value.
pub(super) fn value_rows(array: &dyn Array, first_row: &[usize], order: &[u8]) -> Option<ArrayRef> {
    let rows = UInt32Array::from(
        order
            .iter()
            .map(|&old| first_row[old as usize] as u32)
            .collect::<Vec<_>>(),
    );
    take(array, &rows, None).ok()
}

/// Distinct values with counts over `rows`: the codebook when the column has
/// one, streaming counts otherwise. `keep` bounds the streaming path's list; a
/// codebook never holds more than [`MAX_DISTINCT_VALUES`] values itself.
pub(crate) fn counted(
    dictionary: Option<&Arc<ValueDictionary>>,
    array: &ArrayRef,
    rows: &Selection,
    keep: usize,
) -> Result<ValueCounts> {
    match dictionary {
        Some(dictionary) => {
            let values = dictionary.counts(rows)?;
            Ok(ValueCounts {
                distinct: values.len() as u64,
                top: values,
            })
        }
        None => count_values(array, rows, keep),
    }
}

#[cfg(test)]
mod tests;
