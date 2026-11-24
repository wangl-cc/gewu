//! Bounded display labels are independent of the full values used for searching and selection.
use base::TestSupportExt;

use super::*;

pub(super) struct ChoiceRow {
    label: SharedString,
    accessible: SharedString,
    count: SharedString,
}

impl FilterEditor {
    pub(super) fn install_choices(&mut self, facet: Arc<FilterChoices>, cx: &mut Context<Self>) {
        if self
            .draft
            .facet()
            .is_some_and(|old| Arc::ptr_eq(old, &facet))
        {
            return;
        }
        self.rows = facet
            .values
            .iter()
            .flatten()
            .map(|choice| {
                let label = crate::panels::value_preview(&choice.label);
                ChoiceRow {
                    accessible: format!("{label}, {}", crate::counted(choice.count, "row")).into(),
                    label,
                    count: choice.count.to_string().into(),
                }
            })
            .collect();
        self.draft.set_facet(Some(facet));
        self.search_choices(cx);
    }

    pub(super) fn search_choices(&mut self, cx: &mut Context<Self>) {
        self.search_task = None;
        self.matches.clear();
        let Some(facet) = self.draft.facet().cloned() else {
            return;
        };
        let query = self.search.read(cx).value();
        if query.is_empty() {
            self.matches.extend(0..self.rows.len());
            return;
        }
        self.search_task = Some(cx.spawn(async move |this, cx| {
            let matches = cx
                .background_spawn(async move {
                    let pattern = regex::RegexBuilder::new(&regex::escape(&query))
                        .case_insensitive(true)
                        .build()
                        // Extremely long queries can exceed the regex compiler's size limit.
                        .map_err(|_| query.to_lowercase());
                    facet
                        .values
                        .iter()
                        .flatten()
                        .enumerate()
                        .filter_map(|(i, choice)| {
                            let matches = match &pattern {
                                Ok(pattern) => pattern.is_match(&choice.label),
                                Err(query) => choice.label.to_lowercase().contains(query),
                            };
                            matches.then_some(i)
                        })
                        .collect()
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                editor.matches = matches;
                cx.notify();
            });
        }));
    }

    pub(super) fn choices(
        &self,
        facet: &Arc<FilterChoices>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let colors = crate::theme::GewuPalette::get(cx);
        let mut list = div()
            .id("facet-list")
            .max_h(px(210.))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        for &index in &self.matches {
            let choice = &self.rows[index];
            let checked = self.draft.value_checked(index);
            let source = facet.clone();
            let toggle = cx.listener(move |editor, _: &ClickEvent, _, cx| {
                editor.draft.toggle(&source, index);
                cx.notify();
            });
            list = list.child(
                crate::panels::checkbox(
                    ("facet", index),
                    choice.label.clone(),
                    checked,
                    cx,
                    toggle,
                )
                .accessibility_label(choice.accessible.clone())
                .flex_none()
                .px_2()
                .h(px(27.))
                .hover(move |s| s.bg(colors.button_secondary_hover))
                .child(
                    div()
                        .ml_auto()
                        .flex_none()
                        .text_xs()
                        .font_features(crate::theme::tabular_digits())
                        .text_color(colors.muted_foreground)
                        .child(choice.count.clone()),
                ),
            );
        }
        let mut actions = row().gap_3().px_2();
        for (id, label, all) in [
            ("all-values", "Select all", true),
            ("no-values", "Deselect all", false),
        ] {
            actions = actions.child(
                base::Button::new(id)
                    .accessibility_label(label)
                    .flex()
                    .items_center()
                    .h(px(24.))
                    .cursor_pointer()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .hover(move |s| s.underline().text_color(colors.foreground))
                    .child(label)
                    .on_click(cx.listener(move |editor, _, _, cx| {
                        editor.draft.select_all(all);
                        cx.notify();
                    })),
            );
        }
        let show_missing = facet.null_count > 0
            || self.applied_has_missing_rule()
            || self.draft.missing == MissingPolicy::Only;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(Self::input("Search values", &self.search, window, cx))
            .child(
                div()
                    .id("value-selection")
                    .role(Role::Group)
                    .aria_label("Value selection")
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.canvas)
                    .child(actions.border_b_1().border_color(colors.border))
                    .child(list)
                    .children(show_missing.then(|| {
                        div()
                            .border_t_1()
                            .border_color(colors.border)
                            .px_2()
                            .py_1()
                            .child(self.include_missing(facet.null_count, cx))
                    }))
                    .test_support(),
            )
    }
}
