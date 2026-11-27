//! A column keeps its presentation when it moves or leaves the visible order.
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
    pub(super) label: SharedString,
    pub(super) width: ColumnWidth,
}

/// Position in the visible order, valid until columns are hidden or reordered.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct VisibleCol(usize);

impl VisibleCol {
    pub(super) fn index(self) -> usize {
        self.0
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

    pub(super) fn entry(&self, index: usize) -> &GridColumn {
        &self.schema[self.order[index]]
    }

    pub(super) fn set_width(&mut self, index: usize, width: ColumnWidth) {
        self.schema[self.order[index]].width = width;
    }

    pub(super) fn get(&self, index: usize) -> Option<&ColumnInfo> {
        self.order.get(index).map(|&slot| &self.schema[slot].info)
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

    pub(super) fn width(&self, index: usize) -> f32 {
        self.entry(index).width.points()
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

    pub(super) fn move_to(&mut self, from: usize, to: usize) -> bool {
        if from == to || from >= self.len() || to >= self.len() {
            return false;
        }
        let slot = self.order.remove(from);
        self.order.insert(to, slot);
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

impl Index<usize> for TableColumns {
    type Output = ColumnInfo;

    fn index(&self, index: usize) -> &Self::Output {
        &self.entry(index).info
    }
}

#[cfg(test)]
#[path = "columns_fixture.rs"]
pub(super) mod fixture;
