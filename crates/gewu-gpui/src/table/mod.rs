//! The table view: fixed-height virtualized rows and columns, painted with
//! one canvas. `render` fetches the visible text from Core synchronously and
//! builds a [`Frame`]; the canvas paints it. Nothing is drawn as a
//! placeholder: a cell is either text from Core or `null`.

use std::{cell::RefCell, ops::Range, rc::Rc, sync::Arc};

use gewu_core::ColumnSort;
use gpui_kit::*;

use crate::{
    actions::{self, Motion, Navigate},
    data::{TableData, TableQuery, copy_tsv},
    preferences::Preferences,
    scroll::VerticalScroll,
    selection::{Cell, Selection},
};

#[cfg(test)]
const ROW_HEIGHT: f32 = 32.;
pub const HEADER_HEIGHT: f32 = 32.;
const SCROLLBAR: f32 = 12.;
const MIN_THUMB: f32 = 24.;
const ROW_NUMBER_SIZE: f32 = 11.;
const PAD: f32 = 8.;
const RESIZE_SLOP: f32 = 4.;
const MIN_COLUMN: f32 = 76.;

use layout::HorizontalScroll;

use crate::columns::ColumnWidth;

enum Drag {
    None,
    Select {
        edge_pointer: Option<Point<Pixels>>,
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

pub struct TableView {
    pub(crate) data: TableData,
    pub(crate) document_title: Option<SharedString>,
    pub(crate) worksheet_tabs: Option<AnyView>,
    prefs: Preferences,
    _style_changes: Subscription,
    _window_activation: Subscription,
    pub(crate) scroll: VerticalScroll,
    horizontal: HorizontalScroll,
    selection: Option<Selection>,
    drag: Drag,
    edge_frame_pending: bool,
    header_hover: Option<HeaderHover>,
    thumb_hover: Option<Axis>,
    body_pointer: Option<Point<Pixels>>,
    scrollbar_timer: Option<Task<()>>,
    pub focus: FocusHandle,
    pub(crate) find: Entity<crate::find::FindController>,
    _find_reveal: Subscription,
    _find_changed: Subscription,
    notice: Option<Notice>,
    controls: controls::Controls,
    query_task: Option<Task<()>>,
    copy_task: Option<Task<()>>,
    /// The same committed geometry used to draw and hit-test the grid.
    layout: Rc<Layout>,
    pending_layout: Rc<RefCell<Option<layout::GridFrameLayout>>>,
    _find_viewport: Subscription,
}

struct PaintColumn {
    x: f32,
    width: f32,
    right_aligned: bool,
    cells: Vec<PaintCell>,
}

struct PaintCell {
    y: f32,
    text: SharedString,
    missing: bool,
    current_match: bool,
    /// Slice of the frame's text matches belonging to this cell.
    spans: Range<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HeaderHover {
    Column(usize),
    Divider(usize),
}

struct PaintHeader {
    name: SharedString,
    kind: SharedString,
    sort: Option<SharedString>,
    funnel: Option<bool>,
    selected: bool,
}

struct FrameRow {
    display: gewu_core::ViewRow,
    source: gewu_core::SourceRow,
    y: f32,
    label: SharedString,
    selected: bool,
}

/// Everything one paint needs, moved into the canvas.
struct Frame {
    formatting_error: Option<SharedString>,
    layout: Rc<Layout>,
    header: Vec<PaintHeader>,
    style: Preferences,
    rows: Vec<FrameRow>,
    columns: Vec<PaintColumn>,
    current_match: Option<Bounds<Pixels>>,
    text_matches: Vec<Range<usize>>,
    selection: Option<Bounds<Pixels>>,
    head: Option<Bounds<Pixels>>,
    active_thumb: Option<Axis>,
    show_scrollbars: bool,
    hovered_row_y: Option<f32>,
    header_hover: Option<HeaderHover>,
}

impl TableView {
    /// Create a virtualized table over an immutable dataset.
    pub fn from_dataset(
        dataset: Arc<gewu_core::Dataset>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        crate::platform::follow_appearance(window, cx);
        Self::new(TableData::from_dataset(dataset), window, cx)
    }

    pub(crate) fn new(data: TableData, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let prefs = Preferences::get(cx);
        let style_changes = cx.observe_global::<Preferences>(|table, cx| {
            let prefs = Preferences::get(cx);
            if table.prefs.font_size != prefs.font_size {
                table.data.columns.invalidate_measurements();
            }
            table.prefs = prefs;
            table
                .scroll
                .set_row_height(f64::from(table.prefs.row_height));
            cx.notify();
        });
        let scroll = VerticalScroll::new(
            data.row_count(),
            f64::from(prefs.row_height),
            800.0 - f64::from(HEADER_HEIGHT),
        );
        let scope = data
            .columns
            .iter()
            .map(|c| (c.id, c.name.clone().into()))
            .collect();
        let find =
            cx.new(|cx| crate::find::FindController::new(data.view().clone(), scope, window, cx));
        let reveal = cx.subscribe(&find, |table, _, hit: &gewu_core::CellMatch, cx| {
            table.commit_grid_layout(cx);
            if let Some(cell) = table.data.display_cell(hit.display_row, hit.column) {
                table.set_selection(Some(Selection::single(cell)), cx);
                table.reveal(cell);
                cx.notify();
            }
        });
        let changed = cx.observe(&find, |_, _, cx| cx.notify());
        Self {
            prefs,
            document_title: None,
            worksheet_tabs: None,
            _style_changes: style_changes,
            _window_activation: cx
                .observe_window_activation(window, Self::window_activation_changed),
            data,
            layout: Rc::new(Layout {
                bounds: Bounds::default(),
                viewport: Size::default(),
                gutter: 0.,
                columns: vec![],
                scroll_x: 0.,
                content_width: 0.,
                vertical: scroll.clone(),
            }),
            pending_layout: Rc::default(),
            _find_viewport: cx.subscribe_self(|table, _: &layout::FindViewportChanged, cx| {
                table.refresh_highlights(cx);
            }),
            scroll,
            horizontal: HorizontalScroll::Offset(0.),
            selection: None,
            drag: Drag::None,
            edge_frame_pending: false,
            header_hover: None,
            thumb_hover: None,
            body_pointer: None,
            scrollbar_timer: None,
            focus: cx.focus_handle(),
            find,
            _find_changed: changed,
            _find_reveal: reveal,
            notice: None,
            controls: Default::default(),
            query_task: None,
            copy_task: None,
        }
    }

    fn refresh_highlights(&mut self, cx: &mut Context<Self>) {
        let columns = self
            .layout
            .columns
            .iter()
            .filter_map(|(i, ..)| self.data.columns.get(*i).map(|c| c.id))
            .collect();
        self.find.update(cx, |find, cx| {
            find.set_viewport(self.layout.vertical.visible_rows(), columns, cx)
        });
    }

    fn reset_grid_interaction(&mut self) {
        self.drag = Drag::None;
        self.header_hover = None;
        self.thumb_hover = None;
        self.body_pointer = None;
    }

    fn sync_find_columns(&mut self, reordered: bool, cx: &mut Context<Self>) {
        let columns = self
            .data
            .columns
            .iter()
            .map(|c| (c.id, c.name.clone().into()))
            .collect();
        self.find
            .update(cx, |find, cx| find.set_columns(columns, reordered, cx));
        self.refresh_highlights(cx);
    }

    fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_grid_layout(cx);
        self.find
            .update(cx, |find, cx| find.focus_input(window, cx));
        self.refresh_highlights(cx);
    }

    fn compute_layout(&self, bounds: Bounds<Pixels>, auto_hide_scrollbars: bool) -> Layout {
        let source_rows = self.data.source_row_count();
        let digits = source_rows.max(1).ilog10() + 1;
        let gutter = if self.prefs.show_row_numbers {
            (digits as f32 * ROW_NUMBER_SIZE * 0.65 + 2. * PAD).ceil()
        } else {
            0.
        };
        Layout::new(
            bounds,
            gutter,
            &self.data.columns,
            &self.scroll,
            self.horizontal,
            auto_hide_scrollbars,
        )
    }

    #[cfg(test)]
    pub fn gutter_width(&self) -> f32 {
        self.layout.gutter
    }

    #[cfg(test)]
    fn column_width(&self, column: usize) -> f32 {
        self.data.columns.width(column)
    }

    fn column_left(&self, column: usize) -> Option<f32> {
        self.layout
            .columns
            .iter()
            .find(|c| c.0 == column)
            .map(|c| c.1)
    }

    #[cfg(test)]
    fn thumb_bounds(&self) -> Bounds<Pixels> {
        self.layout.scrollbar(Axis::Vertical).unwrap().thumb
    }

    pub fn scroll_to_row(&mut self, row: u64) {
        self.scroll
            .set_anchor(crate::scroll::Anchor { row, offset: 0.0 });
    }

    #[cfg(feature = "ui-capture")]
    pub(crate) fn capture_cell_point(&self, column: &str, row: u64) -> Option<Point<Pixels>> {
        let index =
            (0..self.data.columns.len()).find(|&i| self.data.columns.entry(i).label == column)?;
        let left = self.column_left(index)?;
        let row_height = self.layout.vertical.row_height() as f32;
        Some(
            self.layout.bounds.origin
                + point(
                    px(left + 10.),
                    px(HEADER_HEIGHT + self.layout.vertical.row_top(row) as f32 + row_height / 2.),
                ),
        )
    }

    #[cfg(feature = "ui-capture")]
    pub(crate) fn capture_header_bounds(&self, column: &str) -> Option<Bounds<Pixels>> {
        let index =
            (0..self.data.columns.len()).find(|&i| self.data.columns.entry(i).label == column)?;
        let left = self.column_left(index)?;
        let width = self.data.columns.width(index);
        Some(Bounds::new(
            self.layout.bounds.origin + point(px(left), px(0.)),
            size(px(width), px(HEADER_HEIGHT)),
        ))
    }

    #[cfg(feature = "ui-capture")]
    pub(crate) fn capture_dimensions(&self) -> (u64, usize) {
        (self.data.row_count(), self.data.columns.len())
    }

    fn scroll_x(&self) -> f32 {
        self.horizontal.offset(
            &self.data.columns,
            (self.layout.body_right() - self.layout.gutter).max(0.),
        )
    }

    pub(crate) fn reveal(&mut self, cell: Cell) {
        self.scroll.reveal(cell.row);
        self.horizontal = HorizontalScroll::Reveal {
            from: self.scroll_x(),
            column: self.data.columns[cell.column].id,
        };
    }

    fn copy_cell_text(&mut self, text: String, cx: &mut Context<Self>) {
        self.copy_task = None;
        crate::clipboard::write_text(text, cx);
        self.copy_finished(None, cx);
    }

    fn copy_finished(&mut self, failure: Option<String>, cx: &mut Context<Self>) {
        if let Some(message) = failure {
            self.notice = Some(Notice::CopyFailed(message));
            cx.notify();
        } else if matches!(self.notice, Some(Notice::CopyFailed(_))) {
            self.notice = None;
            cx.notify();
        }
    }

    fn copy(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = self.selection else {
            return;
        };
        let view = self.data.view().clone();
        let columns = self.data.column_ids(0..self.data.columns.len());
        let rows = selection.rows();
        let header = self.prefs.copy_includes_headers;
        let request = crate::clipboard::CopyRequest::begin(cx);
        self.copy_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { copy_tsv(&view, rows, &columns, header) })
                .await;
            this.update(cx, |table, cx| {
                if !request.is_current(cx) {
                    return;
                }
                let failure = match result {
                    Ok(text) => crate::clipboard::write_tsv(text, cx)
                        .err()
                        .map(String::from),
                    Err(error) => Some(crate::errors::clipboard_message(&error)),
                };
                table.copy_finished(failure, cx);
            })
            .ok();
        }));
    }
}

fn size_(w: f32, h: f32) -> Size<Pixels> {
    size(px(w), px(h))
}

mod find_style;
mod frame;
mod header;
mod input;
mod layout;
use layout::Layout;
mod accessibility;
mod controls;
#[cfg(test)]
mod gesture_tests;
mod paint;
mod query;
use query::Notice;
#[cfg(test)]
mod ui_fixture;
#[cfg(test)]
mod workflow_tests;

mod render;

#[cfg(test)]
mod edge_tests;
#[cfg(test)]
mod layout_tests;
