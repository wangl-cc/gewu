//! Distinct values of a column as a compact `u8` code space.
//!
//! A [`ValueDictionary`] gives every source row a code naming its distinct
//! value: codes `0..=254` name the non-null values in value order, and
//! [`NULL_CODE`] names null. Everything about how a physical Arrow array
//! becomes that dictionary lives here: native dictionaries reuse their keys and
//! values, primitive arrays scan natively, and only text, binary and nested
//! arrays fall back to Arrow's row format. Other modules consume the result
//! through the methods below; they never see codes, values or encoding
//! strategy.
//!
//! The dictionary is a pure function of the column data and is cached by
//! [`crate::Column`]. Columns with more than [`MAX_DISTINCT_VALUES`] non-null
//! values are not encoded; callers fall back to streaming counts, which this
//! module also provides through [`counted`].

use std::sync::Arc;

use arrow::array::{Array, ArrayRef};

use crate::{predicate::Predicate, scope::Selection, selection::RowSelection};

mod encode;
mod primitive;
pub(crate) mod streaming;

pub(crate) use primitive::normalized_bits;

/// Most non-null values a column may have and still be encoded. Non-null
/// values use codes `0..=254`; [`NULL_CODE`] is reserved for null, so one
/// `u8` code space covers 255 values plus null. Columns with more values are
/// counted without an encoding and use streaming counts.
pub(crate) const MAX_DISTINCT_VALUES: usize = 255;

/// Code reserved for null. Non-null values use codes `0..=254`.
pub(crate) const NULL_CODE: u8 = 255;

/// Rows converted to row format at a time, to bound memory.
const CHUNK: usize = 64 * 1024;

pub(crate) struct ValueDictionary {
    /// Code of each source row; [`NULL_CODE`] for null.
    codes: Vec<u8>,
    /// Non-null values in code order.
    values: ArrayRef,
    /// Whether the source column contains at least one null.
    has_null: bool,
}

impl ValueDictionary {
    #[inline]
    pub(crate) fn values(&self) -> &ArrayRef {
        &self.values
    }

    #[inline]
    pub(crate) fn has_null(&self) -> bool {
        self.has_null
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    #[cfg(test)]
    pub(crate) fn n_values(&self) -> usize {
        self.values.len()
    }

    #[cfg(test)]
    pub(crate) fn code_at(&self, row: usize) -> u8 {
        self.codes[row]
    }

    /// Tally occurrences of each code over `rows`.
    /// `counts[0..self.values.len()]` correspond to non-null values, and
    /// `counts[NULL_CODE as usize]` is the count of null rows.
    pub(crate) fn tally(&self, rows: &Selection) -> [u64; 256] {
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
        counts
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

    pub(crate) fn predicate_from_allowed(
        self: &Arc<Self>,
        allowed: [bool; 256],
    ) -> Predicate<'static> {
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
}

#[cfg(test)]
mod tests;
