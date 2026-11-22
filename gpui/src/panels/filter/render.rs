use base::TestSupportExt;

use super::*;

impl FilterEditor {
    pub(super) fn field(label: &'static str, input: &Entity<InputState>, cx: &App) -> Div {
        let colors = base::Theme::global(cx).tokens.colors;
        section(label).child(
            div()
                .id(label)
                .px_2()
                .py_1()
                .rounded_md()
                .bg(colors.background)
                .border_1()
                .border_color(colors.border)
                .child(Input::new(input))
                .test_support(),
        )
    }

    fn range(&self, cx: &mut Context<Self>) -> Div {
        let mut content = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(Self::field("Minimum", &self.low, cx))
            .child(Self::field("Maximum", &self.high, cx));
        if self.column.capabilities.logarithmic {
            content = content.child(crate::panels::toggle(
                "filter-log-bins",
                "Logarithmic bins",
                self.scale == AxisScale::Log10,
                cx,
                cx.listener(|editor, _, _, cx| {
                    editor.scale = charts::toggle_scale(editor.scale);
                    editor.load(cx);
                    cx.notify();
                }),
            ));
        }
        if let Some(h) = self.preview.as_deref().and_then(Option::as_ref) {
            let mut bars = row().w_full().flex_nowrap().gap_0().h(px(68.)).items_end();
            let peak = h.histogram.counts.iter().copied().max().unwrap_or(0);
            let selected = self.selected_bins(cx);
            for (i, count) in h.histogram.counts.iter().enumerate() {
                let height = charts::bar_height(*count, peak, self.log_counts);
                let (a, b) = charts::bin_fraction(&h.histogram, i);
                let colors = base::Theme::global(cx).tokens.colors;
                bars = bars.child(
                    base::Button::new(("range-bin", i))
                        .flex_none()
                        .w(relative(b - a))
                        .h_full()
                        .flex()
                        .items_end()
                        .px(px(0.5))
                        .cursor_pointer()
                        .accessibility_label(format!("Bin {}: {count} rows", i + 1))
                        .child(div().w_full().h(px(height * 64.)).bg(
                            if selected.is_none_or(|(a, b)| (a..=b).contains(&i)) {
                                colors.primary
                            } else {
                                colors.muted_foreground
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
            content = content
                .child(bars)
                .child(charts::histogram_axis(&h.histogram))
                .child(crate::panels::toggle(
                    "filter-log-counts",
                    "Log frequency",
                    self.log_counts,
                    cx,
                    cx.listener(|editor, _, _, cx| {
                        editor.log_counts = !editor.log_counts;
                        cx.notify();
                    }),
                ));
            if h.histogram.non_positive_count + h.histogram.infinite_count > 0 {
                content = content.child(format!(
                    "{} nonpositive and {} infinite omitted",
                    h.histogram.non_positive_count, h.histogram.infinite_count
                ));
            }
        }
        content
    }
}

impl Render for FilterEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = base::Theme::global(cx).tokens.colors;
        let mut panel = div()
            .id("filter-editor")
            .overflow_y_scroll()
            .max_h_full()
            .w(px(320.))
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(colors.border)
            .bg(colors.surface)
            .text_color(colors.surface_foreground)
            .text_sm()
            .child(section(format!("Filter · {}", self.column.name)));
        if let Some(applied) = self
            .snapshot
            .query()
            .filters
            .iter()
            .find(|f| f.column == self.column.id)
        {
            panel = panel.child(super::super::filter_description(&applied.filter));
        }
        if let Some(error) = &self.error {
            panel = panel.child(
                div()
                    .text_color(colors.destructive)
                    .child(crate::errors::message(error)),
            );
        }
        if let Some(facet) = &self.facet {
            let modes = Draft::modes(self.column.capabilities, facet.values.is_some());
            if modes.len() > 1 {
                let mut tabs = base::Tabs::new("filter-tabs")
                    .aria_label("Filter mode")
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_1();
                for mode in modes {
                    let name = match mode {
                        Mode::Values => "Values",
                        Mode::Range => "Range",
                        Mode::Text => "Text",
                        Mode::Missing => "Missing",
                    };
                    tabs = tabs.child(
                        crate::panels::tab(name, self.draft.mode == mode, cx).on_click(
                            cx.listener(move |editor, _, _, cx| {
                                editor.draft.set_mode(mode);
                                editor.sync_checks();
                                if mode == Mode::Range && editor.preview.is_none() {
                                    editor.load(cx);
                                }
                                cx.notify();
                            }),
                        ),
                    );
                }
                panel = panel.child(tabs);
            }
            panel = match self.draft.mode {
                Mode::Values => panel.child(self.choices(facet, cx)),
                Mode::Range => panel.child(self.range(cx)),
                Mode::Text => panel
                    .child(Self::field("Match text", &self.text, cx))
                    .child(
                        row()
                            .child(crate::panels::toggle(
                                "whole",
                                "Whole value",
                                self.draft.comparison == TextMatch::Equals,
                                cx,
                                cx.listener(|editor, _, _, cx| {
                                    editor.draft.comparison = match editor.draft.comparison {
                                        TextMatch::Contains => TextMatch::Equals,
                                        TextMatch::Equals => TextMatch::Contains,
                                    };
                                    cx.notify();
                                }),
                            ))
                            .child(crate::panels::toggle(
                                "case",
                                "Match case",
                                self.draft.case_sensitive,
                                cx,
                                cx.listener(|editor, _, _, cx| {
                                    editor.draft.case_sensitive = !editor.draft.case_sensitive;
                                    cx.notify();
                                }),
                            )),
                    ),
                Mode::Missing => panel,
            };
            let mut missing = row().child(crate::panels::toggle(
                "include-missing",
                format!("Include missing ({})", facet.null_count),
                self.draft.include_missing && !self.draft.only_missing,
                cx,
                cx.listener(|editor, _, _, cx| {
                    editor.draft.only_missing = false;
                    editor.draft.include_missing = !editor.draft.include_missing;
                    cx.notify();
                }),
            ));
            if self.draft.mode != Mode::Values {
                missing = missing.child(crate::panels::toggle(
                    "only-missing",
                    "Only missing",
                    self.draft.only_missing,
                    cx,
                    cx.listener(|editor, _, _, cx| {
                        editor.draft.only_missing = !editor.draft.only_missing;
                        cx.notify();
                    }),
                ));
            }
            panel = panel.child(missing);
        } else if self.error.is_none() {
            panel = panel.child("Loading filter choices…");
        }
        panel.child(
            row()
                .justify_end()
                .child(
                    button("reset-filter", "Reset", false, cx)
                        .on_click(cx.listener(|editor, _, window, cx| editor.reset(window, cx))),
                )
                .child(
                    button("cancel-filter", "Cancel", false, cx)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(FilterEvent::Close))),
                )
                .child(
                    button("apply-filter", "Apply", true, cx)
                        .disabled(self.facet.is_none() || self.error.is_some())
                        .on_click(cx.listener(|editor, _, _, cx| {
                            editor.draft.search = editor.text.read(cx).value().to_string();
                            editor.draft.low = editor.low.read(cx).value().to_string();
                            editor.draft.high = editor.high.read(cx).value().to_string();
                            cx.emit(FilterEvent::Apply(editor.column.id, editor.draft.filter()));
                        })),
                ),
        )
    }
}
