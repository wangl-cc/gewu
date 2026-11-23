use base::TestSupportExt;

use super::*;

impl FilterEditor {
    pub(super) fn input(
        label: &'static str,
        input: &Entity<InputState>,
        cx: &App,
    ) -> impl IntoElement + use<> {
        let colors = base::Theme::global(cx).tokens.colors;
        div()
            .id(label)
            .min_w_0()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(colors.background)
            .border_1()
            .border_color(colors.border)
            .child(Input::new(input))
            .test_support()
    }

    pub(super) fn field(label: &'static str, input: &Entity<InputState>, cx: &App) -> Div {
        let colors = base::Theme::global(cx).tokens.colors;
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(label),
            )
            .child(Self::input(label, input, cx))
    }

    pub(super) fn checkbox(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        checked: bool,
        cx: &App,
        activate: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> base::Checkbox {
        let colors = base::Theme::global(cx).tokens.colors;
        let label = label.into();
        base::Checkbox::new(id)
            .accessibility_label(label.clone())
            .checked(checked)
            .on_change(move |_, ev, w, cx| activate(ev, w, cx))
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .text_size(px(12.))
            .child(
                div()
                    .flex_none()
                    .size(px(14.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .border_1()
                    .border_color(if checked {
                        colors.primary
                    } else {
                        colors.muted_foreground
                    })
                    .bg(if checked {
                        colors.primary
                    } else {
                        colors.background
                    })
                    .text_color(colors.primary_foreground)
                    .child(if checked { "✓" } else { "" }),
            )
            .child(div().min_w_0().truncate().child(label))
    }

    pub(super) fn include_missing(&self, count: u64, cx: &mut Context<Self>) -> base::Checkbox {
        Self::checkbox(
            "include-missing",
            format!("Include missing ({count})"),
            self.draft.include_missing_checked(),
            cx,
            cx.listener(|editor, _, _, cx| {
                editor.draft.toggle_include_missing();
                cx.notify();
            }),
        )
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
        if self.draft.mode == Mode::Values
            && self
                .facet
                .as_ref()
                .is_some_and(|facet| facet.values.is_none())
        {
            panel = panel.child(super::super::exact_description(
                self.draft.values.iter(),
                self.draft.excluded,
            ));
        } else if let Some(applied) = self
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
            let has_choices = facet.values.is_some();
            let modes = Draft::modes(
                self.column.capabilities,
                has_choices || self.draft.has_exact_condition(),
            );
            if modes.len() > 1 {
                let mut tabs = segmented_tabs("filter-tabs", "Filter mode", cx)
                    .border_1()
                    .border_color(colors.border);
                for mode in modes {
                    let name = match mode {
                        Mode::Values if has_choices => "Values",
                        Mode::Values => "Current selection",
                        Mode::Range => "Range",
                        Mode::Text => "Text",
                        Mode::Missing => "Missing",
                    };
                    let selected = self.draft.mode == mode;
                    let tab = base::Tab::new(name)
                        .selected(selected)
                        .accessibility_label(name)
                        .on_click(
                            cx.listener(move |editor, _, _, cx| editor.select_mode(mode, cx)),
                        );
                    tabs = tabs.child(segment(tab, name, selected, cx));
                }
                panel = panel.child(tabs);
            }
            panel = match self.draft.mode {
                Mode::Values if has_choices => panel.child(self.choices(facet, cx)),
                Mode::Values => panel,
                Mode::Range => panel.child(self.range(cx)),
                Mode::Text => panel
                    .child(Self::field("Match text", &self.text, cx))
                    .child(
                        row()
                            .gap_3()
                            .child(Self::checkbox(
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
                            .child(Self::checkbox(
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
            if self.draft.mode != Mode::Values || !has_choices {
                let mut missing = row()
                    .gap_3()
                    .child(self.include_missing(facet.null_count, cx));
                if self.draft.mode != Mode::Values {
                    missing = missing.child(Self::checkbox(
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
            }
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
