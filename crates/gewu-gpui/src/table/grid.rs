//! The grid's interaction state and its rules: the selection, the requested scroll position,
//! the active drag and the pointer hover.
//!
//! Input arrives as a pointer position in canvas coordinates or as a keyboard motion and is
//! resolved against the committed [`Layout`], the geometry that was drawn. Methods change the
//! state and return the work the table must carry out with gpui (sorting, filtering,
//! measuring, showing a cell). Nothing here needs a window, so the rules are tested directly.
//!
//! Scroll positions are requests. The next layout resolves them against the measured column
//! widths and the viewport, and [`GridModel::commit`] takes back what it resolved.

use gpui_kit::{Along, Axis, Pixels, Point};

use super::{
    HEADER_HEIGHT, MIN_COLUMN,
    columns::TableColumns,
    layout::{HeaderPart, HorizontalScroll, Layout, Scrollbar},
    scroll::{Anchor, VerticalScroll},
    selection::{Cell, Selection},
};
use crate::actions::Motion;

/// A selection drag starts scrolling this far inside the body edge.
const EDGE_MARGIN: f32 = 8.;
/// The most a selection drag scrolls per frame, in points.
const EDGE_SCROLL_LIMIT: f32 = 64.;

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    /// Extending the selection. `edge` holds the pointer while it keeps the grid scrolling.
    Select {
        edge: Option<Point<Pixels>>,
    },
    Resize {
        column: usize,
        start_x: f32,
        start_width: f32,
    },
    Scrollbar {
        axis: Axis,
        grab: Pixels,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HeaderHover {
    Column(usize),
    Divider(usize),
}

impl From<HeaderPart> for HeaderHover {
    fn from(part: HeaderPart) -> Self {
        match part {
            HeaderPart::Divider(column) => Self::Divider(column),
            HeaderPart::Title(column) | HeaderPart::Funnel(column) => Self::Column(column),
        }
    }
}

/// Where a pointer position falls in the committed layout.
#[derive(Clone, Copy, Debug)]
pub(super) enum GridHit {
    Scrollbar(Axis, Scrollbar),
    Header(HeaderPart),
    Cell(Cell),
    Outside,
}

/// Work a pointer press asks of the table.
#[derive(Debug, PartialEq)]
pub(super) enum GridEffect {
    CycleSort { column: usize, multiple: bool },
    EditFilter(usize),
    Autofit(usize),
    ShowCell(Cell),
}

/// What a pointer motion with the button held did.
#[derive(Debug, PartialEq)]
pub(super) enum DragUpdate {
    /// The selection or a scrollbar followed the pointer.
    Moved,
    /// The pointer rests past the body edge: keep extending the selection every frame.
    EdgeScrolling,
    Resize {
        column: usize,
        width: f32,
    },
}

/// The geometry a drag resolves against: the committed layout, the column widths and the
/// scrollbar policy, to lay out the grid again after it scrolls.
pub(super) struct Geometry<'a> {
    pub layout: &'a Layout,
    pub columns: &'a TableColumns,
    pub auto_hide_scrollbars: bool,
}

pub(super) struct GridModel {
    selection: Option<Selection>,
    /// The requested vertical position; the committed layout holds the drawn one.
    scroll: VerticalScroll,
    /// The requested horizontal position, resolved by the next layout.
    horizontal: HorizontalScroll,
    drag: Drag,
    header_hover: Option<HeaderHover>,
    thumb_hover: Option<Axis>,
    /// The pointer over the body. The hovered row follows scrolling under a still pointer.
    body_pointer: Option<Point<Pixels>>,
}

impl GridModel {
    pub(super) fn new(scroll: VerticalScroll) -> Self {
        Self {
            selection: None,
            scroll,
            horizontal: HorizontalScroll::Offset(0.),
            drag: Drag::None,
            header_hover: None,
            thumb_hover: None,
            body_pointer: None,
        }
    }

    pub(super) fn selection(&self) -> Option<Selection> {
        self.selection
    }

    pub(super) fn scroll(&self) -> &VerticalScroll {
        &self.scroll
    }

    pub(super) fn horizontal(&self) -> HorizontalScroll {
        self.horizontal
    }

    pub(super) fn is_dragging(&self) -> bool {
        self.drag != Drag::None
    }

    /// The thumb drawn as active: the dragged one, else the hovered one.
    pub(super) fn active_thumb(&self) -> Option<Axis> {
        match self.drag {
            Drag::Scrollbar { axis, .. } => Some(axis),
            _ => self.thumb_hover,
        }
    }

    pub(super) fn header_hover(&self) -> Option<HeaderHover> {
        self.header_hover
    }

    /// The body cell under the pointer in `layout`.
    pub(super) fn hovered_cell(&self, layout: &Layout) -> Option<Cell> {
        let point = self.body_pointer?;
        layout.cell_at(point.x.as_f32(), point.y.as_f32())
    }

    /// Returns whether the selection changed.
    pub(super) fn select(&mut self, selection: Option<Selection>) -> bool {
        let changed = self.selection != selection;
        self.selection = selection;
        changed
    }

    /// Row or column coordinates changed: a drag or hover over the old grid ends.
    pub(super) fn reset_interaction(&mut self) {
        self.drag = Drag::None;
        self.header_hover = None;
        self.thumb_hover = None;
        self.body_pointer = None;
    }

    pub(super) fn set_rows(&mut self, rows: u64) {
        self.scroll.set_rows(rows);
    }

    pub(super) fn set_row_height(&mut self, height: f64) {
        self.scroll.set_row_height(height);
    }

    pub(super) fn scroll_to_row(&mut self, row: u64) {
        self.scroll.set_anchor(Anchor { row, offset: 0.0 });
    }

    /// Give a distant record context when navigating to it from elsewhere.
    pub(super) fn center_if_outside(&mut self, row: u64) {
        self.scroll.center_if_outside(row);
    }

    /// Accept a drawn frame's geometry: its viewport, and the horizontal position it resolved
    /// unless a newer request arrived after the frame was laid out.
    pub(super) fn commit(&mut self, layout: &Layout, requested: HorizontalScroll) {
        self.scroll.set_viewport(layout.vertical.viewport());
        if self.horizontal == requested {
            self.horizontal = HorizontalScroll::Offset(layout.scroll_x);
        }
    }

    /// Scroll the least to show `cell` entirely. The column stays the target while layout
    /// measures the columns around it.
    pub(super) fn reveal(&mut self, cell: Cell, columns: &TableColumns, layout: &Layout) {
        self.scroll.reveal(cell.row);
        self.horizontal = HorizontalScroll::Reveal {
            from: self.scroll_x(columns, layout),
            column: columns[cell.column].id,
        };
    }

    fn scroll_x(&self, columns: &TableColumns, layout: &Layout) -> f32 {
        self.horizontal
            .offset(columns, (layout.body_right() - layout.gutter).max(0.))
    }

    /// Hidden overlay tracks pass the pointer through to the cell underneath, except the
    /// hovered one.
    fn scrollbar_at(
        &self,
        point: Point<Pixels>,
        layout: &Layout,
        show_scrollbars: bool,
    ) -> Option<(Axis, Scrollbar)> {
        [Axis::Vertical, Axis::Horizontal]
            .into_iter()
            .find_map(|axis| {
                if !show_scrollbars && self.thumb_hover != Some(axis) {
                    return None;
                }
                let bar = layout.scrollbar(axis)?;
                bar.track.contains(&point).then_some((axis, bar))
            })
    }

    pub(super) fn hit(
        &self,
        point: Point<Pixels>,
        layout: &Layout,
        show_scrollbars: bool,
    ) -> GridHit {
        if let Some((axis, bar)) = self.scrollbar_at(point, layout, show_scrollbars) {
            return GridHit::Scrollbar(axis, bar);
        }
        let (x, y) = (point.x.as_f32(), point.y.as_f32());
        let part = (0.0..HEADER_HEIGHT)
            .contains(&y)
            .then(|| layout.header_part_at(x))
            .flatten();
        match (part, layout.cell_at(x, y)) {
            (Some(part), _) => GridHit::Header(part),
            (None, Some(cell)) => GridHit::Cell(cell),
            (None, None) => GridHit::Outside,
        }
    }

    pub(super) fn pointer_down(
        &mut self,
        point: Point<Pixels>,
        shift: bool,
        clicks: usize,
        geometry: &Geometry,
        show_scrollbars: bool,
    ) -> Option<GridEffect> {
        let layout = geometry.layout;
        match self.hit(point, layout, show_scrollbars) {
            GridHit::Scrollbar(axis, bar) => {
                let at = point.along(axis);
                let grab = if bar.thumb.contains(&point) {
                    at - bar.thumb.origin.along(axis)
                } else {
                    // A press on the track centers the thumb under the pointer.
                    let half = bar.thumb.size.along(axis) / 2.;
                    self.move_scrollbar(axis, &bar, at - half, layout);
                    half
                };
                self.drag = Drag::Scrollbar { axis, grab };
                None
            }
            GridHit::Header(HeaderPart::Divider(column)) if clicks == 2 => {
                Some(GridEffect::Autofit(column))
            }
            GridHit::Header(HeaderPart::Divider(column)) => {
                self.drag = Drag::Resize {
                    column,
                    start_x: point.x.as_f32(),
                    start_width: geometry.columns.width(column),
                };
                None
            }
            GridHit::Header(HeaderPart::Funnel(column)) => Some(GridEffect::EditFilter(column)),
            GridHit::Header(HeaderPart::Title(column)) => Some(GridEffect::CycleSort {
                column,
                multiple: shift,
            }),
            GridHit::Cell(cell) => {
                self.selection = Some(match (self.selection, shift) {
                    (Some(selection), true) => Selection {
                        anchor: selection.anchor,
                        head: cell,
                    },
                    _ => Selection::single(cell),
                });
                if clicks == 2 {
                    return Some(GridEffect::ShowCell(cell));
                }
                self.drag = Drag::Select { edge: None };
                None
            }
            GridHit::Outside => None,
        }
    }

    fn move_scrollbar(&mut self, axis: Axis, bar: &Scrollbar, start: Pixels, layout: &Layout) {
        let travel = bar.track.size.along(axis) - bar.thumb.size.along(axis);
        if travel <= Pixels::ZERO {
            return;
        }
        let fraction = ((start - bar.track.origin.along(axis)) / travel).clamp(0., 1.);
        match axis {
            Axis::Vertical => self.scroll.set_fraction(f64::from(fraction)),
            Axis::Horizontal => {
                self.horizontal = HorizontalScroll::Offset(fraction * layout.horizontal_overflow())
            }
        }
    }

    /// Pointer motion with no button held. Returns whether the hover changed what is drawn.
    pub(super) fn pointer_hover(
        &mut self,
        point: Point<Pixels>,
        layout: &Layout,
        show_scrollbars: bool,
    ) -> bool {
        let (x, y) = (point.x.as_f32(), point.y.as_f32());
        let header = (0.0..HEADER_HEIGHT)
            .contains(&y)
            .then(|| layout.header_part_at(x).map(HeaderHover::from))
            .flatten();
        let thumb = self
            .scrollbar_at(point, layout, show_scrollbars)
            .map(|(axis, _)| axis);
        let body = if thumb.is_none() {
            layout.cell_at(x, y)
        } else {
            None
        };
        let previous = self.hovered_cell(layout);
        self.body_pointer = body.map(|_| point);
        let changed = self.header_hover != header
            || self.thumb_hover != thumb
            || previous.map(|cell| cell.row) != body.map(|cell| cell.row);
        self.header_hover = header;
        self.thumb_hover = thumb;
        changed
    }

    /// The pointer left the grid. Returns whether a hover was cleared.
    pub(super) fn pointer_left(&mut self) -> bool {
        let hovering = self.header_hover.is_some()
            || self.thumb_hover.is_some()
            || self.body_pointer.is_some();
        self.header_hover = None;
        self.thumb_hover = None;
        self.body_pointer = None;
        hovering
    }

    /// Pointer motion with the button held. `None` when no drag is in progress.
    pub(super) fn pointer_drag(
        &mut self,
        point: Point<Pixels>,
        geometry: &Geometry,
    ) -> Option<DragUpdate> {
        match self.drag {
            Drag::None => None,
            Drag::Scrollbar { axis, grab } => {
                if let Some(bar) = geometry.layout.scrollbar(axis) {
                    self.move_scrollbar(axis, &bar, point.along(axis) - grab, geometry.layout);
                }
                Some(DragUpdate::Moved)
            }
            Drag::Resize {
                column,
                start_x,
                start_width,
            } => Some(DragUpdate::Resize {
                column,
                width: (start_width + point.x.as_f32() - start_x).max(MIN_COLUMN),
            }),
            Drag::Select { .. } => Some(self.extend_selection(point, geometry)),
        }
    }

    /// Continue a selection drag resting past the body edge, one frame later. `None` once the
    /// drag ended or stopped scrolling.
    pub(super) fn edge_scroll(&mut self, geometry: &Geometry) -> Option<DragUpdate> {
        let Drag::Select { edge: Some(point) } = self.drag else {
            return None;
        };
        Some(self.extend_selection(point, geometry))
    }

    fn extend_selection(&mut self, point: Point<Pixels>, geometry: &Geometry) -> DragUpdate {
        let layout = geometry.layout;
        let (x, y) = (point.x.as_f32(), point.y.as_f32());
        let edge_delta = |coord: f32, min: f32, max: f32| {
            let margin = EDGE_MARGIN.min(((max - min) / 2.).max(0.));
            if coord < min + margin {
                (coord - min - margin).max(-EDGE_SCROLL_LIMIT)
            } else if coord > max - margin {
                (coord - max + margin).min(EDGE_SCROLL_LIMIT)
            } else {
                0.
            }
        };
        let dx = edge_delta(x, layout.gutter, layout.body_right());
        let dy = edge_delta(y, HEADER_HEIGHT, layout.body_bottom());
        let before = (
            self.scroll.anchor(),
            self.scroll_x(geometry.columns, layout),
        );
        self.scroll.scroll_by(f64::from(dy));
        // Lay the grid out again to find the cell under the pointer after it scrolled. The
        // layout also clamps the horizontal request to the content.
        let scrolled = Layout::new(
            layout.bounds,
            layout.gutter,
            geometry.columns,
            &self.scroll,
            HorizontalScroll::Offset((before.1 + dx).max(0.)),
            geometry.auto_hide_scrollbars,
        );
        self.horizontal = HorizontalScroll::Offset(scrolled.scroll_x);
        if let (Some(selection), Some(head)) = (self.selection, scrolled.drag_cell_at(x, y)) {
            self.selection = Some(Selection { head, ..selection });
        }
        let moved = before != (self.scroll.anchor(), scrolled.scroll_x);
        self.drag = Drag::Select {
            edge: moved.then_some(point),
        };
        if moved {
            DragUpdate::EdgeScrolling
        } else {
            DragUpdate::Moved
        }
    }

    pub(super) fn pointer_up(&mut self) {
        self.drag = Drag::None;
    }

    /// Scroll by a wheel or trackpad delta in points. Shift turns a vertical wheel horizontal.
    pub(super) fn wheel(
        &mut self,
        mut dx: f64,
        mut dy: f64,
        shift: bool,
        columns: &TableColumns,
        layout: &Layout,
    ) {
        if shift && dx == 0.0 {
            (dx, dy) = (dy, 0.0);
        }
        self.scroll.scroll_by(-dy);
        if dx != 0. {
            self.horizontal =
                HorizontalScroll::Offset((self.scroll_x(columns, layout) - dx as f32).max(0.));
        }
    }

    /// Move the selection head and reveal it. Returns whether the grid scrolled.
    pub(super) fn navigate(
        &mut self,
        motion: Motion,
        extend: bool,
        rows: u64,
        columns: &TableColumns,
        layout: &Layout,
    ) -> bool {
        if rows == 0 || columns.is_empty() {
            return false;
        }
        let current = self
            .selection
            .unwrap_or(Selection::single(Cell { row: 0, column: 0 }));
        let page = self.scroll.page_rows() as i64;
        let far = i64::MAX / 2;
        let (dr, dc) = match motion {
            Motion::Up => (-1, 0),
            Motion::Down => (1, 0),
            Motion::Left => (0, -1),
            Motion::Right => (0, 1),
            Motion::FirstRow => (-far, 0),
            Motion::LastRow => (far, 0),
            Motion::FirstColumn => (0, -far),
            Motion::LastColumn => (0, far),
            Motion::PageUp => (-page, 0),
            Motion::PageDown => (page, 0),
        };
        let next = current.moved(dr, dc, extend, rows, columns.len());
        self.selection = Some(next);
        let before = (self.scroll.anchor(), self.scroll_x(columns, layout));
        self.reveal(next.head, columns, layout);
        before != (self.scroll.anchor(), self.scroll_x(columns, layout))
    }
}

#[cfg(test)]
#[path = "grid_tests.rs"]
mod tests;
