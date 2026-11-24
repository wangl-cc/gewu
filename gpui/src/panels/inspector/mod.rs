//! The selection inspected here is independent of the keyboard selection in the table.
use std::sync::Arc;

use gewu_app::AppliedView;
use gewu_core::{
    AnalysisField, AxisScale, ColumnId, ColumnInfo, LengthSummary, NumericStatistics,
    RangeHistogram, ScatterResult,
};
use gpui_kit::{
    base::{self, TestSupportExt},
    *,
};

use super::{
    button,
    charts::{self, ScatterChart},
    segment, segmented_tabs,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Column,
    Chart,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Picker {
    Column,
    X,
    Y,
}

mod sections;
use sections::{ColumnResults, SectionData};

enum Loaded {
    Column(ColumnResults),
    Chart(Arc<ScatterResult>),
}
enum Showing {
    Empty,
    Loading,
    Column(ColumnResults),
    Chart(Entity<ScatterChart>),
    Failed(gewu_core::Error),
}

#[derive(Clone, Copy)]
pub(crate) struct InspectorSettings {
    width: f32,
    expansion: [Option<bool>; 6],
    page: Page,
    bins: u32,
    bin_scale: AxisScale,
    log_counts: bool,
    x_scale: AxisScale,
    y_scale: AxisScale,
    limit: u32,
}

impl Default for InspectorSettings {
    fn default() -> Self {
        Self {
            width: 300.,
            expansion: [None; 6],
            page: Page::Column,
            bins: 40,
            bin_scale: AxisScale::Linear,
            log_counts: false,
            x_scale: AxisScale::Linear,
            y_scale: AxisScale::Linear,
            limit: 2048,
        }
    }
}

pub(crate) struct Inspector {
    visible: bool,
    snapshot: AppliedView,
    columns: Vec<ColumnInfo>,
    selected: Option<ColumnId>,
    x: Option<ColumnId>,
    y: Option<ColumnId>,
    settings: InspectorSettings,
    chart_options_open: bool,
    showing: Showing,
    task: Option<Task<()>>,
}

impl Inspector {
    pub(crate) fn new(
        snapshot: AppliedView,
        visible_columns: Vec<ColumnInfo>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_settings(
            snapshot,
            visible_columns,
            InspectorSettings::default(),
            true,
            cx,
        )
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if visible && matches!(self.showing, Showing::Empty) {
            self.load(cx);
        } else if !visible && matches!(self.showing, Showing::Loading) {
            self.task = None;
            self.showing = Showing::Empty;
        }
        cx.notify();
    }

    pub(crate) fn settings(&self) -> InspectorSettings {
        self.settings
    }

    pub(crate) fn plotted_column(&self, chart_id: EntityId) -> Option<ColumnId> {
        match &self.showing {
            Showing::Chart(chart) if chart.entity_id() == chart_id => self.y,
            _ => None,
        }
    }

    pub(crate) fn mark_scatter(
        &self,
        chart_id: EntityId,
        row: Option<gewu_core::SourceRow>,
        cx: &mut App,
    ) {
        if let Showing::Chart(chart) = &self.showing
            && chart.entity_id() == chart_id
        {
            // A scatter click dispatches the table action while the chart is still borrowed.
            let chart = chart.downgrade();
            cx.defer(move |cx| {
                let _ = chart.update(cx, |chart, cx| chart.mark(row, cx));
            });
        }
    }

    pub(crate) fn with_settings(
        snapshot: AppliedView,
        visible_columns: Vec<ColumnInfo>,
        settings: InspectorSettings,
        visible: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut inspector = Self {
            visible,
            snapshot,
            columns: visible_columns,
            selected: None,
            x: None,
            y: None,
            settings,
            chart_options_open: false,
            showing: Showing::Empty,
            task: None,
        };
        inspector.repair_selection();
        inspector.load(cx);
        inspector
    }

    pub(crate) fn set_context(
        &mut self,
        snapshot: AppliedView,
        visible_columns: Vec<ColumnInfo>,
        cx: &mut Context<Self>,
    ) {
        let same_rows = Arc::ptr_eq(self.snapshot.filtered(), snapshot.filtered());
        let old = (self.selected, self.x, self.y);
        self.snapshot = snapshot;
        self.columns = visible_columns;
        self.repair_selection();
        if !same_rows || old != (self.selected, self.x, self.y) {
            self.load(cx);
        }
        cx.notify();
    }

    fn repair_selection(&mut self) {
        if self.info(self.selected).is_none() {
            self.selected = self.columns.first().map(|c| c.id);
        }
        if self.info(self.x).is_none_or(|c| !c.capabilities.scatter) {
            self.x = None;
        }
        if self.info(self.y).is_none_or(|c| !c.capabilities.scatter) {
            self.y = self
                .columns
                .iter()
                .find(|c| c.capabilities.scatter)
                .map(|c| c.id);
        }
    }

    pub(crate) fn set_page(&mut self, page: Page, cx: &mut Context<Self>) {
        if self.settings.page != page {
            self.settings.page = page;
            self.load(cx);
            cx.notify();
        }
    }

    pub(crate) fn inspect(&mut self, column: ColumnId, cx: &mut Context<Self>) {
        if self.info(Some(column)).is_some() {
            self.settings.page = Page::Column;
            self.selected = Some(column);
            self.load(cx);
            cx.notify();
        }
    }

    fn info(&self, id: Option<ColumnId>) -> Option<&ColumnInfo> {
        let id = id?;
        self.columns.iter().find(|col| col.id == id)
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        self.showing = Showing::Empty;
        if !self.visible {
            return;
        }
        let page = self.settings.page;
        let desired = if page == Page::Chart {
            self.y
        } else {
            self.selected
        };
        let Some(col) = self.info(desired).cloned() else {
            return;
        };
        let snapshot = self.snapshot.clone();
        let x = self.x;
        let x_name = self.info(x).map(|column| column.name.clone());
        let y_name = col.name.clone();
        let limit = self.settings.limit;
        let settings = self.settings;
        let bin_scale = if col.capabilities.logarithmic {
            self.settings.bin_scale
        } else {
            AxisScale::Linear
        };
        let x_scale = if self.info(x).is_some_and(|c| c.capabilities.logarithmic) {
            self.settings.x_scale
        } else {
            AxisScale::Linear
        };
        let y_scale = if col.capabilities.logarithmic {
            self.settings.y_scale
        } else {
            AxisScale::Linear
        };
        self.showing = Showing::Loading;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    match page {
                        Page::Column => Ok(Loaded::Column(ColumnResults::load(
                            &snapshot, &col, settings, bin_scale,
                        ))),
                        Page::Chart => snapshot
                            .analyses()
                            .scatter(x, col.id, limit, x_scale, y_scale)
                            .map(Loaded::Chart),
                    }
                })
                .await;
            let _ = this.update(cx, |inspector, cx| {
                inspector.task = None;
                inspector.showing = match result {
                    Ok(Loaded::Column(result)) => Showing::Column(result),
                    Ok(Loaded::Chart(result)) => Showing::Chart(cx.new(|cx| {
                        ScatterChart::new(result, x_scale, y_scale, x_name.as_deref(), &y_name, cx)
                    })),
                    Err(error) => Showing::Failed(error),
                };
                cx.notify();
            });
        }));
    }

    fn select(&mut self, picker: Picker, column: Option<ColumnId>, cx: &mut Context<Self>) {
        match picker {
            Picker::Column => self.selected = column,
            Picker::X => self.x = column,
            Picker::Y => self.y = column,
        }
        self.load(cx);
        cx.notify();
    }
}

mod column;
pub(super) mod controls;

struct ResizeInspector;

impl Render for Inspector {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = base::Theme::global(cx).tokens.colors;
        let resize = base::resize_handle("inspector-resize", Axis::Horizontal)
            .inside(base::HandleEdge::Leading)
            .on_drag(ResizeInspector, |_, _, _, cx| cx.new(|_| Empty));
        div()
            .id("inspector")
            .relative()
            .h_full()
            .w(px(self.settings.width))
            .max_w(relative(0.60))
            .flex_none()
            .bg(colors.surface)
            .on_drag_move::<ResizeInspector>(cx.listener(
                |panel, event: &DragMoveEvent<ResizeInspector>, _, cx| {
                    panel.settings.width = (event.bounds.right() - event.event.position.x)
                        .as_f32()
                        .clamp(240., 420.);
                    cx.notify();
                },
            ))
            .child(self.contents(window, cx))
            .child(resize)
            .test_support()
    }
}

impl Inspector {
    fn contents(&self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let colors = base::Theme::global(cx).tokens.colors;
        let mut pane = div()
            .id("inspector-scroll")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_4()
            .p_3()
            .text_color(colors.surface_foreground)
            .text_size(px(13.));
        let mut tabs = segmented_tabs("inspector-tabs", "Inspector", cx);
        for (page, label) in [(Page::Column, "Column"), (Page::Chart, "Chart")] {
            let selected = self.settings.page == page;
            let tab = base::Tab::new(label)
                .selected(selected)
                .accessibility_label(label)
                .on_click(cx.listener(move |panel, _, _, cx| panel.set_page(page, cx)));
            tabs = tabs.child(segment(tab, label, selected, cx));
        }
        pane = pane.child(tabs);
        if self.columns.is_empty() {
            return pane.child("No visible columns");
        }
        match self.settings.page {
            Page::Column => {
                pane = pane.child(self.picker(
                    Picker::Column,
                    "Inspect column",
                    self.selected,
                    window,
                    cx,
                ))
            }
            Page::Chart => {
                pane = pane
                    .child(self.picker(Picker::X, "X axis", self.x, window, cx))
                    .child(self.picker(Picker::Y, "Y axis", self.y, window, cx))
                    .child(self.chart_options(cx))
            }
        }
        match &self.showing {
            Showing::Empty => pane.child("No plottable columns"),
            Showing::Loading => pane.child("Loading analysis…"),
            Showing::Failed(error) => pane
                .child(
                    div()
                        .text_color(colors.destructive)
                        .child(crate::errors::message(error)),
                )
                .child(
                    button("retry-analysis", "Retry", false, cx).on_click(cx.listener(
                        |panel, _, _, cx| {
                            panel.load(cx);
                            cx.notify();
                        },
                    )),
                ),
            Showing::Column(results) => {
                if let Some(col) = self.info(self.selected) {
                    pane.child(self.column_view(col, results, cx))
                } else {
                    pane
                }
            }
            Showing::Chart(chart) => pane.child(chart.clone()),
        }
    }
}
