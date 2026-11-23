//! Document controls operate on stable column identities and the displayed view snapshot.
use gewu_core::{ColumnFilter, ColumnId, Filter};
use gpui_kit::{
    base::{self, input::InputState},
    *,
};

use super::{Cell, Selection, TableView};
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
    menu: Option<menus::MenuTarget>,
    replace_sort: Option<ColumnId>,
    sort_menu: Option<(ColumnId, Point<Pixels>)>,
    inspector: Option<Entity<Inspector>>,
    pub(super) inspector_visible: bool,
    export: Option<Entity<export::ExportController>>,
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
    id: ColumnId,
    name: SharedString,
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
    Header(ColumnId),
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
        confirm: Option<Vec<ColumnId>>,
    },
    Sort(Option<Point<Pixels>>),
    Filters(Point<Pixels>),
    Cell {
        anchor: Point<Pixels>,
        name: String,
        value: Option<SharedString>,
    },
    Export(export::ExportDraft),
}

impl TableView {
    pub(super) fn reveal_scatter_point(
        &mut self,
        hit: &crate::actions::RevealScatterPoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = &self.controls.inspector else {
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
        self.selection = Some(Selection::single(cell));
        self.reveal(cell);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(crate) fn initialize_inspector(&mut self, cx: &mut Context<Self>) {
        self.controls.inspector_visible = Preferences::get(cx).show_inspector;
        if self.controls.inspector_visible {
            self.inspector(cx);
        }
    }

    pub(crate) fn inherit_document_state(
        &mut self,
        previous: &Entity<Self>,
        cx: &mut Context<Self>,
    ) {
        let previous = &previous.read(cx).controls;
        self.controls.inspector_visible = previous.inspector_visible;
        self.controls.export = previous.export.clone();
        let settings = previous
            .inspector
            .as_ref()
            .map(|panel| panel.read(cx).settings());
        if let Some(settings) = settings {
            let snapshot = self.data.snapshot();
            let columns = self.data.columns.iter().cloned().collect();
            self.controls.inspector =
                Some(cx.new(|cx| Inspector::with_settings(snapshot, columns, settings, cx)));
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

    pub(super) fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.set_inspector_visible(!self.controls.inspector_visible, cx);
    }

    fn set_inspector_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.controls.menu = None;
        self.controls.drawer = None;
        self.controls.inspector_visible = visible;
        if visible {
            self.inspector(cx);
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

    pub(super) fn sync_panels(&mut self, cx: &mut Context<Self>) {
        self.controls.menu = None;
        let snapshot = self.data.snapshot();
        if let Some(panel) = &self.controls.inspector {
            panel.update(cx, |panel, cx| {
                panel.set_context(
                    snapshot.clone(),
                    self.data.columns.iter().cloned().collect(),
                    cx,
                )
            });
        }
        match &self.controls.drawer {
            Some(Drawer::Filter { editor, .. }) => {
                editor.update(cx, |panel, cx| panel.set_snapshot(snapshot, cx))
            }
            Some(Drawer::Cell { .. }) => self.controls.drawer = None,
            Some(Drawer::Filters(_)) if self.data.query().filters.is_empty() => {
                self.controls.drawer = None
            }
            _ => {}
        }
    }

    pub(super) fn close_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.controls.sort_menu.take().is_some() {
            cx.notify();
            return true;
        }
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

    fn filter_anchor(&self, column: ColumnId) -> Point<Pixels> {
        let layout = &self.layout;
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
                query.filters.sort_by_key(|f| f.column);
            }
        });
    }

    pub(super) fn panel(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut elements = Vec::new();
        if let Some(menu) = self.controls.menu {
            elements.push(self.menu_view(menu, cx));
        }
        if self.controls.inspector_visible {
            elements.extend(
                self.controls
                    .inspector
                    .iter()
                    .map(|p| p.clone().into_any_element()),
            );
        }
        let panel = match &self.controls.drawer {
            Some(Drawer::Filter { editor, .. }) => editor.clone().into_any_element(),
            Some(Drawer::Columns {
                search, confirm, ..
            }) => self.columns_panel(search, confirm.as_deref(), cx),
            Some(Drawer::Sort(_)) => self.sort_panel(cx),
            Some(Drawer::Filters(_)) => self.filter_summary(cx),
            Some(Drawer::Cell { name, value, .. }) => self.cell_panel(name, value.as_ref(), cx),
            Some(Drawer::Export(draft)) => self.export_panel(draft, cx),
            None => return elements,
        };
        let entity = cx.weak_entity();
        let target = match &self.controls.drawer {
            Some(
                Drawer::Sort(Some(at)) | Drawer::Filters(at) | Drawer::Cell { anchor: at, .. },
            ) => Some(PopoverTarget::Point(*at)),
            Some(Drawer::Filter { column, .. }) => Some(PopoverTarget::Header(*column)),
            _ => None,
        };
        let overlay = if let Some(target) = target {
            let popup = div()
                .id("table-popover")
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
                PopoverTarget::Header(column) => {
                    let table = cx.entity();
                    // Resolve against this frame's grid, after measuring and clipping its headers.
                    canvas(
                        move |_, window, cx| {
                            let at = table.read(cx).filter_anchor(column);
                            let mut popup = anchored()
                                .anchor(Anchor::TopCenter)
                                .position(at)
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
                .backdrop(div().size_full().bg(crate::theme::modal_backdrop()))
                .popup(div().occlude().child(panel))
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

    fn drawer(&self, title: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        let colors = base::Theme::global(cx).tokens.colors;
        div()
            .id("table-drawer")
            .flex_none()
            .w(px(318.))
            .max_w_full()
            .max_h(px(400.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_l_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .text_color(colors.surface_foreground)
            .text_sm()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(
                        button("close-table-drawer", "Close", false, cx).on_click(cx.listener(
                            |t, _, w, cx| {
                                t.close_panel(w, cx);
                            },
                        )),
                    ),
            )
    }
}

mod status;

#[cfg(test)]
mod tests;
