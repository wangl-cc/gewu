//! Filter membership as packed selection bitmaps.
//!
//! A filtered view keeps one bitmap per filter condition over all source rows, plus their
//! intersection. Row IDs are materialized once from the intersection for display order and
//! row-addressed consumers; facets and selection-aware scans combine bitmaps instead of
//! re-running predicates.

use std::sync::Arc;

use arrow::buffer::BooleanBuffer;

use crate::ColumnId;

/// Bitmaps of the filter conditions applied to one view. Cheap to clone.
#[derive(Clone, Default)]
pub(crate) struct FilterSelection {
    /// One bitmap per filtered column, each over every source row.
    conditions: Arc<[(ColumnId, BooleanBuffer)]>,
    /// Intersection of `conditions`. `None` selects every row.
    mask: Option<BooleanBuffer>,
}

impl FilterSelection {
    pub(crate) fn new(conditions: Vec<(ColumnId, BooleanBuffer)>) -> Self {
        let mask = intersect(conditions.iter().map(|(_, bits)| bits));
        Self {
            conditions: conditions.into(),
            mask,
        }
    }

    /// Rows passing every condition, or `None` for all rows.
    pub(crate) fn mask(&self) -> Option<&BooleanBuffer> {
        self.mask.as_ref()
    }

    pub(crate) fn constrains(&self, column: ColumnId) -> bool {
        self.conditions.iter().any(|(c, _)| *c == column)
    }

    /// The selection without `column`'s condition: the rows a filter editor
    /// for `column` chooses from.
    pub(crate) fn without(&self, column: ColumnId) -> Self {
        Self::new(
            self.conditions
                .iter()
                .filter(|(c, _)| *c != column)
                .cloned()
                .collect(),
        )
    }

    /// Intersection used by a column editor, without constructing a row-addressable view.
    pub(super) fn facet_mask(&self, column: ColumnId) -> Option<BooleanBuffer> {
        if self.constrains(column) {
            intersect(
                self.conditions
                    .iter()
                    .filter(|(c, _)| *c != column)
                    .map(|(_, mask)| mask),
            )
        } else {
            self.mask.clone()
        }
    }

    /// Source row IDs of the selected rows, in source order.
    pub(crate) fn row_ids(mask: &BooleanBuffer) -> Vec<u32> {
        let mut rows = Vec::with_capacity(mask.count_set_bits());
        rows.extend(mask.set_indices_u32());
        rows
    }
}

fn intersect<'a>(mut bitmaps: impl Iterator<Item = &'a BooleanBuffer>) -> Option<BooleanBuffer> {
    let first = bitmaps.next()?.clone();
    Some(bitmaps.fold(first, |acc, bits| &acc & bits))
}
