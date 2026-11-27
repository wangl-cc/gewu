//! The grid's geometry for one frame: viewport, scrollbar tracks, visible columns and header
//! parts. Drawing and hit testing share the committed layout; widths measured while laying
//! out a frame are committed after it is drawn.
use std::collections::{BTreeMap, btree_map::Entry};

use super::*;
use crate::table::columns::{TableColumns, VisibleCol};

/// Keep a reveal anchored to its column while layout measures the surrounding widths.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum HorizontalScroll {
    Offset(f32),
    Reveal {
        column: gewu_core::ColumnId,
        from: f32,
    },
}

impl HorizontalScroll {
    pub(super) fn offset(self, columns: &TableColumns, viewport: f32) -> f32 {
        self.offset_with_widths(columns, |index| columns.width(index), viewport)
    }

    fn offset_with_widths(
        self,
        columns: &TableColumns,
        width: impl Fn(VisibleCol) -> f32,
        viewport: f32,
    ) -> f32 {
        match self {
            Self::Offset(offset) => offset,
            Self::Reveal { column, from } => {
                let Some(target) = columns.position(column) else {
                    return from;
                };
                let left: f32 = columns
                    .positions()
                    .take_while(|&column| column < target)
                    .map(&width)
                    .sum();
                let right_aligned = left + width(target) - viewport;
                // Show the entire column when it fits, otherwise keep as much of it in view as
                // possible.
                from.clamp(left.min(right_aligned), left.max(right_aligned))
            }
        }
    }
}

/// A visible column as laid out for one frame, in canvas coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LaidColumn {
    pub(super) column: VisibleCol,
    pub(super) x: f32,
    pub(super) width: f32,
}

impl LaidColumn {
    pub(super) fn right(&self) -> f32 {
        self.x + self.width
    }
}

#[derive(PartialEq)]
pub(super) struct Layout {
    pub(super) bounds: Bounds<Pixels>,
    /// Canvas available to the header, gutter, and data after reserving permanent tracks.
    pub(super) viewport: Size<Pixels>,
    pub(super) gutter: f32,
    /// The visible columns that intersect the body, left to right.
    pub(super) columns: Vec<LaidColumn>,
    pub(super) vertical: VerticalScroll,
    pub(super) scroll_x: f32,
    pub(super) content_width: f32,
}

impl Layout {
    pub(super) fn new(
        bounds: Bounds<Pixels>,
        gutter: f32,
        grid_columns: &TableColumns,
        scroll: &VerticalScroll,
        horizontal: HorizontalScroll,
        auto_hide_scrollbars: bool,
    ) -> Self {
        Self::with_widths(
            bounds,
            gutter,
            grid_columns,
            |column| grid_columns.width(column),
            scroll,
            horizontal,
            auto_hide_scrollbars,
        )
    }

    fn with_widths(
        bounds: Bounds<Pixels>,
        gutter: f32,
        grid_columns: &TableColumns,
        width: impl Fn(VisibleCol) -> f32,
        scroll: &VerticalScroll,
        horizontal: HorizontalScroll,
        auto_hide_scrollbars: bool,
    ) -> Self {
        let content_width = grid_columns.positions().map(&width).sum::<f32>();
        let viewport = viewport_size(
            bounds.size,
            gutter,
            content_width,
            scroll.rows() as f64 * scroll.row_height(),
            auto_hide_scrollbars,
        );
        let body_width = (viewport.width.as_f32() - gutter).max(0.);
        let mut vertical = scroll.clone();
        vertical.set_viewport(f64::from(
            (viewport.height.as_f32() - HEADER_HEIGHT).max(0.),
        ));
        let scroll_x = horizontal
            .offset_with_widths(grid_columns, &width, body_width)
            .clamp(0., (content_width - body_width).max(0.));
        let mut columns = Vec::new();
        let mut x = 0.;
        for column in grid_columns.positions() {
            let width = width(column);
            if x >= scroll_x + body_width {
                break;
            }
            if x + width > scroll_x {
                columns.push(LaidColumn {
                    column,
                    x: gutter + x - scroll_x,
                    width,
                });
            }
            x += width;
        }
        Self {
            bounds,
            viewport,
            gutter,
            columns,
            vertical,
            scroll_x,
            content_width,
        }
    }

    pub(super) fn body_right(&self) -> f32 {
        self.viewport.width.as_f32()
    }

    pub(super) fn body_bottom(&self) -> f32 {
        self.viewport.height.as_f32()
    }

    pub(super) fn horizontal_overflow(&self) -> f32 {
        (self.content_width - (self.body_right() - self.gutter).max(0.)).max(0.)
    }

    /// Tracks hug the outer grid edges. The vertical track owns their shared corner.
    pub(super) fn scrollbar(&self, axis: Axis) -> Option<Scrollbar> {
        let outer = self.bounds.size;
        let width = (outer.width.as_f32() - self.gutter).max(0.);
        let height = (outer.height.as_f32() - HEADER_HEIGHT).max(0.);
        let vertical = self.vertical.max_position_rows() > 0.;
        if width == 0. || height == 0. {
            return None;
        }
        let (track, visible, fraction) = match axis {
            Axis::Vertical if vertical => {
                let w = SCROLLBAR.min(width);
                let content = self.vertical.rows() as f64 * self.vertical.row_height();
                (
                    Bounds::new(
                        point(outer.width - px(w), px(HEADER_HEIGHT)),
                        size_(w, height),
                    ),
                    (self.vertical.viewport() / content) as f32,
                    self.vertical.fraction() as f32,
                )
            }
            Axis::Horizontal if self.horizontal_overflow() > 0. => {
                let w = if vertical {
                    (width - SCROLLBAR).max(0.)
                } else {
                    width
                };
                let h = SCROLLBAR.min(height);
                (
                    Bounds::new(point(px(self.gutter), outer.height - px(h)), size_(w, h)),
                    (self.body_right() - self.gutter).max(0.) / self.content_width,
                    self.scroll_x / self.horizontal_overflow(),
                )
            }
            _ => return None,
        };
        let length = track.size.along(axis);
        let thumb_len = (length * visible).clamp(px(MIN_THUMB).min(length), length);
        let thumb = Bounds::new(
            track
                .origin
                .apply_along(axis, |v| v + (length - thumb_len) * fraction),
            track.size.apply_along(axis, |_| thumb_len),
        );
        Some(Scrollbar { track, thumb })
    }

    /// Clip partially visible cells to the same gutter and viewport as the painted body.
    pub(super) fn clipped_columns(&self) -> impl Iterator<Item = LaidColumn> + '_ {
        self.columns.iter().filter_map(|laid| {
            let left = laid.x.max(self.gutter);
            let right = laid.right().min(self.body_right());
            (right > left).then_some(LaidColumn {
                column: laid.column,
                x: left,
                width: right - left,
            })
        })
    }

    /// The layout of a visible column, if it intersects the body.
    pub(super) fn laid(&self, column: VisibleCol) -> Option<&LaidColumn> {
        self.columns.iter().find(|laid| laid.column == column)
    }

    /// Horizontal edges of the cells in the selection that were laid out for this frame.
    /// Keep partial edge cells at their full width; the paint mask clips their outlines.
    pub(super) fn column_edges(&self, columns: Range<VisibleCol>) -> Option<(f32, f32)> {
        let mut overlap = self
            .columns
            .iter()
            .filter(|laid| columns.contains(&laid.column));
        let first = overlap.next()?;
        let last = overlap.next_back().unwrap_or(first);
        Some((first.x, last.right()))
    }

    /// The left edge of the funnel glyph in the header of the column ending at `right`.
    pub(super) fn funnel_left(right: f32) -> f32 {
        right - PAD + 1. - FUNNEL_WIDTH
    }

    /// The divider band around the right edge of a visible column, clipped to the body.
    pub(super) fn divider_band(&self, column: VisibleCol) -> Option<(f32, f32)> {
        let edge = self.laid(column)?.right();
        let start = (edge - RESIZE_SLOP).max(self.gutter);
        let end = (edge + RESIZE_SLOP).min(self.body_right());
        (end > start).then_some((start, end))
    }

    /// The header part at `x`, for drawing and pointer input alike.
    pub(super) fn header_part_at(&self, x: f32) -> Option<HeaderPart> {
        if !(self.gutter..self.body_right()).contains(&x) {
            return None;
        }
        if let Some(laid) = self
            .columns
            .iter()
            .find(|laid| (x - laid.right()).abs() <= RESIZE_SLOP)
        {
            return Some(HeaderPart::Divider(laid.column));
        }
        let laid = self
            .columns
            .iter()
            .find(|laid| (laid.x..laid.right()).contains(&x))?;
        Some(if x >= Self::funnel_left(laid.right()) - FUNNEL_SLOP {
            HeaderPart::Funnel(laid.column)
        } else {
            HeaderPart::Title(laid.column)
        })
    }

    /// The left edge of a visible column.
    pub(super) fn column_left(&self, column: VisibleCol) -> Option<f32> {
        self.laid(column).map(|laid| laid.x)
    }

    pub(super) fn column_at(&self, x: f32) -> Option<VisibleCol> {
        if !(self.gutter..self.body_right()).contains(&x) {
            return None;
        }
        self.columns
            .iter()
            .find(|laid| (laid.x..laid.right()).contains(&x))
            .map(|laid| laid.column)
    }

    pub(super) fn cell_at(&self, x: f32, y: f32) -> Option<Cell> {
        if !(HEADER_HEIGHT..self.body_bottom()).contains(&y) {
            return None;
        }
        Some(Cell {
            column: self.column_at(x)?,
            row: gewu_core::ViewRow(self.vertical.row_at(f64::from(y - HEADER_HEIGHT))?),
        })
    }

    /// A drag past the painted cells extends to the nearest visible cell.
    pub(super) fn drag_cell_at(&self, x: f32, y: f32) -> Option<Cell> {
        let right = self.body_right().min(self.columns.last()?.right());
        let rows = self.vertical.visible_rows();
        let bottom = self
            .body_bottom()
            .min(HEADER_HEIGHT + self.vertical.row_top(rows.end) as f32);
        if rows.is_empty() || right <= self.gutter || bottom <= HEADER_HEIGHT {
            return None;
        }
        self.cell_at(
            x.clamp(self.gutter, right.next_down()),
            y.clamp(HEADER_HEIGHT, bottom.next_down()),
        )
    }

    fn highlight_blocks(&self) -> Range<u64> {
        let rows = self.vertical.visible_rows();
        if rows.is_empty() {
            return 0..0;
        }
        let block = gewu_app::find::HIGHLIGHT_BLOCK_ROWS;
        rows.start / block..rows.end.div_ceil(block)
    }

    fn visible_columns(&self) -> Option<(VisibleCol, VisibleCol)> {
        Some((self.columns.first()?.column, self.columns.last()?.column))
    }
}

/// Always-visible tracks take space only where the data overflows. Either track
/// can reduce the other axis enough to require a second track.
fn viewport_size(
    outer: Size<Pixels>,
    gutter: f32,
    content_width: f32,
    content_height: f64,
    auto_hide_scrollbars: bool,
) -> Size<Pixels> {
    let width = (outer.width.as_f32() - gutter).max(0.);
    let height = (outer.height.as_f32() - HEADER_HEIGHT).max(0.);
    if auto_hide_scrollbars || width == 0. || height == 0. {
        return outer;
    }
    let wide = content_width > width;
    let tall = content_height > f64::from(height);
    let vertical = tall || (wide && content_height > f64::from((height - SCROLLBAR).max(0.)));
    let horizontal = wide || (tall && content_width > (width - SCROLLBAR).max(0.));
    outer
        - size_(
            if vertical { SCROLLBAR.min(width) } else { 0. },
            if horizontal {
                SCROLLBAR.min(height)
            } else {
                0.
            },
        )
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Scrollbar {
    pub track: Bounds<Pixels>,
    pub thumb: Bounds<Pixels>,
}

/// A part of a visible column's header under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HeaderPart {
    /// The resize target around the column's right edge.
    Divider(VisibleCol),
    /// The filter funnel near the right edge, and the slop around it up to the divider.
    Funnel(VisibleCol),
    /// The rest of the header, which sorts the column.
    Title(VisibleCol),
}

impl HeaderPart {
    pub(super) fn column(self) -> VisibleCol {
        match self {
            Self::Divider(column) | Self::Funnel(column) | Self::Title(column) => column,
        }
    }
}

/// The funnel glyph's width. It ends one point into the header's right padding.
pub(super) const FUNNEL_WIDTH: f32 = 10.;
/// How far left of the glyph a press still opens the filter.
const FUNNEL_SLOP: f32 = 5.;

struct MeasuredColumn {
    id: gewu_core::ColumnId,
    width: f32,
}

/// A layout for this frame and the measurements it discovered. Prepaint can fill the
/// text cache, but changes to width, scroll and Find are committed after drawing.
pub(super) struct GridFrameLayout {
    pub(super) layout: Rc<Layout>,
    measured: BTreeMap<VisibleCol, MeasuredColumn>,
    font_size: f32,
    horizontal: HorizontalScroll,
}

pub(super) struct FindViewportChanged;

impl EventEmitter<FindViewportChanged> for TableView {}

impl TableView {
    pub(super) fn prepare_grid_layout(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &Window,
        cx: &App,
    ) -> GridFrameLayout {
        let auto_hide = cx.should_auto_hide_scrollbars();
        let mut layout = self.compute_layout(bounds, auto_hide);
        let mut measured = BTreeMap::<VisibleCol, MeasuredColumn>::new();
        loop {
            let mut changed = false;
            for laid in &layout.columns {
                let column = laid.column;
                let col = self.data.columns.entry(column);
                if col.width == ColumnWidth::Unmeasured
                    && let Entry::Vacant(entry) = measured.entry(column)
                {
                    let id = col.info.id;
                    let width = self.initial_width(column, window);
                    entry.insert(MeasuredColumn { id, width });
                    changed = true;
                }
            }
            if !changed {
                break;
            }
            layout = Layout::with_widths(
                bounds,
                layout.gutter,
                &self.data.columns,
                |column| {
                    measured.get(&column).map_or_else(
                        || self.data.columns.width(column),
                        |measured| measured.width,
                    )
                },
                self.grid.scroll(),
                self.grid.horizontal(),
                auto_hide,
            );
        }
        GridFrameLayout {
            layout: Rc::new(layout),
            measured,
            font_size: self.prefs.font_size,
            horizontal: self.grid.horizontal(),
        }
    }

    pub(super) fn grid_layout_changed(&self, frame: &GridFrameLayout) -> bool {
        self.layout != frame.layout
            || !frame.measured.is_empty()
            || self.grid.horizontal() != HorizontalScroll::Offset(frame.layout.scroll_x)
    }

    pub(super) fn commit_grid_layout(&mut self, cx: &mut Context<Self>) {
        // GPUI may draw synchronously before dispatching input. The effect normally
        // drains this slot first, but input must use that fresh frame if it hasn't.
        let Some(frame) = self.pending_layout.take() else {
            return;
        };
        if frame.font_size == self.prefs.font_size {
            for (column, measurement) in frame.measured {
                if self
                    .data
                    .columns
                    .get(column)
                    .is_some_and(|c| c.id == measurement.id)
                    && self.data.columns.entry(column).width == ColumnWidth::Unmeasured
                {
                    self.data
                        .columns
                        .set_width(column, ColumnWidth::Measured(measurement.width));
                }
            }
        }
        self.grid.commit(&frame.layout, frame.horizontal);
        let find_viewport_changed = frame.layout.highlight_blocks()
            != self.layout.highlight_blocks()
            || frame.layout.visible_columns() != self.layout.visible_columns();
        self.layout = frame.layout;
        if find_viewport_changed {
            cx.emit(FindViewportChanged);
        }
    }
}
