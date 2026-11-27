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

impl ColumnWidth {
    fn points(self) -> f32 {
        match self {
            Self::Unmeasured => 110.,
            Self::Measured(points) | Self::Manual(points) => points,
        }
    }
}

pub(crate) struct GridColumn {
    pub info: ColumnInfo,
    pub label: SharedString,
    pub width: ColumnWidth,
}

/// Position in the visible order, valid until columns are hidden or reordered.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct VisibleCol(usize);

impl VisibleCol {
    pub fn index(self) -> usize {
        self.0
    }
}

pub(crate) struct TableColumns {
    schema: Vec<GridColumn>,
    /// Schema slots of visible columns, in their current display order.
    order: Vec<usize>,
}

impl TableColumns {
    pub fn new(columns: Vec<ColumnInfo>) -> Self {
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

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn entry(&self, index: usize) -> &GridColumn {
        &self.schema[self.order[index]]
    }

    pub fn set_width(&mut self, index: usize, width: ColumnWidth) {
        self.schema[self.order[index]].width = width;
    }

    pub fn get(&self, index: usize) -> Option<&ColumnInfo> {
        self.order.get(index).map(|&slot| &self.schema[slot].info)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &ColumnInfo> + Clone {
        self.order.iter().map(|&slot| &self.schema[slot].info)
    }

    /// Visible columns in display order, then hidden columns in source order.
    pub fn visible_then_hidden(&self) -> impl Iterator<Item = (Option<VisibleCol>, &ColumnInfo)> {
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

    pub fn position(&self, id: ColumnId) -> Option<VisibleCol> {
        self.iter()
            .position(|column| column.id == id)
            .map(VisibleCol)
    }

    pub fn width(&self, index: usize) -> f32 {
        self.entry(index).width.points()
    }

    pub fn invalidate_measurements(&mut self) {
        for column in &mut self.schema {
            if matches!(column.width, ColumnWidth::Measured(_)) {
                column.width = ColumnWidth::Unmeasured;
            }
        }
    }

    pub fn set_visible(&mut self, mut requested: HashSet<ColumnId>, visible: bool) -> bool {
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

    pub fn move_to(&mut self, from: usize, to: usize) -> bool {
        if from == to || from >= self.len() || to >= self.len() {
            return false;
        }
        let slot = self.order.remove(from);
        self.order.insert(to, slot);
        true
    }

    pub fn is_in_source_order(&self) -> bool {
        self.order.is_sorted()
    }

    /// Restore the relative source order of the visible columns.
    pub fn restore_source_order(&mut self) -> bool {
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
