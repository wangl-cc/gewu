use gpui_kit::prelude::FluentBuilder;

use super::*;

impl TableView {
    pub(crate) fn document_titlebar(name: SharedString, window: &mut Window, cx: &mut App) -> Div {
        let colors = crate::theme::GewuPalette::get(cx);
        let title = div()
            .id("document-title")
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .justify_center()
            .child(
                div()
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(name),
            );
        #[cfg(target_os = "macos")]
        let title = {
            let pressed = window.use_state(cx, |_, _| false);
            title
                .pl(if window.is_fullscreen() {
                    px(12.)
                } else {
                    px(90.)
                })
                .on_mouse_down(
                    MouseButton::Left,
                    window.listener_for(&pressed, |pressed, ev: &MouseDownEvent, w, _| {
                        *pressed = ev.click_count == 1;
                        if ev.click_count == 2 {
                            w.titlebar_double_click();
                        }
                    }),
                )
                .on_mouse_down_out(
                    window.listener_for(&pressed, |pressed, _, _, _| *pressed = false),
                )
                .on_mouse_move(window.listener_for(
                    &pressed,
                    |pressed, ev: &MouseMoveEvent, w, _| {
                        if std::mem::take(pressed) && ev.pressed_button == Some(MouseButton::Left) {
                            w.start_window_move();
                        }
                    },
                ))
        };
        #[cfg(not(target_os = "macos"))]
        let title = {
            let _ = window;
            title.pl_3()
        };
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .h(px(52.))
            .pr_3()
            .bg(colors.surface)
            .border_b_1()
            .border_color(colors.border)
            .text_sm()
            .child(title.test_support())
    }
}

fn formatting_alert(message: SharedString, cx: &App) -> AnyElement {
    let colors = crate::theme::GewuPalette::get(cx);
    div()
        .relative()
        .size_full()
        .child(
            div()
                .id("table-formatting-error")
                .role(Role::Alert)
                .aria_label(message.clone())
                .absolute()
                .top(px(HEADER_HEIGHT))
                .left_0()
                .right_0()
                .px_2()
                .py_1()
                .bg(colors.surface)
                .text_color(colors.danger_text)
                .child(crate::ui::errors::inline_warning(message)),
        )
        .into_any_element()
}

impl Render for TableView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dragging = !matches!(self.drag, Drag::None);
        let entity = cx.entity();
        let publish = entity.downgrade();
        let pending_layout = Rc::clone(&self.pending_layout);
        let filter_anchor = self.filter_panel_column().map(|column| {
            let current = self.filter_anchor(column, &self.layout);
            (column, Rc::new(std::cell::Cell::new(current)))
        });
        let grid_anchor = filter_anchor.clone();
        let table = accessibility::grid(self, window)
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|table, _: &base::input::Copy, window, cx| table.copy(window, cx)),
            )
            .key_context("GewuGrid")
            .on_action(cx.listener(Self::navigate))
            .on_action(cx.listener(|t, _: &base::input::SelectAll, _, cx| {
                t.set_selection(Selection::all(t.data.row_count(), t.data.columns.len()), cx);
                cx.notify();
            }))
            .on_action(cx.listener(|t, _: &actions::FilterSelection, _, cx| t.filter_selected(cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::context_menu))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_hover(cx.listener(|table, hovering, _, cx| {
                if !hovering
                    && (table.header_hover.is_some()
                        || table.thumb_hover.is_some()
                        || table.body_pointer.is_some())
                {
                    table.header_hover = None;
                    table.thumb_hover = None;
                    table.body_pointer = None;
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_scroll(cx.listener(Self::scroll_wheel))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let (frame, mut overlays, pending) = entity.update(cx, |table, cx| {
                            let geometry = table.prepare_grid_layout(bounds, window, cx);
                            if let Some((column, anchor)) = &grid_anchor {
                                anchor.set(table.filter_anchor(*column, &geometry.layout));
                            }
                            let frame = table.build_frame(Rc::clone(&geometry.layout), cx);
                            let semantics = window
                                .is_a11y_active()
                                .then(|| accessibility::rows(table, &frame, cx));
                            let header = table.hover_target(&frame.layout);
                            let error = frame
                                .formatting_error
                                .clone()
                                .map(|m| formatting_alert(m, cx));
                            let pending = table.grid_layout_changed(&geometry).then_some(geometry);
                            (frame, [semantics, header, error], pending)
                        });
                        for overlay in overlays.iter_mut().flatten() {
                            overlay.prepaint_as_root(bounds.origin, bounds.size.into(), window, cx);
                        }
                        (frame, overlays, pending)
                    },
                    move |bounds, (frame, overlays, pending), window, cx| {
                        if dragging {
                            // The grid owns a drag until mouse-up, including motion over the
                            // toolbar or status bar.
                            let table = publish.clone();
                            window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
                                if phase == DispatchPhase::Capture
                                    && ev.pressed_button == Some(MouseButton::Left)
                                    && !bounds.contains(&ev.position)
                                {
                                    _ = table
                                        .update(cx, |table, cx| table.mouse_move(ev, window, cx));
                                }
                            });
                        }
                        paint::paint(frame, window, cx);
                        for mut overlay in overlays.into_iter().flatten() {
                            overlay.paint(window, cx);
                        }
                        let changed = pending.is_some();
                        let queued = pending_layout.replace(pending).is_some();
                        if changed && !queued {
                            // Publish only the frame that actually painted, outside the draw cycle.
                            cx.defer(move |cx| {
                                let _ = publish.update(cx, |table, cx| {
                                    table.commit_grid_layout(cx);
                                });
                            });
                        }
                    },
                )
                .size_full(),
            );
        let colors = crate::theme::GewuPalette::get(cx);
        let table = if self.data.row_count() == 0
            && let Some(filter) = self.data.query().filters.first()
        {
            let column = filter.column;
            div()
                .relative()
                .flex()
                .flex_1()
                .min_w_0()
                .child(table)
                .child(
                    div()
                        .absolute()
                        .top(px(HEADER_HEIGHT))
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors.muted_foreground)
                                .child("No rows match the current filters"),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    crate::ui::button(
                                        "empty-edit-filter",
                                        "Edit filter…",
                                        false,
                                        cx,
                                    )
                                    .on_click(cx.listener(
                                        move |table, _, window, cx| {
                                            table.filter_column(column, window, cx)
                                        },
                                    )),
                                )
                                .child(
                                    crate::ui::button(
                                        "empty-clear-filters",
                                        "Clear filters",
                                        false,
                                        cx,
                                    )
                                    .on_click(
                                        cx.listener(|table, _, _, cx| table.clear_filters(cx)),
                                    ),
                                ),
                        ),
                )
                .into_any_element()
        } else {
            table.into_any_element()
        };
        let toolbar = if let Some(name) = &self.document_title {
            Self::document_titlebar(name.clone(), window, cx)
        } else {
            div()
                .flex_none()
                .flex()
                .justify_end()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .bg(colors.surface)
                .border_b_1()
                .border_color(colors.border)
                .text_sm()
        };
        let inspector_visible = self.inspector_visible(cx);
        let toolbar = toolbar.child(self.find.clone()).child(
            crate::ui::ghost_button("show-inspector", "Inspector", inspector_visible, false, cx)
                .aria_expanded(inspector_visible)
                .size(px(28.))
                .rounded(px(6.))
                .child(
                    div()
                        .w(px(18.))
                        .h(px(14.))
                        .flex()
                        .justify_end()
                        .rounded(px(3.))
                        .border_1()
                        .border_color(colors.foreground)
                        .child(div().mr(px(4.)).w(px(1.)).h_full().bg(colors.foreground)),
                )
                .on_click(cx.listener(|table, _, _, cx| table.toggle_inspector(cx))),
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .key_context("GewuDocument")
            .on_action(cx.listener(Self::reveal_scatter_point))
            .when(!self.requested_query().filters.is_empty(), |view| {
                view.on_action(
                    cx.listener(|t, _: &actions::ClearFilters, _, cx| t.clear_filters(cx)),
                )
            })
            .when(self.find.read(cx).can_navigate(), |view| {
                view.on_action(cx.listener(|t, _: &actions::FindNext, _, cx| {
                    t.find.update(cx, |find, cx| find.find_match(false, cx));
                }))
                .on_action(cx.listener(|t, _: &actions::FindPrevious, _, cx| {
                    t.find.update(cx, |find, cx| find.find_match(true, cx));
                }))
            })
            .on_action(cx.listener(|table, _: &base::input::Search, window, cx| {
                table.open_find(window, cx);
            }))
            .on_action(cx.listener(|table, _: &base::input::Escape, window, cx| {
                if table.close_panel(window, cx) {
                    return;
                }
                if table.find.read(cx).open {
                    table.find.update(cx, |find, cx| find.close(window, cx));
                    window.focus(&table.focus, cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|t, _: &actions::ShowInspector, _, cx| t.toggle_inspector(cx)))
            .on_action(cx.listener(|t, _: &actions::ShowChartInspector, _, cx| t.show_chart(cx)))
            .when_some(
                self.selected_column()
                    .filter(|_| !self.data.queries().is_running()),
                |view, id| {
                    view.on_action(cx.listener(
                        move |t, _: &actions::AnalyzeSelectedColumn, _, cx| {
                            t.analyze_column(id, cx);
                        },
                    ))
                },
            )
            .on_action(cx.listener(|_, _: &actions::ToggleRowNumbers, _, cx| {
                Preferences::update(cx, |p| p.show_row_numbers = !p.show_row_numbers);
            }))
            .when_some(self.selected_column(), |view, id| {
                view.on_action(
                    cx.listener(move |t, _: &actions::EditFilter, w, cx| {
                        t.filter_column(id, w, cx)
                    }),
                )
            })
            .on_action(cx.listener(|t, _: &actions::ManageColumns, w, cx| t.open_columns(w, cx)))
            .on_action(cx.listener(|t, _: &actions::EditSort, w, cx| t.open_sort(w, cx)))
            .when(self.can_export(cx), |view| {
                view.on_action(
                    cx.listener(|t, _: &actions::ExportTable, w, cx| t.open_export(w, cx)),
                )
            })
            .child(toolbar)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(table)
                    .children(self.panel(window, cx, filter_anchor.map(|(_, anchor)| anchor))),
            )
            .child(self.status_bar(window, cx))
    }
}
