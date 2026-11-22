//! The table view: fixed-height virtualized rows and columns, painted with
//! one canvas. `render` fetches the visible text from Core synchronously and
//! builds a [`Frame`]; the canvas paints it. Nothing is drawn as a
//! placeholder: a cell is either text from Core or `null`.

use std::{
    cell::RefCell,
    ops::Range,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gewu_core::ColumnSort;
use gpui_kit::*;

use crate::{
    data::{TableData, TableQuery, copy_tsv},
    scroll::VerticalScroll,
    selection::{Cell, Selection},
};

pub const ROW_HEIGHT: f32 = 32.;
pub const HEADER_HEIGHT: f32 = 32.;
pub const COLUMN_WIDTH: f32 = 110.;
const SCROLLBAR: f32 = 12.;
const MIN_THUMB: f32 = 24.;
const FONT_SIZE: f32 = 13.;
const PAD: f32 = 8.;
const RESIZE_SLOP: f32 = 4.;
const MIN_COLUMN: f32 = 40.;

/// What the last frames did, read by the benchmark driver.
#[derive(Default)]
pub struct Probe {
    pub enabled: bool,
    pub render_start: Option<Instant>,
    pub fetch: Duration,
    /// (render start, paint end, time in Core, cells without text)
    pub frames: Vec<(Instant, Instant, Duration, usize)>,
}

enum Drag {
    None,
    Select,
    Resize {
        column: usize,
        start_x: f32,
        start_width: f32,
    },
    Thumb {
        grab: f32,
    },
}

pub struct TableView {
    pub data: TableData,
    pub scroll: VerticalScroll,
    scroll_x: f32,
    widths: Vec<f32>,
    names: Vec<SharedString>,
    pub selection: Option<Selection>,
    drag: Drag,
    pub focus: FocusHandle,
    pub(crate) find: crate::find::TableFind,
    requested: Arc<TableQuery>,
    pub query_error: Option<String>,
    /// When the pending sort started.
    pub sorting: Option<Instant>,
    pub last_sort: Option<Duration>,
    pub last_copy: Option<(usize, Duration)>,
    copy_version: u64,
    pub probe: Rc<RefCell<Probe>>,
    /// Last laid-out geometry, for hit testing.
    layout: Layout,
}

#[derive(Clone)]
struct Layout {
    size: Size<Pixels>,
    gutter: f32,
    /// Visible columns: (index, x in window, width).
    columns: Vec<(usize, f32, f32)>,
    vertical: VerticalScroll,
    scroll_x: f32,
}

impl Layout {
    fn body_right(&self) -> f32 {
        self.size.width.as_f32() - SCROLLBAR
    }

    fn body_bottom(&self) -> f32 {
        self.size.height.as_f32()
    }

    fn column_at(&self, x: f32) -> Option<usize> {
        if !(self.gutter..self.body_right()).contains(&x) {
            return None;
        }
        self.columns
            .iter()
            .find(|(_, left, width)| (*left..left + width).contains(&x))
            .map(|c| c.0)
    }

    fn cell_at(&self, x: f32, y: f32) -> Option<Cell> {
        if !(HEADER_HEIGHT..self.body_bottom()).contains(&y) {
            return None;
        }
        Some(Cell {
            column: self.column_at(x)?,
            row: self.vertical.row_at(f64::from(y - HEADER_HEIGHT))?,
        })
    }

    /// A drag past the painted cells extends to the nearest visible cell.
    fn drag_cell_at(&self, x: f32, y: f32) -> Option<Cell> {
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

struct PaintColumn {
    x: f32,
    width: f32,
    right_aligned: bool,
    /// Cell top, rendered text, and whether it represents null.
    cells: Vec<(f32, SharedString, bool)>,
}

/// Everything one paint needs, moved into the canvas.
struct Frame {
    layout: Layout,
    header: Vec<(f32, f32, SharedString, Option<bool>)>,
    gutter: Vec<(f32, SharedString)>,
    columns: Vec<PaintColumn>,
    stripes: Vec<f32>,
    matches: Vec<Bounds<Pixels>>,
    selection: Option<Bounds<Pixels>>,
    head: Option<Bounds<Pixels>>,
    thumb: Bounds<Pixels>,
    missing: usize,
    status: Option<SharedString>,
    probe: Rc<RefCell<Probe>>,
}

impl TableView {
    pub fn new(data: TableData, cx: &mut Context<Self>) -> Self {
        let names = data
            .columns
            .iter()
            .map(|c| SharedString::from(c.name.clone()))
            .collect();
        let widths = vec![COLUMN_WIDTH; data.columns.len()];
        let scroll = VerticalScroll::new(
            data.row_count(),
            ROW_HEIGHT.into(),
            800.0 - f64::from(HEADER_HEIGHT),
        );
        Self {
            requested: data.query().clone(),
            data,
            layout: Layout {
                size: Size::default(),
                gutter: 0.,
                columns: vec![],
                scroll_x: 0.,
                vertical: scroll.clone(),
            },
            scroll,
            scroll_x: 0.,
            widths,
            names,
            selection: None,
            drag: Drag::None,
            focus: cx.focus_handle(),
            find: Default::default(),
            query_error: None,
            sorting: None,
            last_sort: None,
            last_copy: None,
            copy_version: 0,
            probe: Default::default(),
        }
    }

    fn content_width(&self) -> f32 {
        self.widths.iter().sum()
    }

    fn compute_layout(&mut self, size: Size<Pixels>) -> Layout {
        let rows = self.data.row_count();
        let digits = rows.max(1).to_string().len() as f32;
        let gutter = (digits * 8. + 2. * PAD).ceil();
        let body_width = size.width.as_f32() - gutter - SCROLLBAR;
        self.scroll
            .set_viewport((size.height.as_f32() - HEADER_HEIGHT) as f64);
        self.scroll_x = self
            .scroll_x
            .clamp(0., (self.content_width() - body_width).max(0.));
        let mut columns = vec![];
        let mut x = 0.;
        for (index, &width) in self.widths.iter().enumerate() {
            if x + width > self.scroll_x && x < self.scroll_x + body_width {
                columns.push((index, gutter + x - self.scroll_x, width));
            }
            x += width;
        }
        Layout {
            size,
            gutter,
            columns,
            scroll_x: self.scroll_x,
            vertical: self.scroll.clone(),
        }
    }

    // MARK: Render

    fn build_frame(&mut self, layout: Layout) -> Frame {
        let visible: Vec<usize> = layout.columns.iter().map(|c| c.0).collect();
        let rows = layout.vertical.visible_rows();
        let fetch = self.data.prefetch(&visible, rows.clone());
        self.probe.borrow_mut().fetch = fetch;

        let row_y = |row: u64| HEADER_HEIGHT + layout.vertical.row_top(row) as f32;
        let mut missing = 0;
        let mut matches = vec![];
        let mut columns = Vec::with_capacity(layout.columns.len());
        for &(column, x, width) in &layout.columns {
            let mut cells = Vec::with_capacity(rows.clone().count());
            for row in rows.clone() {
                let (text, null) = match self.data.cell(column, row) {
                    Some(Some(text)) => (text, false),
                    Some(None) => (SharedString::new_static("null"), true),
                    None => {
                        missing += 1;
                        continue;
                    }
                };
                if self.find.matched(self.data.columns[column].id, row) {
                    matches.push(Bounds::new(
                        point(px(x), px(row_y(row))),
                        size_(width, ROW_HEIGHT),
                    ));
                }
                cells.push((row_y(row), text, null));
            }
            columns.push(PaintColumn {
                x,
                width,
                right_aligned: self.data.is_number(column),
                cells,
            });
        }
        let header = layout
            .columns
            .iter()
            .map(|&(column, x, width)| {
                let id = self.data.columns[column].id;
                let sort = self
                    .data
                    .query()
                    .sorts
                    .iter()
                    .find(|s| s.column == id)
                    .map(|s| s.descending);
                (x, width, self.names[column].clone(), sort)
            })
            .collect();
        let gutter = rows
            .clone()
            .map(|row| {
                (
                    row_y(row),
                    SharedString::from((self.data.source_row(row) + 1).to_string()),
                )
            })
            .collect();
        let stripes = rows.filter(|r| r % 2 == 1).map(row_y).collect();
        let cell_bounds = |rows: Range<u64>, cols: Range<usize>| -> Option<Bounds<Pixels>> {
            let x0 =
                layout.gutter + self.widths[..cols.start].iter().sum::<f32>() - layout.scroll_x;
            let x1 = layout.gutter + self.widths[..cols.end].iter().sum::<f32>() - layout.scroll_x;
            let y0 = (HEADER_HEIGHT as f64 + layout.vertical.row_top(rows.start)).max(-1e6) as f32;
            let y1 = (HEADER_HEIGHT as f64 + layout.vertical.row_top(rows.end)).min(1e6) as f32;
            (y1 > HEADER_HEIGHT && y0 < layout.body_bottom())
                .then(|| Bounds::from_corners(point(px(x0), px(y0)), point(px(x1), px(y1))))
        };
        let selection = self
            .selection
            .and_then(|s| cell_bounds(s.rows(), s.columns()));
        let head = self.selection.and_then(|s| {
            cell_bounds(s.head.row..s.head.row + 1, s.head.column..s.head.column + 1)
        });
        let status = self
            .sorting
            .map(|t| {
                SharedString::from(format!("Applying query… {:.1}s", t.elapsed().as_secs_f32()))
            })
            .or_else(|| {
                self.query_error
                    .as_ref()
                    .map(|message| message.clone().into())
            })
            .or_else(|| {
                (!self.data.query().filters.is_empty())
                    .then(|| format!("Filtered · {} rows", self.data.row_count()).into())
            });
        Frame {
            thumb: self.thumb(&layout),
            layout,
            header,
            gutter,
            columns,
            stripes,
            matches,
            selection,
            head,
            missing,
            status,
            probe: self.probe.clone(),
        }
    }

    fn thumb(&self, layout: &Layout) -> Bounds<Pixels> {
        let track = (layout.body_bottom() - HEADER_HEIGHT).max(0.);
        let content = self.scroll.rows() as f64 * self.scroll.row_height();
        let size = ((self.scroll.viewport() / content.max(1.0)) as f32 * track)
            .clamp(MIN_THUMB.min(track), track);
        let y = HEADER_HEIGHT + self.scroll.fraction() as f32 * (track - size);
        Bounds::new(
            point(px(layout.body_right()), px(y)),
            size_(SCROLLBAR, size),
        )
    }

    // Geometry for the self-test.
    pub fn gutter_width(&self) -> f32 {
        self.layout.gutter
    }

    pub fn column_width(&self, column: usize) -> f32 {
        self.widths[column]
    }

    pub fn column_left(&self, column: usize) -> Option<f32> {
        self.layout
            .columns
            .iter()
            .find(|c| c.0 == column)
            .map(|c| c.1)
    }

    /// Visible (rows, columns) in the last layout.
    pub fn visible_counts(&self) -> (usize, usize) {
        (
            self.layout.vertical.visible_rows().count(),
            self.layout.columns.len(),
        )
    }

    pub fn thumb_bounds(&self) -> Bounds<Pixels> {
        self.thumb(&self.layout)
    }

    // MARK: Interaction

    pub fn scroll_to_row(&mut self, row: u64) {
        self.scroll
            .set_anchor(crate::scroll::Anchor { row, offset: 0.0 });
    }

    pub(crate) fn reveal(&mut self, cell: Cell) {
        self.scroll.reveal(cell.row);
        let body = self.layout.body_right() - self.layout.gutter;
        let x0: f32 = self.widths[..cell.column].iter().sum();
        let x1 = x0 + self.widths[cell.column];
        if x0 < self.scroll_x {
            self.scroll_x = x0;
        } else if x1 > self.scroll_x + body {
            self.scroll_x = x1 - body;
        }
    }

    /// The column whose right divider is under `x` in the header.
    fn divider_at(&self, x: f32) -> Option<usize> {
        self.layout
            .columns
            .iter()
            .find(|&&(_, cx, w)| (x - (cx + w)).abs() <= RESIZE_SLOP)
            .map(|c| c.0)
    }

    fn mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let (x, y) = (ev.position.x.as_f32(), ev.position.y.as_f32());
        let l = &self.layout;
        if y >= HEADER_HEIGHT && x >= l.body_right() {
            let thumb = self.thumb(l);
            if thumb.contains(&ev.position) {
                self.drag = Drag::Thumb {
                    grab: y - thumb.origin.y.as_f32(),
                };
            } else {
                // Track click: jump there, centered on the thumb.
                self.drag_thumb(y - thumb.size.height.as_f32() / 2.);
                self.drag = Drag::Thumb {
                    grab: thumb.size.height.as_f32() / 2.,
                };
            }
        } else if (0.0..HEADER_HEIGHT).contains(&y) && (l.gutter..l.body_right()).contains(&x) {
            if let Some(column) = self.divider_at(x) {
                if ev.click_count == 2 {
                    self.autofit(column, window);
                } else {
                    self.drag = Drag::Resize {
                        column,
                        start_x: x,
                        start_width: self.widths[column],
                    };
                }
            } else if let Some(column) = l.column_at(x) {
                self.cycle_sort(column, cx);
            }
        } else if let Some(cell) = l.cell_at(x, y) {
            self.selection = Some(match (self.selection, ev.modifiers.shift) {
                (Some(s), true) => Selection {
                    anchor: s.anchor,
                    head: cell,
                },
                _ => Selection::single(cell),
            });
            self.drag = Drag::Select;
        }
        cx.notify();
    }

    fn drag_thumb(&mut self, top: f32) {
        let l = &self.layout;
        let track = l.body_bottom() - HEADER_HEIGHT;
        let size = self.thumb(l).size.height.as_f32();
        let fraction = ((top - HEADER_HEIGHT) / (track - size).max(1.)) as f64;
        self.scroll.set_fraction(fraction);
    }

    fn mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if ev.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let (x, y) = (ev.position.x.as_f32(), ev.position.y.as_f32());
        match self.drag {
            Drag::None => return,
            Drag::Thumb { grab } => self.drag_thumb(y - grab),
            Drag::Resize {
                column,
                start_x,
                start_width,
            } => {
                self.widths[column] = (start_width + x - start_x).max(MIN_COLUMN);
            }
            Drag::Select => {
                // Follow the pointer past the body edges (simplified: one step per move event).
                let bottom = self.layout.body_bottom();
                if y < HEADER_HEIGHT + 8. {
                    self.scroll.scroll_by((y - HEADER_HEIGHT - 8.) as f64);
                } else if y > bottom - 8. {
                    self.scroll.scroll_by((y - bottom + 8.) as f64);
                }
                if x > self.layout.body_right() - 8. {
                    self.scroll_x += x - self.layout.body_right() + 8.;
                } else if x < self.layout.gutter + 8. {
                    self.scroll_x = (self.scroll_x + x - self.layout.gutter - 8.).max(0.);
                }
                let head = if self.layout.vertical.anchor() == self.scroll.anchor()
                    && self.layout.scroll_x == self.scroll_x
                {
                    self.layout.drag_cell_at(x, y)
                } else {
                    self.compute_layout(self.layout.size).drag_cell_at(x, y)
                };
                if let (Some(s), Some(head)) = (&mut self.selection, head) {
                    s.head = head;
                }
            }
        }
        cx.notify();
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.drag = Drag::None;
    }

    fn scroll_wheel(&mut self, ev: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        // Trackpads send exact pixels, including the platform's momentum events
        // after the fingers lift (phase `Moved`); wheels send lines.
        let delta = ev.delta.pixel_delta(px(ROW_HEIGHT));
        let (mut dx, mut dy) = (delta.x.as_f32() as f64, delta.y.as_f32() as f64);
        if ev.modifiers.shift && dx == 0.0 {
            (dx, dy) = (dy, 0.0);
        }
        self.scroll.scroll_by(-dy);
        self.scroll_x = (self.scroll_x - dx as f32).max(0.);
        cx.notify();
        cx.stop_propagation();
    }

    fn key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let (cmd, shift) = (k.modifiers.platform, k.modifiers.shift);
        if cmd && k.key == "l" && (shift || k.modifiers.alt) {
            if shift {
                self.filter_selected(cx);
            } else {
                self.clear_filters(cx);
            }
            cx.stop_propagation();
            return;
        }
        let rows = self.data.row_count();
        let columns = self.widths.len();
        if rows == 0 {
            return;
        }
        let current = self
            .selection
            .unwrap_or(Selection::single(Cell { row: 0, column: 0 }));
        let page = self.scroll.page_rows() as i64;
        let far = i64::MAX / 2;
        let next = match (k.key.as_str(), cmd) {
            ("a", true) => Some(Selection::all(rows, columns)),
            ("c", true) => {
                self.copy(window, cx);
                None
            }
            ("up", false) => Some(current.moved(-1, 0, shift, rows, columns)),
            ("down", false) => Some(current.moved(1, 0, shift, rows, columns)),
            ("left", false) => Some(current.moved(0, -1, shift, rows, columns)),
            ("right", false) => Some(current.moved(0, 1, shift, rows, columns)),
            ("up", true) | ("home", _) => Some(current.moved(-far, 0, shift, rows, columns)),
            ("down", true) | ("end", _) => Some(current.moved(far, 0, shift, rows, columns)),
            ("left", true) => Some(current.moved(0, -far, shift, rows, columns)),
            ("right", true) => Some(current.moved(0, far, shift, rows, columns)),
            ("pageup", _) => Some(current.moved(-page, 0, shift, rows, columns)),
            ("pagedown", _) => Some(current.moved(page, 0, shift, rows, columns)),
            _ => None,
        };
        if let Some(next) = next {
            self.selection = Some(next);
            if k.key != "a" {
                self.reveal(next.head);
            }
            cx.notify();
            cx.stop_propagation();
        }
    }

    // MARK: Columns

    fn autofit(&mut self, column: usize, window: &mut Window) {
        let font = window.text_style().font();
        let measure = |text: &SharedString| {
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color: black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(text.clone(), px(FONT_SIZE), &[run], None)
                .width
                .as_f32()
        };
        let mut width = measure(&self.names[column]) + 16.;
        for row in self.layout.vertical.visible_rows() {
            if let Some(Some(text)) = self.data.cell(column, row) {
                width = width.max(measure(&text));
            }
        }
        self.widths[column] = (width + 2. * PAD).ceil().max(MIN_COLUMN);
    }

    // MARK: Sorting

    /// Advance the requested ordering even while an earlier sort is running.
    pub fn cycle_sort(&mut self, column: usize, cx: &mut Context<Self>) {
        let id = self.data.columns[column].id;
        let sorts = &mut Arc::make_mut(&mut self.requested).sorts;
        *sorts = match sorts.iter().find(|s| s.column == id) {
            None => vec![ColumnSort {
                column: id,
                descending: false,
            }],
            Some(s) if !s.descending => vec![ColumnSort {
                column: id,
                descending: true,
            }],
            Some(_) => vec![],
        };
        self.apply_query(cx);
    }

    pub fn filter_selected(&mut self, cx: &mut Context<Self>) {
        let Some(cell) = self.selection.map(|s| s.head) else {
            self.query_error = Some("Select a cell to filter by its value".into());
            cx.notify();
            return;
        };
        let id = self.data.columns[cell.column].id;
        match self.data.view().exact_filter_for_cell(id, cell.row) {
            Ok(Some(filter)) => {
                let filters = &mut Arc::make_mut(&mut self.requested).filters;
                filters.retain(|f| f.column != id);
                filters.push(filter);
                self.apply_query(cx);
            }
            Ok(None) => (),
            Err(error) => {
                self.query_error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    pub fn clear_filters(&mut self, cx: &mut Context<Self>) {
        Arc::make_mut(&mut self.requested).filters.clear();
        self.apply_query(cx);
    }

    /// Keep at most one worker active. Coalesce further input into the latest
    /// requested query, without ever publishing an obsolete result.
    fn apply_query(&mut self, cx: &mut Context<Self>) {
        self.query_error = None;
        if self.sorting.is_some() || &self.requested == self.data.query() {
            cx.notify();
            return;
        }
        let requested = self.requested.clone();
        let previous = self.data.snapshot();
        let started = Instant::now();
        self.sorting = Some(started);
        cx.spawn(async move |this, cx| {
            let query = requested.clone();
            let result = cx
                .background_spawn(async move { previous.applying(query) })
                .await;
            this.update(cx, |table, cx| {
                table.sorting = None;
                if requested != table.requested {
                    table.apply_query(cx);
                    return;
                }
                match result {
                    Ok(applied) => {
                        table.last_sort = Some(started.elapsed());
                        table.data.install(applied);
                        table.scroll.set_rows(table.data.row_count());
                        table.selection = None;
                        table.drag = Drag::None;
                        table.reset_find_for_view();
                    }
                    Err(error) => {
                        table.requested = table.data.query().clone();
                        table.query_error = Some(error.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    // MARK: Copy

    fn copy(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = self.selection else {
            return;
        };
        self.copy_version += 1;
        let version = self.copy_version;
        let view = self.data.view().clone();
        let columns = self.data.column_ids(selection.columns());
        let rows = selection.rows();
        let started = Instant::now();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { copy_tsv(&view, rows, &columns, false) })
                .await;
            this.update(cx, |table, cx| {
                if version != table.copy_version {
                    return;
                }
                match result {
                    Ok(text) => {
                        table.last_copy = Some((text.len(), started.elapsed()));
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                    Err(error) => eprintln!("copy failed: {error}"),
                }
            })
            .ok();
        })
        .detach();
    }
}

fn size_(w: f32, h: f32) -> Size<Pixels> {
    size(px(w), px(h))
}

impl Render for TableView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.probe.borrow().enabled {
            self.probe.borrow_mut().render_start = Some(Instant::now());
        }
        let mut table_size = window.viewport_size();
        if self.find.open {
            table_size.height = (table_size.height - px(40.)).max(px(1.));
        }
        let layout = self.compute_layout(table_size);
        if self.find.open {
            let columns: Vec<_> = layout.columns.iter().map(|c| c.0).collect();
            self.request_highlights(layout.vertical.visible_rows(), &columns, cx);
        }
        self.layout = layout.clone();
        let frame = self.build_frame(layout);
        if self.sorting.is_some() {
            // Keep the status line ticking while sorting.
            window.request_animation_frame();
        }
        let table = div()
            .w_full()
            .h(table_size.height)
            .flex_none()
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|table, _: &base::input::Copy, window, cx| table.copy(window, cx)),
            )
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_scroll(cx.listener(Self::scroll_wheel))
            .child(
                canvas(
                    |_, _, _| (),
                    move |_, _, window, cx| paint(frame, window, cx),
                )
                .size_full(),
            );
        div()
            .size_full()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(|table, ev: &KeyDownEvent, window, cx| {
                let k = &ev.keystroke;
                match (k.key.as_str(), k.modifiers.platform) {
                    ("f", true) => table.open_find(window, cx),
                    ("g", true) => {
                        if !table.find.open {
                            table.open_find(window, cx);
                        }
                        table.find_match(k.modifiers.shift, cx);
                    }
                    ("escape", false) if table.find.open => table.close_find(window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|table, _: &base::input::Search, window, cx| {
                table.open_find(window, cx);
            }))
            .on_action(cx.listener(|table, _: &base::input::Escape, window, cx| {
                if table.find.open {
                    table.close_find(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .child(table)
            .children(self.find_bar(cx))
    }
}

fn paint(frame: Frame, window: &mut Window, cx: &mut App) {
    let l = &frame.layout;
    let (w, h) = (l.size.width.as_f32(), l.size.height.as_f32());
    let bg = rgb(0xffffff);
    let stripe = rgb(0xf6f7f9);
    let grid = rgb(0xe3e5e8);
    let header_bg = rgb(0xf0f1f3);
    let text_color: Hsla = rgb(0x1f2328).into();
    let muted: Hsla = rgb(0x8c959f).into();
    let font = window.text_style().font();
    let run = |len: usize, color: Hsla| TextRun {
        len,
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let rect = |x: f32, y: f32, w: f32, h: f32| Bounds::new(point(px(x), px(y)), size_(w, h));
    let text_y = |y: f32| y + (ROW_HEIGHT - FONT_SIZE * 1.3) / 2.;
    let line_height = px(FONT_SIZE * 1.3);

    window.paint_quad(fill(rect(0., 0., w, h), bg));
    for &y in &frame.stripes {
        window.paint_quad(fill(rect(l.gutter, y, w - l.gutter, ROW_HEIGHT), stripe));
    }
    for bounds in frame.matches {
        window.paint_quad(fill(bounds, rgb(0xffe594)));
    }
    if let Some(selection) = frame.selection {
        window.paint_quad(fill(selection, rgba(0x2f6feb22)));
    }
    if let Some(head) = frame.head {
        window.paint_quad(outline(head, rgb(0x2f6feb), BorderStyle::Solid));
    }

    // Cells, one content mask per column.
    let body = rect(
        l.gutter,
        HEADER_HEIGHT,
        l.body_right() - l.gutter,
        h - HEADER_HEIGHT,
    );
    window.with_content_mask(Some(ContentMask { bounds: body }), |window| {
        for PaintColumn {
            x,
            width,
            right_aligned,
            cells,
        } in &frame.columns
        {
            window.paint_quad(fill(
                rect(x + width - 1., HEADER_HEIGHT, 1., h - HEADER_HEIGHT),
                grid,
            ));
            let mask = rect(
                x + PAD,
                HEADER_HEIGHT,
                (width - 2. * PAD).max(0.),
                h - HEADER_HEIGHT,
            );
            window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
                for (y, text, null) in cells {
                    let line = window.text_system().shape_line(
                        text.clone(),
                        px(FONT_SIZE),
                        &[run(text.len(), if *null { muted } else { text_color })],
                        None,
                    );
                    let tx = if *right_aligned {
                        x + width - PAD - line.width.as_f32()
                    } else {
                        x + PAD
                    };
                    line.paint(
                        point(px(tx), px(text_y(*y))),
                        line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .ok();
                }
            });
        }
    });

    // Row number gutter with source rows.
    window.paint_quad(fill(
        rect(0., HEADER_HEIGHT, l.gutter, h - HEADER_HEIGHT),
        header_bg,
    ));
    window.paint_quad(fill(rect(l.gutter - 1., 0., 1., h), grid));
    window.with_content_mask(
        Some(ContentMask {
            bounds: rect(0., HEADER_HEIGHT, l.gutter, h),
        }),
        |window| {
            for (y, label) in &frame.gutter {
                let line = window.text_system().shape_line(
                    label.clone(),
                    px(FONT_SIZE),
                    &[run(label.len(), muted)],
                    None,
                );
                let tx = l.gutter - PAD - line.width.as_f32();
                line.paint(
                    point(px(tx), px(text_y(*y))),
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .ok();
            }
        },
    );

    // Header.
    window.paint_quad(fill(rect(0., 0., w, HEADER_HEIGHT), header_bg));
    window.paint_quad(fill(rect(0., HEADER_HEIGHT - 1., w, 1.), grid));
    let header_mask = rect(l.gutter, 0., l.body_right() - l.gutter, HEADER_HEIGHT);
    window.with_content_mask(
        Some(ContentMask {
            bounds: header_mask,
        }),
        |window| {
            for (x, width, name, sort) in &frame.header {
                window.paint_quad(fill(rect(x + width - 1., 0., 1., HEADER_HEIGHT), grid));
                let label: SharedString = match sort {
                    Some(false) => format!("{name} ▲").into(),
                    Some(true) => format!("{name} ▼").into(),
                    None => name.clone(),
                };
                let line = window.text_system().shape_line(
                    label.clone(),
                    px(FONT_SIZE),
                    &[run(label.len(), text_color)],
                    None,
                );
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: rect(x + PAD, 0., width - 2. * PAD, HEADER_HEIGHT),
                    }),
                    |window| {
                        line.paint(
                            point(px(x + PAD), px(text_y(0.))),
                            line_height,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        )
                        .ok();
                    },
                );
            }
        },
    );

    // Scrollbar.
    window.paint_quad(fill(
        rect(l.body_right(), HEADER_HEIGHT, SCROLLBAR, h - HEADER_HEIGHT),
        rgb(0xf3f4f6),
    ));
    let thumb = frame.thumb;
    let inset = Bounds::new(
        point(thumb.origin.x + px(2.), thumb.origin.y),
        size(thumb.size.width - px(4.), thumb.size.height),
    );
    window.paint_quad(fill(inset, rgb(0xb8bec6)).corner_radii(px(4.)));

    if let Some(status) = &frame.status {
        let line = window.text_system().shape_line(
            status.clone(),
            px(FONT_SIZE),
            &[run(status.len(), rgb(0xcf222e).into())],
            None,
        );
        line.paint(
            point(px(PAD), px(text_y(0.))),
            line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        )
        .ok();
    }

    let mut probe = frame.probe.borrow_mut();
    if let Some(start) = probe.render_start.take() {
        let fetch = probe.fetch;
        probe
            .frames
            .push((start, Instant::now(), fetch, frame.missing));
    }
}

#[cfg(test)]
mod tests {
    use super::{Cell, Layout, ROW_HEIGHT, VerticalScroll, size_};
    use crate::scroll::Anchor;

    #[test]
    fn click_requires_a_painted_cell_but_drag_extends_to_the_edge() {
        let mut layout = Layout {
            size: size_(200., 200.),
            gutter: 24.,
            scroll_x: 194.,
            // Column 2 starts behind the frozen gutter.
            columns: vec![(2, -10., 80.), (3, 70., 60.)],
            vertical: VerticalScroll::new(2, ROW_HEIGHT.into(), 168.),
        };
        let first = Cell { row: 0, column: 2 };
        let last = Cell { row: 1, column: 3 };
        assert_eq!(layout.cell_at(24., 32.), Some(first));
        assert_eq!(layout.cell_at(70., 64.), Some(last));
        for (x, y) in [(23., 32.), (24., 31.), (130., 48.), (80., 96.)] {
            assert_eq!(layout.cell_at(x, y), None);
        }
        assert_eq!(layout.drag_cell_at(-10., 0.), Some(first));
        assert_eq!(layout.drag_cell_at(300., 300.), Some(last));

        layout.vertical = VerticalScroll::new(100, ROW_HEIGHT.into(), 168.);
        layout.vertical.set_anchor(Anchor {
            row: 45,
            offset: 12.,
        });
        layout.columns[1].2 = 180.;
        assert_eq!(layout.cell_at(70., 52.), Some(Cell { row: 46, column: 3 }));
        assert_eq!(layout.cell_at(188., 52.), None);
        assert_eq!(layout.cell_at(70., 200.), None);
        assert_eq!(
            layout.drag_cell_at(300., 300.),
            Some(Cell { row: 50, column: 3 })
        );
    }
}
