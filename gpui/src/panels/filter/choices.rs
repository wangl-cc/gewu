//! Bounded display labels are independent of the full values used for searching and selection.
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
        let mut list = div()
            .id("facet-list")
            .max_h(px(210.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        for &index in &self.matches {
            let choice = &self.rows[index];
            let checked = choice.checked;
            let colors = base::Theme::global(cx).tokens.colors;
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
                base::Checkbox::new(("facet", index))
                    .checked(checked)
                    .accessibility_label(choice.accessible.clone())
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .px_2()
                    .py_1()
                    .hover(move |s| s.bg(colors.accent))
                    .child(
                        div()
                            .w(px(16.))
                            .h(px(16.))
                            .rounded_sm()
                            .border_1()
                            .border_color(colors.border)
                            .bg(if checked {
                                colors.primary
                            } else {
                                colors.background
                            })
                            .text_color(colors.primary_foreground)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(if checked { "✓" } else { "" }),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .child(choice.label.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(choice.count.clone()),
                    )
                    .on_change(move |_, event, window, cx| toggle(event, window, cx)),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(Self::field("Search values", &self.search, cx))
            .child(
                row()
                    .child(
                        button("all-values", "Select all", false, cx).on_click(cx.listener(
                            |editor, _, _, cx| {
                                editor.draft.select_all(true);
                                editor.sync_checks();
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        button("no-values", "Deselect all", false, cx).on_click(cx.listener(
                            |editor, _, _, cx| {
                                editor.draft.select_all(false);
                                editor.sync_checks();
                                cx.notify();
                            },
                        )),
                    ),
            )
            .child(list)
    }
}
