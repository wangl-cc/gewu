use gpui_kit::prelude::FluentBuilder;

use super::*;

pub(in crate::panels) fn chevron(down: bool, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let center = bounds.center();
            let mut path = PathBuilder::stroke(px(1.25));
            let vertices = if down {
                [(-3., -1.5), (0., 1.5), (3., -1.5)]
            } else {
                [(-1.5, -3.), (1.5, 0.), (-1.5, 3.)]
            };
            let [a, b, c] = vertices.map(|(x, y)| center + point(px(x), px(y)));
            path.move_to(a);
            path.line_to(b);
            path.line_to(c);
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(12.))
    .flex_none()
}

impl Inspector {
    pub(super) fn picker(
        &self,
        picker: Picker,
        title: &'static str,
        selected: Option<ColumnId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        use crate::panels::choice::{Choice, ChoiceField, column_labels};

        let columns = self
            .columns
            .iter()
            .filter(|c| picker == Picker::Column || c.capabilities.scatter);
        let mut options = Vec::new();
        if picker == Picker::X {
            options.push(Choice {
                value: None,
                label: "Source row".into(),
            });
        }
        options.extend(
            column_labels(columns, (picker == Picker::X).then_some("Source row")).map(
                |(id, label)| Choice {
                    value: Some(id),
                    label,
                },
            ),
        );
        let choose = cx.listener(move |panel, value: &Option<ColumnId>, _, cx| {
            panel.select(picker, *value, cx)
        });
        let field = ChoiceField {
            id: title.into(),
            label: title,
            options,
            selected: Some(selected),
            searchable: true,
        }
        .element(window, cx, choose);
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .whitespace_nowrap()
                    .w(px(96.))
                    .child(title),
            )
            .child(div().min_w_0().flex_1().h(px(28.)).child(field))
    }

    pub(super) fn chart_options(&self, cx: &mut Context<Self>) -> Div {
        let colors = base::Theme::global(cx).tokens.colors;
        let mut choices = base::RadioGroup::new("scatter-limits")
            .axis(Axis::Horizontal)
            .aria_label("Maximum points")
            .flex()
            .items_center()
            .p(px(2.))
            .rounded_md()
            .bg(colors.muted);
        for limit in [512, 1024, 2048, 4096] {
            let label = limit.to_string();
            let selected = self.settings.limit == limit;
            let click = cx.listener(move |panel, _, _, cx| {
                panel.settings.limit = limit;
                panel.load(cx);
                cx.notify();
            });
            let radio = base::Radio::new(("points", limit as usize))
                .checked(selected)
                .accessibility_label(label.clone())
                .on_change(move |_, e, w, cx| click(e, w, cx));
            choices = choices.child(segment(radio, label, selected, cx));
        }
        let mut controls = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_2()
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(colors.muted_foreground)
                    .child("Maximum points"),
            )
            .child(choices);
        for (direction, column, scale) in [
            (Axis::Horizontal, self.x, self.settings.x_scale),
            (Axis::Vertical, self.y, self.settings.y_scale),
        ] {
            if !self
                .info(column)
                .is_some_and(|c| c.capabilities.logarithmic)
            {
                continue;
            }
            let (id, label) = match direction {
                Axis::Horizontal => ("log-x", "Log X"),
                Axis::Vertical => ("log-y", "Log Y"),
            };
            let checked = scale == AxisScale::Log10;
            let click = cx.listener(move |panel, _, _, cx| {
                let scale = match direction {
                    Axis::Horizontal => &mut panel.settings.x_scale,
                    Axis::Vertical => &mut panel.settings.y_scale,
                };
                *scale = charts::toggle_scale(*scale);
                panel.load(cx);
                cx.notify();
            });
            let switch = base::Switch::new(id)
                .checked(checked)
                .accessibility_label(label)
                .on_change(move |_, e, w, cx| click(e, w, cx))
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_between()
                .h(px(24.))
                .child(label)
                .child(
                    div()
                        .w(px(27.))
                        .h(px(16.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .px(px(2.))
                        .bg(if checked {
                            colors.primary
                        } else {
                            colors.muted_foreground
                        })
                        .when(checked, |s| s.justify_end())
                        .child(div().size(px(12.)).rounded_full().bg(if checked {
                            colors.primary_foreground
                        } else {
                            colors.background
                        })),
                );
            controls = controls.child(switch);
        }
        controls
    }
}
