use super::*;
use crate::columns::TableColumns;

/// Keep a reveal anchored to its column while layout measures the surrounding widths.
#[derive(Clone, Copy)]
pub(super) enum HorizontalScroll {
    Offset(f32),
    Reveal {
        column: gewu_core::ColumnId,
        from: f32,
    },
}

impl HorizontalScroll {
    pub(super) fn offset(self, columns: &TableColumns, viewport: f32) -> f32 {
        match self {
            Self::Offset(offset) => offset,
            Self::Reveal { column, from } => {
                let Some(index) = columns.position(column) else {
                    return from;
                };
                let index = index.index();
                let left: f32 = columns.widths().take(index).sum();
                let right_aligned = left + columns.width(index) - viewport;
                // Show the entire column when it fits, otherwise keep as much of it in view as
                // possible.
                from.clamp(left.min(right_aligned), left.max(right_aligned))
            }
        }
    }
}

pub(super) struct Layout {
    pub(super) bounds: Bounds<Pixels>,
    pub(super) gutter: f32,
    /// Visible columns: (index, x in window, width).
    pub(super) columns: Vec<(usize, f32, f32)>,
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
    ) -> Self {
        let size = bounds.size;
        let body_width = (size.width.as_f32() - SCROLLBAR - gutter).max(0.);
        let widths = grid_columns.widths();
        let content_width = widths.clone().sum::<f32>();
        let scrollbar_height = if content_width > body_width {
            SCROLLBAR
        } else {
            0.
        };
        let mut vertical = scroll.clone();
        vertical.set_viewport(f64::from(
            (size.height.as_f32() - HEADER_HEIGHT - scrollbar_height).max(0.),
        ));
        let scroll_x = horizontal
            .offset(grid_columns, body_width)
            .clamp(0., (content_width - body_width).max(0.));
        let mut columns = Vec::new();
        let mut x = 0.;
        for (index, width) in widths.enumerate() {
            if x >= scroll_x + body_width {
                break;
            }
            if x + width > scroll_x {
                columns.push((index, gutter + x - scroll_x, width));
            }
            x += width;
        }
        Self {
            bounds,
            gutter,
            columns,
            vertical,
            scroll_x,
            content_width,
        }
    }

    pub(super) fn vertical_thumb(&self) -> Bounds<Pixels> {
        let track = (self.body_bottom() - HEADER_HEIGHT).max(0.);
        let content = self.vertical.rows() as f64 * self.vertical.row_height();
        let height = ((self.vertical.viewport() / content.max(1.)) as f32 * track)
            .clamp(MIN_THUMB.min(track), track);
        let y = HEADER_HEIGHT + self.vertical.fraction() as f32 * (track - height);
        Bounds::new(
            point(px(self.body_right()), px(y)),
            size_(SCROLLBAR, height),
        )
    }

    pub(super) fn body_right(&self) -> f32 {
        (self.bounds.size.width.as_f32() - SCROLLBAR).max(0.)
    }

    pub(super) fn body_bottom(&self) -> f32 {
        (self.bounds.size.height.as_f32()
            - if self.has_horizontal_scroll() {
                SCROLLBAR
            } else {
                0.
            })
        .max(0.)
    }

    pub(super) fn has_horizontal_scroll(&self) -> bool {
        self.content_width > (self.body_right() - self.gutter).max(0.)
    }

    pub(super) fn horizontal_thumb(&self) -> Bounds<Pixels> {
        let track = (self.body_right() - self.gutter).max(0.);
        let width = (track * track / self.content_width.max(1.)).clamp(MIN_THUMB.min(track), track);
        let max_scroll = (self.content_width - track).max(1.);
        let left = self.gutter + self.scroll_x / max_scroll * (track - width);
        Bounds::new(
            point(px(left), px(self.body_bottom())),
            size_(width, SCROLLBAR),
        )
    }

    pub(super) fn column_at(&self, x: f32) -> Option<usize> {
        if !(self.gutter..self.body_right()).contains(&x) {
            return None;
        }
        self.columns
            .iter()
            .find(|(_, left, width)| (*left..left + width).contains(&x))
            .map(|c| c.0)
    }

    pub(super) fn cell_at(&self, x: f32, y: f32) -> Option<Cell> {
        if !(HEADER_HEIGHT..self.body_bottom()).contains(&y) {
            return None;
        }
        Some(Cell {
            column: self.column_at(x)?,
            row: self.vertical.row_at(f64::from(y - HEADER_HEIGHT))?,
        })
    }

    /// A drag past the painted cells extends to the nearest visible cell.
    pub(super) fn drag_cell_at(&self, x: f32, y: f32) -> Option<Cell> {
        let &(_, last_x, last_width) = self.columns.last()?;
        let right = self.body_right().min(last_x + last_width);
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
}

pub(super) struct FindViewportChanged;

impl EventEmitter<FindViewportChanged> for TableView {}

impl TableView {
    /// GPUI supplies the resolved grid bounds after flex layout. Resolve measured widths and
    /// clamp the scroll request here before publishing one snapshot for paint, input and AX.
    pub(super) fn layout_grid(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Rc<Layout> {
        let mut layout = self.compute_layout(bounds);
        while self.size_new_columns(&layout, window) {
            layout = self.compute_layout(bounds);
        }
        self.scroll.set_viewport(layout.vertical.viewport());
        self.horizontal = HorizontalScroll::Offset(layout.scroll_x);
        if layout.highlight_blocks() != self.layout.highlight_blocks()
            || layout.visible_columns() != self.layout.visible_columns()
        {
            cx.emit(FindViewportChanged);
        }
        self.layout = Rc::new(layout);
        Rc::clone(&self.layout)
    }
}

impl Layout {
    fn highlight_blocks(&self) -> Range<u64> {
        let rows = self.vertical.visible_rows();
        if rows.is_empty() {
            return 0..0;
        }
        rows.start / crate::data::BLOCK_ROWS..rows.end.div_ceil(crate::data::BLOCK_ROWS)
    }

    fn visible_columns(&self) -> Option<(usize, usize)> {
        Some((self.columns.first()?.0, self.columns.last()?.0))
    }
}
