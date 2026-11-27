//! A column keeps its presentation when it moves or leaves the visible order. Visible
//! positions are [`VisibleCol`]s; this module is the one place that turns them into indices.
use std::{collections::HashSet, ops::Index};

use gewu_core::{ColumnId, ColumnInfo};
use gpui_kit::SharedString;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ColumnWidth {
    Unmeasured,
    Measured(f32),
    Manual(f32),
}

/// The width assumed for a column until a drawn frame measures it.
const UNMEASURED_WIDTH: f32 = 110.;

impl ColumnWidth {
    fn points(self) -> f32 {
        match self {
            Self::Unmeasured => UNMEASURED_WIDTH,
            Self::Measured(points) | Self::Manual(points) => points,
        }
    }
}

pub(crate) struct GridColumn {
    pub(super) info: ColumnInfo,
    /// The column's name as shared text, cloned into every drawn header without copying.
    pub(super) label: SharedString,
    pub(super) width: ColumnWidth,
}

/// Position in the visible order, valid until columns are hidden or reordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct VisibleCol(usize);

impl VisibleCol {
    pub(super) const FIRST: Self = Self(0);

    /// The zero-based position, for accessibility indices and displayed ordinals.
    pub(super) fn index(self) -> usize {
        self.0
    }

    /// The position `by` columns away, clamped to the `count` visible columns.
    pub(super) fn moved(self, by: i64, count: usize) -> Self {
        let index = (self.0 as i128 + i128::from(by)).clamp(0, count.saturating_sub(1) as i128);
        Self(index as usize)
    }

    /// The position `by` columns away, if one of the `count` visible columns is there.
    pub(super) fn step(self, by: isize, count: usize) -> Option<Self> {
        self.0
            .checked_add_signed(by)
            .filter(|&index| index < count)
            .map(Self)
    }

    /// The column after this one, for ranges of columns.
    pub(super) fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

pub(crate) struct TableColumns {
    schema: Vec<GridColumn>,
    /// Schema slots of visible columns, in their current display order.
    order: Vec<usize>,
}

impl TableColumns {
    pub(super) fn new(columns: Vec<ColumnInfo>) -> Self {
        Self {
            order: (0..columns.len()).collect(),
            schema: columns
                .into_iter()
                .map(|info| GridColumn {
                    label: info.name.clone().into(),
                    info,
                    width: ColumnWidth::Unmeasured,
                })
                .collect(),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.order.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Visible positions in display order.
    pub(super) fn positions(&self) -> impl ExactSizeIterator<Item = VisibleCol> + use<> {
        (0..self.order.len()).map(VisibleCol)
    }

    pub(super) fn last(&self) -> Option<VisibleCol> {
        self.order.len().checked_sub(1).map(VisibleCol)
    }

    pub(super) fn entry(&self, column: VisibleCol) -> &GridColumn {
        &self.schema[self.order[column.0]]
    }

    pub(super) fn set_width(&mut self, column: VisibleCol, width: ColumnWidth) {
        self.schema[self.order[column.0]].width = width;
    }

    pub(super) fn get(&self, column: VisibleCol) -> Option<&ColumnInfo> {
        self.order
            .get(column.0)
            .map(|&slot| &self.schema[slot].info)
    }

    /// The stable identities of the visible columns, in display order.
    pub(super) fn ids(&self) -> Vec<ColumnId> {
        self.iter().map(|column| column.id).collect()
    }

    pub(super) fn iter(&self) -> impl ExactSizeIterator<Item = &ColumnInfo> + Clone {
        self.order.iter().map(|&slot| &self.schema[slot].info)
    }

    /// Visible columns in display order, then hidden columns in source order.
    pub(super) fn visible_then_hidden(
        &self,
    ) -> impl Iterator<Item = (Option<VisibleCol>, &ColumnInfo)> {
        let mut visible = vec![false; self.schema.len()];
        for &slot in &self.order {
            visible[slot] = true;
        }
        self.iter()
            .enumerate()
            .map(|(index, column)| (Some(VisibleCol(index)), column))
            .chain(
                self.schema
                    .iter()
                    .enumerate()
                    .filter_map(move |(slot, column)| {
                        (!visible[slot]).then_some((None, &column.info))
                    }),
            )
    }

    pub(super) fn position(&self, id: ColumnId) -> Option<VisibleCol> {
        self.iter()
            .position(|column| column.id == id)
            .map(VisibleCol)
    }

    pub(super) fn width(&self, column: VisibleCol) -> f32 {
        self.entry(column).width.points()
    }

    pub(super) fn invalidate_measurements(&mut self) {
        for column in &mut self.schema {
            if matches!(column.width, ColumnWidth::Measured(_)) {
                column.width = ColumnWidth::Unmeasured;
            }
        }
    }

    pub(super) fn set_visible(&mut self, mut requested: HashSet<ColumnId>, visible: bool) -> bool {
        let previous = self.order.len();
        if visible {
            for &slot in &self.order {
                requested.remove(&self.schema[slot].info.id);
            }
            self.order.extend(
                self.schema.iter().enumerate().filter_map(|(slot, column)| {
                    requested.contains(&column.info.id).then_some(slot)
                }),
            );
        } else {
            self.order
                .retain(|&slot| !requested.contains(&self.schema[slot].info.id));
        }
        self.order.len() != previous
    }

    pub(super) fn move_to(&mut self, from: VisibleCol, to: VisibleCol) -> bool {
        if from == to || from.0 >= self.len() || to.0 >= self.len() {
            return false;
        }
        let slot = self.order.remove(from.0);
        self.order.insert(to.0, slot);
        true
    }

    pub(super) fn is_in_source_order(&self) -> bool {
        self.order.is_sorted()
    }

    /// Restore the relative source order of the visible columns.
    pub(super) fn restore_source_order(&mut self) -> bool {
        if self.is_in_source_order() {
            return false;
        }
        self.order.sort_unstable();
        true
    }
}

impl Index<VisibleCol> for TableColumns {
    type Output = ColumnInfo;

    fn index(&self, column: VisibleCol) -> &Self::Output {
        &self.entry(column).info
    }
}

#[cfg(test)]
#[path = "columns_fixture.rs"]
pub(super) mod fixture;
