use gpui_kit::prelude::FluentBuilder;

use super::*;

impl TableView {
    pub(crate) fn document_titlebar(name: SharedString, window: &mut Window, cx: &mut App) -> Div {
        let colors = base::Theme::global(cx).tokens.colors;
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
                    .font_weight(FontWeight::MEDIUM)
                    .child(name),
            );
        #[cfg(target_os = "macos")]
        let title = {
            let pressed = window.use_state(cx, |_, _| false);
            title
                .pl(if window.is_fullscreen() {
                    px(12.)
                } else {
                    px(84.)
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
    let colors = base::Theme::global(cx).tokens.colors;
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
                .text_color(colors.destructive)
                .child(message),
        )
        .into_any_element()
}

impl Render for TableView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
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
                t.selection = Selection::all(t.data.row_count(), t.data.columns.len());
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
                        let (frame, mut overlays) = entity.update(cx, |table, cx| {
                            let layout = table.layout_grid(bounds, window, cx);
                            let frame = table.build_frame(layout, cx);
                            let semantics = window
                                .is_a11y_active()
                                .then(|| accessibility::rows(table, &frame.layout, cx));
                            let header = table.hover_target(&frame.layout);
                            let error = frame
                                .formatting_error
                                .clone()
                                .map(|m| formatting_alert(m, cx));
                            (frame, [semantics, header, error])
                        });
                        for overlay in overlays.iter_mut().flatten() {
                            overlay.prepaint_as_root(bounds.origin, bounds.size.into(), window, cx);
                        }
                        (frame, overlays)
                    },
                    move |_, (frame, overlays), window, cx| {
                        paint::paint(frame, window, cx);
                        for mut overlay in overlays.into_iter().flatten() {
                            overlay.paint(window, cx);
                        }
                    },
                )
                .size_full(),
            );
        let colors = base::Theme::global(cx).tokens.colors;
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
        let toolbar = toolbar.child(self.find.clone()).child(
            base::Button::new("show-inspector")
                .accessibility_label("Inspector")
                .aria_expanded(self.controls.inspector_visible)
                .when(self.controls.inspector_visible, |button| {
                    button.bg(colors.secondary)
                })
                .flex_none()
                .size(px(30.))
                .flex()
                .justify_center()
                .items_center()
                .rounded_md()
                .cursor_pointer()
                .hover(move |style| style.bg(colors.accent))
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
            .on_action(cx.listener(|t, _: &actions::EditSort, _, cx| t.open_sort(cx)))
            .when(self.can_export(cx), |view| {
                view.on_action(cx.listener(|t, _: &actions::ExportTable, _, cx| t.open_export(cx)))
            })
            .child(toolbar)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(table)
                    .children(self.panel(cx)),
            )
            .child(self.status_bar(cx))
    }
}
