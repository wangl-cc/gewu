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

pub(in crate::panels) fn sliders_icon(color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let o = bounds.origin;
            for (y, knob_x) in [(3.5, 2.75), (8.375, 6.75)] {
                window.paint_quad(
                    fill(
                        Bounds::new(o + point(px(1.5), px(y)), size(px(10.), px(1.25))),
                        color,
                    )
                    .corner_radii(px(0.625)),
                );
                window.paint_quad(
                    fill(
                        Bounds::new(o + point(px(knob_x), px(y - 1.125)), size(px(3.5), px(3.5))),
                        color,
                    )
                    .corner_radii(px(1.75)),
                );
            }
        },
    )
    .size(px(13.))
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
        if picker == Picker::Column {
            return div().min_w_0().w_full().h(px(28.)).child(field);
        }
        let colors = crate::theme::GewuPalette::get(cx);
        let (short, direction, log_id, log_label, scale) = match picker {
            Picker::X => (
                "X",
                Axis::Horizontal,
                "log-x",
                "Log X",
                self.settings.x_scale,
            ),
            Picker::Y => ("Y", Axis::Vertical, "log-y", "Log Y", self.settings.y_scale),
            Picker::Column => unreachable!(),
        };
        let supports_log = self
            .info(selected)
            .is_some_and(|c| c.capabilities.logarithmic);
        let pressed = supports_log && scale == AxisScale::Log10;
        let click_log = cx.listener(move |panel, _, _, cx| {
            let scale = match direction {
                Axis::Horizontal => &mut panel.settings.x_scale,
                Axis::Vertical => &mut panel.settings.y_scale,
            };
            *scale = charts::toggle_scale(*scale);
            panel.load(cx);
            cx.notify();
        });
        let log_toggle = base::Toggle::new(log_id)
            .pressed(pressed)
            .disabled(!supports_log)
            .accessibility_label(log_label)
            .styles(|styles| {
                styles.disabled(|style| {
                    style
                        .opacity(crate::theme::DISABLED_OPACITY)
                        .cursor_default()
                })
            })
            .flex_none()
            .h(px(28.))
            .px(px(8.))
            .rounded(px(6.))
            .text_size(px(12.))
            .when(pressed, |toggle| {
                toggle
                    .bg(colors.accent)
                    .text_color(colors.accent_foreground)
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(supports_log, |toggle| {
                        toggle
                            .cursor_pointer()
                            .hover(move |s| s.bg(colors.accent_hover))
                            .active(move |s| s.bg(colors.accent_pressed()))
                    })
            })
            .when(!pressed, |toggle| {
                toggle
                    .bg(colors.button_secondary)
                    .text_color(colors.foreground)
                    .font_weight(FontWeight::MEDIUM)
                    .when(supports_log, |toggle| {
                        toggle
                            .cursor_pointer()
                            .hover(move |s| s.bg(colors.button_secondary_hover))
                            .active(move |s| s.bg(colors.secondary_pressed()))
                    })
            })
            .when(supports_log, |toggle| {
                toggle
                    .focus_visible(move |s| s.shadow(colors.focus_shadows()))
                    .on_change(move |_, ev, w, cx| click_log(ev, w, cx))
            })
            .child("Log");
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_none()
                    .w(px(14.))
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors.muted_foreground)
                    .child(short),
            )
            .child(div().min_w_0().flex_1().h(px(28.)).child(field))
            .child(log_toggle)
    }

    pub(super) fn scatter_limit_popover(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> base::Popover {
        let colors = crate::theme::GewuPalette::get(cx);
        let custom = self.settings.limit != 2048;
        let active = custom || self.scatter_options_open;
        let formatted_limit = crate::messages::format_count(u64::from(self.settings.limit));
        let ink = if active {
            colors.accent
        } else {
            colors.muted_foreground
        };
        let trigger = base::Button::new("scatter-limits")
            .accessibility_label("Maximum points")
            .aria_value(formatted_limit.clone())
            .aria_expanded(self.scatter_options_open)
            .flex()
            .items_center()
            .gap_1()
            .h(px(22.))
            .px(px(6.))
            .rounded(px(5.))
            .cursor_pointer()
            .text_size(px(11.))
            .font_features(crate::theme::tabular_digits())
            .text_color(ink)
            .when(self.scatter_options_open, |b| b.bg(colors.button_secondary))
            .hover(move |s| {
                s.bg(colors.button_secondary_hover)
                    .text_color(colors.foreground)
            })
            .active(move |s| s.bg(colors.secondary_pressed()))
            .focus_visible(move |s| s.shadow(colors.focus_shadows()))
            .child(format!("Max {formatted_limit}"))
            .child(chevron(true, ink));
        let content = self.scatter_options_open.then(|| {
            let mut choices = crate::panels::segmented_radios(
                "scatter-limit-choices",
                "Maximum points",
                window,
                cx,
            );
            for limit in [512u32, 1024, 2048, 4096] {
                let selected = self.settings.limit == limit;
                let label = crate::messages::format_count(u64::from(limit));
                let pick = cx.listener(move |panel, _, _, cx| {
                    panel.settings.limit = limit;
                    panel.scatter_options_open = false;
                    panel.load(cx);
                    cx.notify();
                });
                let radio = base::Radio::new(("points", limit as usize))
                    .checked(selected)
                    .accessibility_label(label.clone())
                    .on_change(move |_, ev, w, cx| pick(ev, w, cx));
                choices = choices.child(segment(radio, label, selected, cx));
            }
            crate::panels::elevated(div(), window, cx)
                .w(px(228.))
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.muted_foreground)
                        .child("Maximum points"),
                )
                .child(choices)
        });
        base::Popover::new("scatter-limit-popover")
            .anchor(Anchor::TopRight)
            .offset(px(4.))
            .open(self.scatter_options_open)
            .trigger(trigger)
            .on_open_change(cx.listener(|panel, open: &bool, _, cx| {
                panel.scatter_options_open = *open;
                cx.notify();
            }))
            .content(move |_, _, _| div().children(content))
    }
}
