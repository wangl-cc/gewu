//! The table view: fixed-height virtualized rows and columns, painted with
//! one canvas. `render` fetches the visible text from Core synchronously and
//! builds a [`Frame`]; the canvas paints it. Nothing is drawn as a
//! placeholder: a cell is either text from Core or `null`. `TableView` is the shell around
//! the pure [`GridModel`] that owns selection, scrolling, drags and hover.

mod accessibility;
mod columns;
mod controls;
pub(crate) mod data;
mod find;
mod find_style;
mod frame;
mod grid;
mod header;
mod input;
mod layout;
mod paint;
mod query;
mod render;
mod scroll;
mod selection;

use std::{cell::RefCell, ops::Range, rc::Rc};

use columns::ColumnWidth;
pub(crate) use controls::ExportDraft;
use data::{TableData, TableQuery, copy_tsv};
use gewu_core::ColumnSort;
use gpui_kit::*;
use grid::{GridModel, HeaderHover};
use layout::Layout;
pub(crate) use query::CopyFailure;
use query::Notice;
use scroll::VerticalScroll;
use selection::{Cell, Selection};

use crate::{
    actions::{self, Navigate},
    preferences::Preferences,
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

/// What the document hosting a table reacts to.
pub(crate) enum TableEvent {
    /// The displayed snapshot or the visible columns changed.
    ContextChanged,
    SelectionChanged,
    /// The user confirmed an export of the captured view.
    Export(ExportDraft),
}

impl EventEmitter<TableEvent> for TableView {}

pub(crate) struct TableView {
    data: TableData,
    prefs: Preferences,
    _style_changes: Subscription,
    _window_activation: Subscription,
    grid: GridModel,
    /// A frame callback is queued to continue a selection drag past the body edge.
    edge_frame_pending: bool,
    scrollbar_timer: Option<Task<()>>,
    pub(crate) focus: FocusHandle,
    find: Entity<crate::table::find::FindController>,
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
    pub(crate) fn new(data: TableData, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let prefs = Preferences::get(cx);
        let style_changes = cx.observe_global::<Preferences>(|table, cx| {
            let prefs = Preferences::get(cx);
            if table.prefs.font_size != prefs.font_size {
                table.data.columns.invalidate_measurements();
            }
            table.prefs = prefs;
            table.grid.set_row_height(f64::from(table.prefs.row_height));
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
        let find = cx.new(|cx| {
            crate::table::find::FindController::new(data.view().clone(), scope, window, cx)
        });
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
            grid: GridModel::new(scroll),
            pending_layout: Rc::default(),
            _find_viewport: cx.subscribe_self(|table, _: &layout::FindViewportChanged, cx| {
                table.refresh_highlights(cx);
            }),
            edge_frame_pending: false,
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

    /// The search field the document shows in its title bar.
    pub(crate) fn find_bar(&self) -> &Entity<crate::table::find::FindController> {
        &self.find
    }

    pub(crate) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_grid_layout(cx);
        self.find
            .update(cx, |find, cx| find.focus_input(window, cx));
        self.refresh_highlights(cx);
    }

    pub(crate) fn can_step_find(&self, cx: &App) -> bool {
        self.find.read(cx).can_navigate()
    }

    pub(crate) fn step_find(&mut self, previous: bool, cx: &mut Context<Self>) {
        self.find
            .update(cx, |find, cx| find.find_match(previous, cx));
    }

    /// Close the innermost open popover, or else the search, returning focus to the grid.
    /// Returns false when nothing was open.
    pub(crate) fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.close_panel(window, cx) {
            return true;
        }
        if !self.find.read(cx).is_open() {
            return false;
        }
        self.find.update(cx, |find, cx| find.close(window, cx));
        window.focus(&self.focus, cx);
        true
    }

    pub(crate) fn snapshot(&self) -> gewu_app::AppliedView {
        self.data.snapshot()
    }

    /// Visible columns in display order.
    pub(crate) fn visible_columns(&self) -> Vec<gewu_core::ColumnInfo> {
        self.data.columns.iter().cloned().collect()
    }

    /// Whether the applied or the requested query filters or sorts any column.
    pub(crate) fn has_conditions(&self) -> bool {
        [self.data.query().as_ref(), self.requested_query()]
            .iter()
            .any(|query| !query.filters.is_empty() || !query.sorts.is_empty())
    }

    pub(crate) fn has_requested_filters(&self) -> bool {
        !self.requested_query().filters.is_empty()
    }

    /// The selected column, once the displayed query has settled.
    pub(crate) fn analyzable_column(&self) -> Option<gewu_core::ColumnId> {
        self.selected_column()
            .filter(|_| !self.data.queries().is_running())
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
            self.grid.scroll(),
            self.grid.horizontal(),
            auto_hide_scrollbars,
        )
    }

    #[cfg(test)]
    fn column_width(&self, column: usize) -> f32 {
        self.data.columns.width(column)
    }

    pub(crate) fn scroll_to_row(&mut self, row: u64) {
        self.grid.scroll_to_row(row);
    }

    #[cfg(feature = "ui-capture")]
    pub(crate) fn capture_cell_point(&self, column: &str, row: u64) -> Option<Point<Pixels>> {
        let index =
            (0..self.data.columns.len()).find(|&i| self.data.columns.entry(i).label == column)?;
        let left = self.layout.column_left(index)?;
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
        let left = self.layout.column_left(index)?;
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

    pub(crate) fn reveal(&mut self, cell: Cell) {
        self.grid.reveal(cell, &self.data.columns, &self.layout);
    }

    fn copy_cell_text(&mut self, text: String, cx: &mut Context<Self>) {
        self.copy_task = None;
        crate::platform::clipboard::write_text(text, cx);
        self.copy_finished(None, cx);
    }

    fn copy_finished(&mut self, failure: Option<CopyFailure>, cx: &mut Context<Self>) {
        if let Some(failure) = failure {
            self.notice = Some(Notice::CopyFailed(failure));
            cx.notify();
        } else if matches!(self.notice, Some(Notice::CopyFailed(_))) {
            self.notice = None;
            cx.notify();
        }
    }

    fn copy(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = self.grid.selection() else {
            return;
        };
        let view = self.data.view().clone();
        let columns = self.data.column_ids(0..self.data.columns.len());
        let rows = selection.rows();
        let header = self.prefs.copy_includes_headers;
        let request = crate::platform::clipboard::CopyRequest::begin(cx);
        self.copy_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { copy_tsv(&view, rows, &columns, header) })
                .await;
            this.update(cx, |table, cx| {
                if !request.is_current(cx) {
                    return;
                }
                let failure = match result {
                    Ok(text) => crate::platform::clipboard::write_tsv(text, cx)
                        .err()
                        .map(CopyFailure::Pasteboard),
                    Err(error) => Some(CopyFailure::Encode(error)),
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

#[cfg(test)]
mod edge_tests;
#[cfg(test)]
mod gesture_tests;
#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod ui_fixture;
#[cfg(test)]
mod workflow_tests;
