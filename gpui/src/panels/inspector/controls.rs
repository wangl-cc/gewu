use gpui_kit::prelude::FluentBuilder;

use super::*;

pub(super) fn chevron(down: bool, color: Hsla) -> impl IntoElement {
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
        cx: &mut Context<Self>,
    ) -> Div {
        let selected_name =
            self.info(selected)
                .map(|c| c.name.as_str())
                .unwrap_or(if picker == Picker::X {
                    "Source row"
                } else {
                    "—"
                });
        let colors = base::Theme::global(cx).tokens.colors;
        let picker_button = base::Button::new(title)
            .accessibility_label(format!("{title}, {selected_name}"))
            .min_w_0()
            .flex_1()
            .flex()
            .items_center()
            .justify_between()
            .gap_1()
            .h(px(26.))
            .px_2()
            .rounded_md()
            .bg(colors.secondary)
            .hover(move |s| s.bg(colors.accent))
            .cursor_pointer()
            .child(div().min_w_0().truncate().child(selected_name.to_owned()))
            .child(chevron(true, colors.muted_foreground))
            .aria_expanded(self.picker == Some(picker))
            .on_click(cx.listener(move |panel, _, _, cx| {
                panel.picker = if panel.picker == Some(picker) {
                    None
                } else {
                    Some(picker)
                };
                cx.notify();
            }));
        let mut area = div().flex().flex_col().flex_none().gap_1().child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(80.))
                        .flex_none()
                        .text_color(colors.muted_foreground)
                        .child(title),
                )
                .child(picker_button),
        );
        if self.picker != Some(picker) {
            return area;
        }
        let option = |element: ElementId, label: SharedString, id: Option<ColumnId>| {
            let current = id == selected;
            crate::panels::radio(
                element,
                label,
                current,
                cx,
                cx.listener(move |panel, _, _, cx| {
                    panel.select(picker, id, cx);
                }),
            )
            // Confirming the current option still dismisses the picker.
            .when(current, |item| {
                item.on_click(cx.listener(|panel, _, _, cx| {
                    panel.picker = None;
                    cx.notify();
                }))
            })
        };
        let mut options = base::RadioGroup::new("inspector-options")
            .aria_label(title)
            .max_h(px(180.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        if picker == Picker::X {
            options = options.child(option("axis-source".into(), "Source row".into(), None));
        }
        for column in self
            .columns
            .iter()
            .filter(|c| picker == Picker::Column || c.capabilities.scatter)
        {
            options = options.child(option(
                ("axis-option", column.id.into_raw() as usize).into(),
                column.name.clone().into(),
                Some(column.id),
            ));
        }
        area = area.child(options);
        area
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
