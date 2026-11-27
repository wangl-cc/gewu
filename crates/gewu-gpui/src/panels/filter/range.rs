//! The filter editor's range mode: bound inputs and the brushable distribution preview.
use gpui_kit::{base::TestSupportExt, prelude::FluentBuilder};

use super::*;

impl FilterEditor {
    fn range_field(&self, index: usize, window: &Window, cx: &App) -> Div {
        let (label, input) = if index == 0 {
            ("Minimum", &self.low)
        } else {
            ("Maximum", &self.high)
        };
        let focus = input.read(cx).focus_handle(cx);
        let mut control = div()
            .relative()
            .child(Self::input(label, input, window, cx));
        if self.column.floating_point
            && !focus.is_focused(window)
            && let Some(exact) = &self.brushed_bounds[index]
            && input.read(cx).value().as_ref() == exact
            && let Ok(value) = exact.parse::<f64>()
        {
            let short = charts::significant_digits(
                value,
                crate::preferences::Preferences::get(cx).statistics_digits,
            );
            if short.len() + 2 < exact.len() {
                let colors = crate::theme::GewuPalette::get(cx);
                let hint = exact.clone();
                control = control.child(
                    div()
                        .id(("brush-preview", index))
                        .test_support()
                        .absolute()
                        .left(px(1.))
                        .right(px(1.))
                        .top(px(1.))
                        .bottom(px(1.))
                        .flex()
                        .items_center()
                        .px_2()
                        .rounded_md()
                        .bg(colors.canvas)
                        .cursor_text()
                        .child(div().truncate().child(format!("≈ {short}")))
                        .tooltip(crate::ui::tooltip(hint))
                        .on_mouse_down(MouseButton::Left, move |_, w, cx| w.focus(&focus, cx)),
                );
            }
        }
        Self::labeled(label, control, cx).flex_1()
    }

    pub(super) fn range(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let mut content = div().flex().flex_col().gap_2();
        if let Some(preview) = self.preview() {
            content = content.child(self.range_plot(preview, window, cx));
        }
        content.child(
            div()
                .flex()
                .gap_2()
                .child(self.range_field(0, window, cx))
                .child(self.range_field(1, window, cx)),
        )
    }

    fn range_plot(&self, h: &RangeHistogram, window: &Window, cx: &mut Context<Self>) -> Div {
        let colors = crate::theme::GewuPalette::get(cx);
        let cues = crate::theme::VisualCues::get(cx);
        let hairline = px(1. / window.scale_factor());
        let has_bins = h.histogram.lo.is_some() && h.histogram.hi.is_some();
        let peak = h.histogram.counts.iter().copied().max().unwrap_or(0);
        let custom = self.scale == AxisScale::Log10 || self.log_counts;
        let mut options_host = div()
            .relative()
            .flex_none()
            .child(
                crate::ui::options_trigger(
                    "range-display-options",
                    custom,
                    self.display_options,
                    cx,
                )
                .on_click(cx.listener(|editor, _, _, cx| {
                    editor.display_options = !editor.display_options;
                    cx.notify();
                })),
            )
            .on_mouse_down_out(cx.listener(|editor, _, _, cx| {
                if editor.display_options {
                    editor.display_options = false;
                    cx.notify();
                }
            }));
        if self.display_options {
            let mut popover = crate::ui::elevated(div().id("range-options-popover"), window, cx)
                .occlude()
                .absolute()
                .top(px(26.))
                .right_0()
                .w(px(176.))
                .p(px(10.))
                .flex()
                .flex_col()
                .gap_2();
            if self.column.capabilities.logarithmic {
                popover = popover.child(crate::ui::checkbox(
                    "filter-log-bins",
                    "Log bins",
                    self.scale == AxisScale::Log10,
                    cx,
                    cx.listener(|editor, _, _, cx| {
                        editor.scale = charts::toggle_scale(editor.scale);
                        editor.load(cx);
                        cx.notify();
                    }),
                ));
            }
            popover = popover.child(crate::ui::checkbox(
                "filter-log-counts",
                "Log frequency",
                self.log_counts,
                cx,
                cx.listener(|editor, _, _, cx| {
                    editor.log_counts = !editor.log_counts;
                    cx.notify();
                }),
            ));
            options_host = options_host.child(popover.test_support());
        }
        let header = div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(div().children(
                (has_bins && peak > 0).then(|| charts::histogram_peak_label(peak, self.log_counts)),
            ))
            .child(options_host);
        let mut body = div().flex().flex_col().gap_1();
        if has_bins {
            let mut bars = div()
                .relative()
                .w_full()
                .h(px(72.))
                .flex()
                .items_end()
                .border_b(hairline)
                .border_color(colors.chart_grid)
                .child(
                    div()
                        .absolute()
                        .top(relative(0.55))
                        .w_full()
                        .h(hairline)
                        .bg(colors.chart_grid),
                );
            let selected = self.selected_bins(cx);
            for (i, count) in h.histogram.counts.iter().enumerate() {
                let height = charts::bar_height(*count, peak, self.log_counts) * 0.90;
                let (a, b) = charts::bin_fraction(&h.histogram, i);
                let active = selected.is_some_and(|(lo, hi)| (lo..=hi).contains(&i));
                let interval = charts::bin_interval(&h.ranges, i, self.column.floating_point);
                let hint = format!("{interval} · {}", crate::counted(*count, "row"));
                let cursor = match selected {
                    Some((lo, hi)) if i == lo || i == hi => CursorStyle::ResizeLeftRight,
                    Some((lo, hi)) if lo < i && i < hi => {
                        if self.drag.is_some() {
                            CursorStyle::ClosedHand
                        } else {
                            CursorStyle::OpenHand
                        }
                    }
                    _ => CursorStyle::Crosshair,
                };
                bars = bars.child(
                    base::Button::new(("range-bin", i))
                        .flex_none()
                        .w(relative(b - a))
                        .h_full()
                        .flex()
                        .items_end()
                        .group("range-bin")
                        .when(active, |bin| bin.bg(colors.brush_fill))
                        .pr(hairline)
                        .cursor(cursor)
                        .accessibility_label(hint.clone())
                        .tooltip(crate::ui::tooltip(hint))
                        .child(
                            div()
                                .w_full()
                                .h(relative(height))
                                .rounded_t(px(1.))
                                .bg(if active {
                                    colors.chart_primary
                                } else {
                                    colors.chart_muted
                                })
                                .when(!active && *count > 0, |bar| {
                                    bar.border(cues.unselected_bin_outline)
                                        .border_color(colors.muted_foreground)
                                })
                                .group_hover("range-bin", move |s| {
                                    s.bg(colors.chart_primary_hover)
                                }),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |editor, event: &MouseDownEvent, window, cx| {
                                editor.start_brush(i, event.modifiers.shift, window, cx);
                            }),
                        )
                        .on_mouse_move(cx.listener(
                            move |editor, event: &MouseMoveEvent, window, cx| {
                                if editor.drag.is_some()
                                    && event.pressed_button == Some(MouseButton::Left)
                                {
                                    editor.brush(i, window, cx);
                                }
                            },
                        ))
                        .on_click(cx.listener(move |editor, ev: &ClickEvent, window, cx| {
                            if matches!(ev, ClickEvent::Keyboard(_)) {
                                editor.start_brush(i, ev.modifiers().shift, window, cx);
                                editor.drag = None;
                            }
                        })),
                );
            }
            let (left, right, active_selection) = match selected {
                Some((first, last)) => (
                    charts::bin_fraction(&h.histogram, first).0,
                    charts::bin_fraction(&h.histogram, last).1,
                    true,
                ),
                None => (0., 1., false),
            };
            if active_selection {
                let edge = cues.brush_outline.max(px(1.5));
                bars = bars.child(
                    div()
                        .absolute()
                        .top_0()
                        .left(relative(left))
                        .w(relative(right - left))
                        .h_full()
                        .border_l(edge)
                        .border_t(cues.brush_outline)
                        .border_r(edge)
                        .border_color(colors.accent),
                );
            }
            for (id, label, position, is_end) in [
                ("range-handle-min", "Adjust minimum bound", left, false),
                ("range-handle-max", "Adjust maximum bound", right, true),
            ] {
                let border_width = if active_selection { px(1.5) } else { hairline };
                let edge_color = if active_selection {
                    colors.accent
                } else {
                    colors.muted_foreground
                };
                let handle = div()
                    .id(id)
                    .role(Role::Slider)
                    .aria_label(label)
                    .group(id)
                    .absolute()
                    .top(relative(0.5))
                    .mt(px(-12.))
                    .left(relative(position))
                    .when(!is_end, |h| {
                        h.ml(px(-8.)).rounded_l(px(4.)).rounded_r(px(2.))
                    })
                    .when(is_end, |h| h.rounded_r(px(4.)).rounded_l(px(2.)))
                    .w(px(8.))
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(colors.elevated)
                    .border(border_width)
                    .border_color(edge_color)
                    .shadow_xs()
                    .cursor(CursorStyle::ResizeLeftRight)
                    .hover(move |s| s.border_color(colors.accent))
                    .child(
                        div()
                            .w(px(if active_selection { 1.5 } else { 1. }))
                            .h(px(10.))
                            .rounded_full()
                            .bg(edge_color)
                            .group_hover(id, move |s| s.bg(colors.accent)),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |editor, _, window, cx| {
                            editor.start_handle_drag(is_end, window, cx);
                            cx.stop_propagation();
                        }),
                    );
                bars = bars.child(handle.test_support());
            }
            bars = bars
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|editor, _, _, _| editor.drag = None),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|editor, _, _, _| editor.drag = None),
                );
            let plot_well = div()
                .w_full()
                .rounded(px(6.))
                .border(hairline)
                .border_color(colors.border)
                .bg(colors.canvas)
                .px(px(10.))
                .pt(px(6.))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|editor, _, _, _| editor.drag = None),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|editor, _, _, _| editor.drag = None),
                )
                .child(bars);
            body = body
                .child(plot_well)
                .child(charts::histogram_axis(&h.histogram).px(px(10.)));
        } else {
            body = body.child(if self.scale == AxisScale::Log10 {
                "No positive finite values to plot"
            } else {
                "No values to plot"
            });
        }
        body = body.children(crate::ui::messages::omitted_values(
            h.histogram.non_positive_count,
            h.histogram.infinite_count,
        ));
        // Paint header last via reverse column order so the corner floating card stacks above bins.
        div()
            .flex()
            .flex_col_reverse()
            .gap(px(6.))
            .rounded(px(8.))
            .border(hairline)
            .border_color(colors.border)
            .bg(colors.surface)
            .p(px(10.))
            .font_features(crate::theme::tabular_digits())
            .text_size(px(11.))
            .text_color(colors.muted_foreground)
            .child(body)
            .child(header)
    }
}
