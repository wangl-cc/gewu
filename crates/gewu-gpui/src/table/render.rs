//! The grid's element tree: the painted canvas, its input handlers and accessibility overlay,
//! the empty-result prompt and the table's popovers.
use super::*;

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
        let dragging = self.grid.is_dragging();
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
            .on_hover(cx.listener(Self::mouse_left))
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
        div().size_full().flex().child(table).children(self.panel(
            window,
            cx,
            filter_anchor.map(|(_, anchor)| anchor),
        ))
    }
}
