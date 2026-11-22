use std::sync::Arc;

use super::*;
use crate::row_ids::RowIds;

pub(crate) struct Fixture {
    column: Column,
    rows: RowIds,
    mask: Option<BooleanBuffer>,
}

impl Fixture {
    pub(crate) fn all(array: &ArrayRef) -> Self {
        Self {
            column: Column::new("test", array.clone()).unwrap(),
            rows: RowIds::All(array.len()),
            mask: None,
        }
    }

    pub(crate) fn new(array: &ArrayRef, rows: Vec<u32>) -> Self {
        Self {
            column: Column::new("test", array.clone()).unwrap(),
            rows: RowIds::Selected(Arc::from(rows)),
            mask: None,
        }
    }

    /// A filtered view of `rows` (ascending), carrying its filter bitmap.
    pub(crate) fn masked(array: &ArrayRef, rows: Vec<u32>) -> Self {
        let mask = BooleanBuffer::collect_bool(array.len(), |row| {
            rows.binary_search(&(row as u32)).is_ok()
        });
        Self {
            mask: Some(mask),
            ..Self::new(array, rows)
        }
    }

    pub(crate) fn view(&self) -> ColView<'_> {
        ColView::new(&self.column, self.rows.as_slice()).with_mask(self.mask.as_ref())
    }
}
