use gpui_kit::prelude::FluentBuilder;

use super::*;
use crate::panels::{button, segment};

struct Settings;
struct SettingsWindow(AnyWindowHandle);
impl Global for SettingsWindow {}

pub(crate) fn open(cx: &mut App) {
    if let Some(existing) = cx.try_global::<SettingsWindow>().map(|w| w.0)
        && existing
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let options = crate::platform::sized_window(430., 424., cx);
    match gpui_kit::open_window(options, cx, |w, cx| {
        crate::platform::follow_appearance(w, cx);
        w.set_window_title("Gewu Settings");
        cx.new(|_| Settings)
    }) {
        Ok((w, _)) => cx.set_global(SettingsWindow(w)),
        Err(error) => cx.show_system_notification(SystemNotification {
            tag: "settings".into(),
            title: "Could not open settings".into(),
            body: error.to_string().into(),
            actions: Vec::new(),
        }),
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Preferences::get(cx);
        let colors = crate::theme::GewuPalette::get(cx);
        let hairline = px(1. / window.scale_factor());
        let section = |title: &'static str, first: bool| {
            div()
                .when(!first, |s| {
                    s.mt_1()
                        .pt_3()
                        .border_t(hairline)
                        .border_color(colors.border)
                })
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.muted_foreground)
                .child(title)
        };
        let mut view = div()
            .id("settings-scroll")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .bg(colors.canvas)
            .text_color(colors.foreground)
            .text_size(px(13.))
            .child(section("Display", true));
        type SetNumber = fn(&mut Preferences, f32);
        let numbers: [(&str, f32, f32, f32, SetNumber); 2] = [
            ("Text size", p.font_size, 10., 20., |p, n| p.font_size = n),
            (
                "Statistics precision",
                p.statistics_digits as f32,
                3.,
                15.,
                |p, n| p.statistics_digits = n as usize,
            ),
        ];
        for (index, (label, n, min, max, set)) in numbers.into_iter().enumerate() {
            view = view.child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().child(label))
                    .child(
                        button(("minus", index), "−", false, cx)
                            .w(px(28.))
                            .accessibility_label(format!("Decrease {label}"))
                            .disabled(n <= min)
                            .on_click(move |_, _, cx| {
                                Preferences::update(cx, |p| set(p, (n - 1.).max(min)))
                            }),
                    )
                    .child(
                        div()
                            .w(px(24.))
                            .text_center()
                            .font_features(crate::theme::tabular_digits())
                            .child(n.to_string()),
                    )
                    .child(
                        button(("plus", index), "+", false, cx)
                            .w(px(28.))
                            .accessibility_label(format!("Increase {label}"))
                            .disabled(n >= max)
                            .on_click(move |_, _, cx| {
                                Preferences::update(cx, |p| set(p, (n + 1.).min(max)))
                            }),
                    ),
            );
        }
        let mut density = crate::panels::segmented_radios("row-heights", "Row height", window, cx);
        for height in [24_u32, 28, 32] {
            let label = height.to_string();
            let selected = p.row_height == height as f32;
            let radio = base::Radio::new(("row-height", height as usize))
                .checked(selected)
                .accessibility_label(label.clone())
                .on_change(move |_, _, _, cx| {
                    Preferences::update(cx, |p| p.row_height = height as f32)
                });
            density = density.child(segment(radio, label, selected, cx).flex_none().px_2());
        }
        let label = if [24., 28., 32.].contains(&p.row_height) {
            "Row height".to_owned()
        } else {
            format!("Row height ({})", p.row_height)
        };
        view = view.child(
            div()
                .flex()
                .items_center()
                .child(div().flex_1().child(label))
                .child(density),
        );
        let mut contrast =
            crate::panels::segmented_radios("contrast-options", "High contrast", window, cx);
        for (i, (label, mode)) in [
            ("Follow System", HighContrast::System),
            ("Off", HighContrast::Off),
            ("On", HighContrast::On),
        ]
        .into_iter()
        .enumerate()
        {
            let selected = p.high_contrast == mode;
            let radio = base::Radio::new(("high-contrast", i))
                .checked(selected)
                .accessibility_label(label)
                .on_change(move |_, _, _, cx| Preferences::update(cx, |p| p.high_contrast = mode));
            contrast = contrast.child(segment(radio, label, selected, cx).flex_none().px_2());
        }
        view = view.child(
            div()
                .flex()
                .items_center()
                .child(div().flex_1().child("High contrast"))
                .child(contrast),
        );
        let choice = |index: usize, label, checked, toggle: fn(&mut Preferences)| {
            crate::panels::checkbox(("pref", index), label, checked, cx, move |_, _, cx| {
                Preferences::update(cx, toggle);
            })
        };
        view.child(choice(0, "Show row numbers", p.show_row_numbers, |p| {
            p.show_row_numbers = !p.show_row_numbers
        }))
        .child(
            div().pl(px(22.)).child(
                choice(1, "Start row numbering at 1", p.row_number_base == 1, |p| {
                    if p.show_row_numbers {
                        p.row_number_base = 1 - p.row_number_base;
                    }
                })
                .disabled(!p.show_row_numbers),
            ),
        )
        .child(section("Copy and export", false))
        .child(choice(
            2,
            "Include column names when copying",
            p.copy_includes_headers,
            |p| p.copy_includes_headers = !p.copy_includes_headers,
        ))
        .child(choice(
            3,
            "Remember last export format",
            p.remember_export_format,
            |p| p.remember_export_format = !p.remember_export_format,
        ))
        .child(section("Confirmation", false))
        .child(choice(
            4,
            "Confirm hiding sorted or filtered columns",
            !p.skip_hide_warning,
            |p| p.skip_hide_warning = !p.skip_hide_warning,
        ))
    }
}
