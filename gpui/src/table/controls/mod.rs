//! Document controls operate on stable column identities and the displayed view snapshot.
use std::{cell::Cell as ValueCell, collections::HashSet, rc::Rc};

use gewu_core::{ColumnFilter, ColumnId, Filter, SourceRow, ViewRow};
use gpui_kit::{
    base::{self, input::InputState},
    prelude::FluentBuilder,
    *,
};

use super::{Cell, Layout, Selection, TableView};
use crate::{
    panels::{FilterEditor, FilterEvent, Inspector, button},
    preferences::Preferences,
};

mod columns;
mod export;
mod menus;
mod sorting;

#[derive(Default)]
pub(super) struct Controls {
    drawer: Option<Drawer>,
    menu: Option<menus::OpenMenu>,
    /// Column visibility commits with the query that clears these columns' conditions.
    pub(super) pending_hides: HashSet<ColumnId>,
    panel_focus: Option<FocusHandle>,
    inspector: Option<Entity<Inspector>>,
    marked_point: Option<MarkedPoint>,
    export: Option<Entity<export::ExportController>>,
}

struct MarkedPoint {
    chart: EntityId,
    column: ColumnId,
    row: SourceRow,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Order {
    Columns,
    Sorting,
}

#[derive(Clone)]
struct ColumnDrag {
    document: EntityId,
    order: Order,
    index: usize,
    id: ColumnId,
    name: SharedString,
}
impl ColumnDrag {
    fn insertion_indicator(
        &self,
        style: StyleRefinement,
        dragged: &Self,
        color: Hsla,
    ) -> StyleRefinement {
        if dragged.document != self.document
            || dragged.order != self.order
            || dragged.index == self.index
        {
            return style;
        }
        if dragged.index < self.index {
            style.border_b_2().border_color(color)
        } else {
            style.border_t_2().border_color(color)
        }
    }
}

impl Render for ColumnDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = base::Theme::global(cx).tokens.colors;
        div()
            .px_2()
            .py_1()
            .bg(colors.surface)
            .text_color(colors.foreground)
            .child(self.name.clone())
    }
}

enum PopoverTarget {
    Point(Point<Pixels>),
    Header(Rc<ValueCell<Point<Pixels>>>),
}

enum Drawer {
    Filter {
        column: ColumnId,
        editor: Entity<FilterEditor>,
        _events: Subscription,
    },
    Columns {
        search: Entity<InputState>,
        _events: Subscription,
        confirm: Option<columns::HideConfirmation>,
        list: columns::ColumnListScroll,
    },
    Sort {
        summary: bool,
    },
    Filters,
    Cell {
        anchor: Point<Pixels>,
        name: String,
        value: Option<SharedString>,
    },
    Export(export::ExportDraft),
}

impl TableView {
    pub(super) fn set_selection(&mut self, selection: Option<Selection>, cx: &mut Context<Self>) {
        self.selection = selection;
        let Some(mark) = &self.controls.marked_point else {
            return;
        };
        // Reordering visible columns moves the cell's coordinates but keeps its identity.
        if selection.is_some_and(|s| {
            s.anchor == s.head
                && self
                    .data
                    .columns
                    .get(s.head.column)
                    .is_some_and(|col| col.id == mark.column)
                && self.data.view().source_row_at(ViewRow(s.head.row)) == Some(mark.row)
        }) {
            return;
        }
        let chart = mark.chart;
        self.controls.marked_point = None;
        if let Some(panel) = &self.controls.inspector {
            panel.update(cx, |panel, cx| panel.mark_scatter(chart, None, cx));
        }
    }

    pub(super) fn reveal_scatter_point(
        &mut self,
        hit: &crate::actions::RevealScatterPoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_grid_layout(cx);
        let Some(panel) = self.controls.inspector.clone() else {
            return;
        };
        let Some(id) = panel.read(cx).plotted_column(hit.chart) else {
            return;
        };
        let Some(row) = self.data.view().display_row_for_source(hit.source_row) else {
            return;
        };
        let Some(cell) = self.data.display_cell(row, id) else {
            return;
        };
        self.set_selection(Some(Selection::single(cell)), cx);
        self.controls.marked_point = Some(MarkedPoint {
            chart: hit.chart,
            column: id,
            row: hit.source_row,
        });
        panel.update(cx, |panel, cx| {
            panel.mark_scatter(hit.chart, Some(hit.source_row), cx)
        });
        self.scroll.center_if_outside(cell.row);
        self.reveal(cell);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(crate) fn initialize_inspector(&mut self, cx: &mut Context<Self>) {
        if Preferences::get(cx).show_inspector {
            self.inspector(cx);
        }
    }

    pub(crate) fn inherit_document_state(
        &mut self,
        previous: &Entity<Self>,
        cx: &mut Context<Self>,
    ) {
        let previous = &previous.read(cx).controls;
        self.controls.export = previous.export.clone();
        if let Some(panel) = &previous.inspector {
            let visible = panel.read(cx).is_visible();
            let settings = panel.read(cx).settings();
            let snapshot = self.data.snapshot();
            let columns = self.data.columns.iter().cloned().collect();
            self.controls.inspector = Some(
                cx.new(|cx| Inspector::with_settings(snapshot, columns, settings, visible, cx)),
            );
        }
    }

    fn inspector(&mut self, cx: &mut Context<Self>) -> Entity<Inspector> {
        self.controls
            .inspector
            .get_or_insert_with(|| {
                let snapshot = self.data.snapshot();
                cx.new(|cx| {
                    Inspector::new(snapshot, self.data.columns.iter().cloned().collect(), cx)
                })
            })
            .clone()
    }

    pub(super) fn inspector_visible(&self, cx: &App) -> bool {
        self.controls
            .inspector
            .as_ref()
            .is_some_and(|panel| panel.read(cx).is_visible())
    }

    pub(super) fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.set_inspector_visible(!self.inspector_visible(cx), cx);
    }

    fn set_inspector_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.controls.menu = None;
        self.controls.drawer = None;
        if visible {
            self.inspector(cx);
        }
        if let Some(panel) = &self.controls.inspector {
            panel.update(cx, |panel, cx| panel.set_visible(visible, cx));
        }
        Preferences::update(cx, |p| p.show_inspector = visible);
        cx.notify();
    }

    pub(super) fn show_chart(&mut self, cx: &mut Context<Self>) {
        self.set_inspector_visible(true, cx);
        self.inspector(cx).update(cx, |panel, cx| {
            panel.set_page(crate::panels::InspectorPage::Chart, cx)
        });
    }

    pub(super) fn analyze_column(&mut self, column: ColumnId, cx: &mut Context<Self>) {
        self.set_inspector_visible(true, cx);
        self.inspector(cx)
            .update(cx, |panel, cx| panel.inspect(column, cx));
    }

    /// Row or column coordinates changed. Drop interactions tied to the old grid.
    pub(super) fn grid_changed(&mut self, cx: &mut Context<Self>) {
        self.reset_grid_interaction();
        self.controls.menu = None;
        if matches!(self.controls.drawer, Some(Drawer::Cell { .. })) {
            self.controls.drawer = None;
        }
        if let Some(panel) = &self.controls.inspector {
            panel.update(cx, |panel, cx| {
                panel.set_context(
                    self.data.snapshot(),
                    self.data.columns.iter().cloned().collect(),
                    cx,
                )
            });
        }
    }

    pub(super) fn refresh_query_drawer(&mut self, cx: &mut Context<Self>) {
        match &self.controls.drawer {
            Some(Drawer::Filter { editor, .. }) => {
                editor.update(cx, |panel, cx| panel.set_snapshot(self.data.snapshot(), cx))
            }
            Some(Drawer::Sort { summary: true }) if self.data.query().sorts.is_empty() => {
                self.controls.drawer = None
            }
            Some(Drawer::Filters) if self.data.query().filters.is_empty() => {
                self.controls.drawer = None
            }
            _ => {}
        }
    }

    fn focus_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self
            .controls
            .panel_focus
            .get_or_insert_with(|| cx.focus_handle());
        window.focus(handle, cx);
    }

    pub(super) fn close_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.controls.menu.take().is_none() && self.controls.drawer.take().is_none() {
            return false;
        }
        window.focus(&self.focus, cx);
        cx.notify();
        true
    }

    pub(super) fn selected_column(&self) -> Option<ColumnId> {
        let index = self.selection.map_or(0, |s| s.head.column);
        self.data.columns.get(index).map(|column| column.id)
    }

    pub(in crate::table) fn filter_column(
        &mut self,
        id: ColumnId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(column) = self.data.columns.iter().find(|c| c.id == id).cloned() else {
            return;
        };
        let snapshot = self.data.snapshot();
        let applied = self
            .requested_query()
            .filters
            .iter()
            .find(|f| f.column == id)
            .map(|f| f.filter.clone());
        let editor = cx.new(|cx| FilterEditor::new(snapshot, column, applied, window, cx));
        let events = cx.subscribe_in(
            &editor,
            window,
            |table, _, event: &FilterEvent, window, cx| {
                table.close_panel(window, cx);
                if let FilterEvent::Apply(id, filter) = event {
                    table.set_filter(*id, filter.clone(), cx);
                }
            },
        );
        self.controls.menu = None;
        self.controls.drawer = Some(Drawer::Filter {
            column: id,
            editor,
            _events: events,
        });
        cx.notify();
    }

    pub(super) fn filter_panel_column(&self) -> Option<ColumnId> {
        match &self.controls.drawer {
            Some(Drawer::Filter { column, .. }) => Some(*column),
            _ => None,
        }
    }

    pub(super) fn filter_anchor(&self, column: ColumnId, layout: &Layout) -> Point<Pixels> {
        let center = layout
            .clipped_columns()
            .find(|&(index, ..)| self.data.columns.get(index).is_some_and(|c| c.id == column))
            .map(|(_, left, width)| left + width / 2.)
            .unwrap_or_else(
                || match (layout.columns.last(), self.data.columns.position(column)) {
                    (Some(&(last, ..)), Some(index)) if index.index() > last => layout.body_right(),
                    _ => layout.gutter,
                },
            );
        layout.bounds.origin + point(px(center), px(super::HEADER_HEIGHT))
    }

    pub(super) fn set_filter(
        &mut self,
        id: ColumnId,
        filter: Option<Filter>,
        cx: &mut Context<Self>,
    ) {
        self.edit_query(cx, |query| {
            query.filters.retain(|f| f.column != id);
            if let Some(filter) = filter {
                query.filters.push(ColumnFilter { column: id, filter });
            }
        });
    }

    pub(super) fn panel(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        filter_anchor: Option<Rc<ValueCell<Point<Pixels>>>>,
    ) -> Vec<AnyElement> {
        let mut elements = Vec::new();
        if let Some(menu) = &self.controls.menu {
            elements.push(menu.element());
        }
        if let Some(panel) = &self.controls.inspector
            && panel.read(cx).is_visible()
        {
            elements.push(panel.clone().into_any_element());
        }
        let panel = match &self.controls.drawer {
            Some(Drawer::Filter { editor, .. }) => editor.clone().into_any_element(),
            Some(Drawer::Columns {
                search,
                confirm,
                list,
                ..
            }) => self.columns_panel(search, confirm.as_ref(), list, window, cx),
            Some(Drawer::Sort { summary: false }) => self.sort_panel(window, cx),
            Some(Drawer::Cell { name, value, .. }) => {
                self.cell_panel(name, value.as_ref(), window, cx)
            }
            Some(Drawer::Export(draft)) => self.export_panel(draft, window, cx),
            // The status-bar triggers own the geometry of their popups.
            Some(Drawer::Filters | Drawer::Sort { summary: true }) | None => return elements,
        };
        let entity = cx.weak_entity();
        let target = match &self.controls.drawer {
            Some(Drawer::Cell { anchor, .. }) => Some(PopoverTarget::Point(*anchor)),
            _ => filter_anchor.map(PopoverTarget::Header),
        };
        let overlay = if let Some(target) = target {
            let popup = div()
                .id("table-popover")
                .when(matches!(target, PopoverTarget::Point(_)), |popup| {
                    popup.when_some(self.controls.panel_focus.as_ref(), |popup, handle| {
                        popup.track_focus(handle)
                    })
                })
                .occlude()
                .on_mouse_down_out(cx.listener(|table, _, window, cx| {
                    table.close_panel(window, cx);
                }))
                .child(panel);
            let popup = match target {
                PopoverTarget::Point(at) => anchored()
                    .position(at)
                    .snap_to_window()
                    .child(popup)
                    .into_any_element(),
                PopoverTarget::Header(anchor) => {
                    // Resolve against this frame's grid, after measuring and clipping its headers.
                    canvas(
                        move |_, window, cx| {
                            let mut popup = anchored()
                                .anchor(Anchor::TopCenter)
                                .position(anchor.get())
                                .snap_to_window()
                                .child(popup)
                                .into_any_element();
                            popup.prepaint_as_root(
                                Point::default(),
                                window.viewport_size().into(),
                                window,
                                cx,
                            );
                            popup
                        },
                        |_, mut popup, window, cx| popup.paint(window, cx),
                    )
                    .absolute()
                    .into_any_element()
                }
            };
            deferred(popup).with_priority(8).into_any_element()
        } else {
            base::Dialog::new(cx)
                .accessibility_label("Table options")
                .when_some(self.controls.panel_focus.clone(), |dialog, handle| {
                    dialog.focus_handle(handle)
                })
                .backdrop(div().size_full().bg(crate::theme::modal_backdrop()))
                .popup(div().occlude().child(panel))
                .on_ok({
                    let table = entity.clone();
                    move |_, window, cx| {
                        table
                            .update(cx, |table, cx| match &table.controls.drawer {
                                Some(Drawer::Columns {
                                    confirm: Some(_), ..
                                }) => {
                                    table.confirm_hide(window, cx);
                                    false
                                }
                                Some(Drawer::Columns { search, .. })
                                    if search.focus_handle(cx).is_focused(window) =>
                                {
                                    false
                                }
                                Some(Drawer::Export(_)) => {
                                    table.choose_export_path(cx);
                                    window.focus(&table.focus, cx);
                                    false
                                }
                                _ => true,
                            })
                            .unwrap_or(true)
                    }
                })
                .on_cancel({
                    let table = entity.clone();
                    move |_, window, cx| {
                        table
                            .update(cx, |table, cx| {
                                if let Some(Drawer::Columns {
                                    confirm: Some(_), ..
                                }) = &table.controls.drawer
                                {
                                    table.cancel_hide(window, cx);
                                    false
                                } else {
                                    true
                                }
                            })
                            .unwrap_or(true)
                    }
                })
                .on_open_change(move |open, _, window, cx| {
                    if !open {
                        let _ = entity.update(cx, |t, cx| t.close_panel(window, cx));
                    }
                })
                .into_any_element()
        };
        elements.push(overlay);
        elements
    }

    fn drawer(
        &self,
        title: impl Into<SharedString>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> base::ObservedElement<Stateful<Div>> {
        let colors = base::Theme::global(cx).tokens.colors;
        let title = title.into();
        div()
            .id("table-drawer")
            .test_support()
            .flex_none()
            .w(px(368.).min((window.viewport_size().width - px(12.)).max(px(0.))))
            .max_h(px(400.).min((window.viewport_size().height - px(40.)).max(px(0.))))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .rounded(px(10.))
            .border_1()
            .border_color(colors.border)
            .shadow_md()
            .bg(colors.surface)
            .text_color(colors.surface_foreground)
            .text_sm()
            .child(
                div()
                    .id("table-drawer-title")
                    .role(Role::Label)
                    .aria_label(title.clone())
                    .test_support()
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
    }

    fn close_button(&self, label: &'static str, cx: &mut Context<Self>) -> base::Button {
        button("close-table-drawer", label, false, cx).on_click(cx.listener(|t, _, w, cx| {
            t.close_panel(w, cx);
        }))
    }
}

const ICON_BUTTON_SIZE: f32 = 22.;

fn icon_button(
    id: impl Into<ElementId>,
    label: &'static str,
    glyph: &'static str,
    cx: &App,
) -> base::Button {
    let colors = base::Theme::global(cx).tokens.colors;
    crate::panels::button_base(id)
        .accessibility_label(label)
        .size(px(ICON_BUTTON_SIZE))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .text_color(colors.muted_foreground)
        .hover(move |s| s.bg(colors.accent))
        .child(glyph)
}

fn footer() -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
}

mod status;

#[cfg(test)]
mod tests;
