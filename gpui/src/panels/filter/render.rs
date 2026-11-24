use base::TestSupportExt;

use super::*;

impl FilterEditor {
    pub(super) fn applied_has_missing_rule(&self) -> bool {
        self.snapshot
            .query()
            .filters
            .iter()
            .find(|f| f.column == self.column.id)
            .is_some_and(|entry| {
                matches!(
                    entry.filter,
                    Filter::OnlyNulls
                        | Filter::NotNull
                        | Filter::Condition {
                            include_nulls: false,
                            ..
                        }
                )
            })
    }

    pub(super) fn input(
        label: &'static str,
        input: &Entity<InputState>,
        window: &Window,
        cx: &App,
    ) -> impl IntoElement + use<> {
        crate::panels::input_frame(div().id(label), true, &input.focus_handle(cx), window, cx)
            .px_2()
            .flex()
            .items_center()
            .child(div().flex_1().min_w_0().child(Input::new(input)))
            .test_support()
    }

    pub(super) fn field(
        label: &'static str,
        input: &Entity<InputState>,
        window: &Window,
        cx: &App,
    ) -> Div {
        Self::labeled(label, Self::input(label, input, window, cx), cx)
    }

    pub(super) fn labeled(label: &'static str, control: impl IntoElement, cx: &App) -> Div {
        let colors = crate::theme::GewuPalette::get(cx);
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
            .child(control)
    }

    pub(super) fn include_missing(&self, count: u64, cx: &mut Context<Self>) -> base::Checkbox {
        crate::panels::checkbox(
            "include-missing",
            format!("Include missing ({count})"),
            self.draft.missing != MissingPolicy::Exclude,
            cx,
            cx.listener(|editor, _, _, cx| {
                editor.draft.toggle_include_missing();
                cx.notify();
            }),
        )
    }
}

impl Render for FilterEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = crate::theme::GewuPalette::get(cx);
        let mut panel = crate::panels::elevated(div().id("filter-editor"), window, cx)
            .overflow_y_scroll()
            .max_h_full()
            .w(px(368.).min(window.viewport_size().width))
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .text_sm()
            .child(section(format!("Filter · {}", self.column.name)));
        if self.draft.mode == Mode::Values
            && self
                .draft
                .facet()
                .is_some_and(|facet| facet.values.is_none())
        {
            panel = panel.child(self.draft.exact_description());
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
                crate::errors::inline_warning(crate::errors::message(error))
                    .text_color(colors.danger_text),
            );
        }
        if let Some(facet) = self.draft.facet() {
            let has_choices = facet.values.is_some();
            let modes = Draft::modes(
                self.column.capabilities,
                has_choices || self.draft.has_exact_condition(),
            );
            if modes.len() > 1 {
                let mut tabs = segmented_tabs("filter-tabs", "Filter mode", window, cx);
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
                Mode::Values if has_choices => panel.child(self.choices(facet, window, cx)),
                Mode::Values => panel,
                Mode::Range => panel.child(self.range(window, cx)),
                Mode::Text => panel
                    .child(Self::field("Match text", &self.text, window, cx))
                    .child(
                        row()
                            .gap_3()
                            .child(crate::panels::checkbox(
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
                            .child(crate::panels::checkbox(
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
            let show_missing_policy = (self.draft.mode != Mode::Values || !has_choices)
                && (self.draft.mode == Mode::Missing
                    || facet.null_count > 0
                    || self.draft.missing != MissingPolicy::Include
                    || self.applied_has_missing_rule());
            if show_missing_policy {
                let mut choices =
                    crate::panels::segmented_radios("missing-policy", "Missing values", window, cx);
                for (id, label, policy) in [
                    ("exclude-missing", "Exclude", MissingPolicy::Exclude),
                    ("include-missing", "Include", MissingPolicy::Include),
                    ("only-missing", "Only missing", MissingPolicy::Only),
                ] {
                    let selected = self.draft.missing == policy;
                    let select = cx.listener(move |editor, _: &ClickEvent, _, cx| {
                        editor.draft.missing = policy;
                        cx.notify();
                    });
                    let radio = base::Radio::new(id)
                        .accessibility_label(label)
                        .checked(selected)
                        .on_change(move |_, ev, w, cx| select(ev, w, cx));
                    choices = choices.child(segment(radio, label, selected, cx));
                }
                panel = panel.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.muted_foreground)
                                .child(format!("Missing values ({})", facet.null_count)),
                        )
                        .child(choices),
                );
            }
        } else if self.error.is_none() {
            panel = panel.child("Loading filter choices…");
        }
        panel
            .child(
                row()
                    .w_full()
                    .pt_2()
                    .border_t_1()
                    .border_color(colors.border)
                    .child(
                        button("reset-filter", "Reset", false, cx).on_click(
                            cx.listener(|editor, _, window, cx| editor.reset(window, cx)),
                        ),
                    )
                    .child(div().flex_1())
                    .child(
                        button("cancel-filter", "Cancel", false, cx)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(FilterEvent::Close))),
                    )
                    .child(
                        button("apply-filter", "Apply", true, cx)
                            .disabled(self.draft.facet().is_none() || self.error.is_some())
                            .on_click(cx.listener(|editor, _, _, cx| {
                                editor.draft.search = editor.text.read(cx).value().to_string();
                                editor.draft.low = editor.low.read(cx).value().to_string();
                                editor.draft.high = editor.high.read(cx).value().to_string();
                                cx.emit(FilterEvent::Apply(
                                    editor.column.id,
                                    editor.draft.filter(),
                                ));
                            })),
                    ),
            )
            .test_support()
    }
}
