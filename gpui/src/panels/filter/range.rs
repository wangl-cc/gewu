use gpui_kit::prelude::FluentBuilder;

use super::*;

impl FilterEditor {
    pub(super) fn range(&self, cx: &mut Context<Self>) -> Div {
        let mut content = div().flex().flex_col().gap_2();
        if let Some(preview) = self.preview.as_deref().and_then(Option::as_ref) {
            content = content.child(self.range_plot(preview, cx));
        }
        content = content.child(
            div()
                .flex()
                .gap_2()
                .child(Self::field("Minimum", &self.low, cx).flex_1())
                .child(Self::field("Maximum", &self.high, cx).flex_1()),
        );
        let mut options = row().gap_3();
        if self.column.capabilities.logarithmic {
            options = options.child(Self::checkbox(
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
        if self.preview.as_deref().is_some_and(Option::is_some) {
            options = options.child(Self::checkbox(
                "filter-log-counts",
                "Log frequency",
                self.log_counts,
                cx,
                cx.listener(|editor, _, _, cx| {
                    editor.log_counts = !editor.log_counts;
                    cx.notify();
                }),
            ));
        }
        content.child(options)
    }

    fn range_plot(&self, h: &RangeHistogram, cx: &mut Context<Self>) -> Div {
        let colors = base::Theme::global(cx).tokens.colors;
        let mut chart = div()
            .flex()
            .flex_col()
            .gap_1()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(colors.background)
            .p_2()
            .font_features(crate::theme::tabular_digits())
            .text_xs()
            .text_color(colors.muted_foreground);
        if h.histogram.lo.is_some() && h.histogram.hi.is_some() {
            let mut bars = div()
                .relative()
                .w_full()
                .h(px(72.))
                .flex()
                .items_end()
                .border_b_1()
                .border_color(colors.border)
                .child(
                    div()
                        .absolute()
                        .top(relative(0.5))
                        .w_full()
                        .h(px(1.))
                        .bg(colors.border.opacity(0.55)),
                );
            let peak = h.histogram.counts.iter().copied().max().unwrap_or(0);
            let selected = self.selected_bins(cx);
            for (i, count) in h.histogram.counts.iter().enumerate() {
                let height = charts::bar_height(*count, peak, self.log_counts);
                let (a, b) = charts::bin_fraction(&h.histogram, i);
                let active = selected.is_some_and(|(lo, hi)| (lo..=hi).contains(&i));
                bars =
                    bars.child(
                        base::Button::new(("range-bin", i))
                            .flex_none()
                            .w(relative(b - a))
                            .h_full()
                            .flex()
                            .items_end()
                            .pr(px(1.))
                            .when(active, |bin| bin.bg(colors.selection))
                            .hover(move |bin| bin.bg(colors.selection))
                            .cursor_pointer()
                            .accessibility_label(format!("Bin {}: {count} rows", i + 1))
                            .child(div().w_full().h(relative(height)).rounded_t(px(1.)).bg(
                                if active {
                                    colors.primary
                                } else {
                                    colors.muted_foreground.opacity(0.68)
                                },
                            ))
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
            bars = bars
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|editor, _, _, _| editor.drag = None),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|editor, _, _, _| editor.drag = None),
                );
            chart = chart
                .child(bars)
                .child(charts::histogram_axis(&h.histogram));
        } else {
            chart = chart.child(if self.scale == AxisScale::Log10 {
                "No positive finite values to plot"
            } else {
                "No values to plot"
            });
        }
        if h.histogram.non_positive_count + h.histogram.infinite_count > 0 {
            chart = chart.child(format!(
                "{} nonpositive and {} infinite omitted",
                h.histogram.non_positive_count, h.histogram.infinite_count
            ));
        }
        chart
    }
}
