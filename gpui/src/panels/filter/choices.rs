//! Bounded display labels are independent of the full values used for searching and selection.
use base::TestSupportExt;

use super::*;

pub(super) struct ChoiceRow {
    label: SharedString,
    accessible: SharedString,
    count: SharedString,
    checked: bool,
}

impl FilterEditor {
    pub(super) fn install_choices(&mut self, facet: Arc<FilterChoices>, cx: &mut Context<Self>) {
        if self
            .facet
            .as_ref()
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
                    accessible: format!("{label}, {} rows", choice.count).into(),
                    label,
                    count: choice.count.to_string().into(),
                    checked: false,
                }
            })
            .collect();
        self.facet = Some(facet);
        self.sync_checks();
        self.search_choices(cx);
    }

    pub(super) fn sync_checks(&mut self) {
        if let Some(choices) = self.facet.as_ref().and_then(|facet| facet.values.as_ref()) {
            for (row, choice) in self.rows.iter_mut().zip(choices) {
                row.checked = self.draft.values.contains(&choice.value) != self.draft.excluded;
            }
        }
    }

    pub(super) fn search_choices(&mut self, cx: &mut Context<Self>) {
        self.search_task = None;
        self.matches.clear();
        let Some(facet) = self.facet.clone() else {
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

    pub(super) fn choices(&self, facet: &Arc<FilterChoices>, cx: &mut Context<Self>) -> Div {
        let colors = base::Theme::global(cx).tokens.colors;
        let mut list = div()
            .id("facet-list")
            .max_h(px(210.))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        for &index in &self.matches {
            let choice = &self.rows[index];
            let checked = choice.checked;
            let source = facet.clone();
            let toggle = cx.listener(move |editor, _: &ClickEvent, _, cx| {
                if let Some(value) = editor
                    .facet
                    .as_ref()
                    .filter(|f| Arc::ptr_eq(f, &source))
                    .and_then(|f| f.values.as_ref())
                    .and_then(|v| v.get(index))
                {
                    editor.draft.toggle(value.value.clone());
                    editor.rows[index].checked = !editor.rows[index].checked;
                    cx.notify();
                }
            });
            list = list.child(
                Self::checkbox(("facet", index), choice.label.clone(), checked, cx, toggle)
                    .accessibility_label(choice.accessible.clone())
                    .flex_none()
                    .px_2()
                    .h(px(27.))
                    .hover(move |s| s.bg(colors.accent))
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
                        editor.sync_checks();
                        cx.notify();
                    })),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(Self::input("Search values", &self.search, cx))
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
                    .bg(colors.background)
                    .child(actions.border_b_1().border_color(colors.border))
                    .child(list)
                    .child(
                        div()
                            .border_t_1()
                            .border_color(colors.border)
                            .px_2()
                            .py_1()
                            .child(self.include_missing(facet.null_count, cx)),
                    )
                    .test_support(),
            )
    }
}
