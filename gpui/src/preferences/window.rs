use super::*;
use crate::panels::button;

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
    let options = crate::platform::sized_window(470., 410., cx);
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Preferences::get(cx);
        let colors = base::Theme::global(cx).tokens.colors;
        let mut view = div()
            .id("settings-scroll")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .bg(colors.background)
            .text_color(colors.foreground);
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
                            .accessibility_label(format!("Decrease {label}"))
                            .disabled(n <= min)
                            .on_click(move |_, _, cx| {
                                Preferences::update(cx, |p| set(p, (n - 1.).max(min)))
                            }),
                    )
                    .child(n.to_string())
                    .child(
                        button(("plus", index), "+", false, cx)
                            .accessibility_label(format!("Increase {label}"))
                            .disabled(n >= max)
                            .on_click(move |_, _, cx| {
                                Preferences::update(cx, |p| set(p, (n + 1.).min(max)))
                            }),
                    ),
            );
        }
        let mut density = crate::panels::radio_group("row-heights", "Row height");
        for height in [24_u32, 28, 32] {
            density = density.child(crate::panels::radio(
                ("row-height", height as usize),
                height.to_string(),
                p.row_height == height as f32,
                cx,
                move |_, _, cx| Preferences::update(cx, |p| p.row_height = height as f32),
            ));
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
        type Toggle = fn(&mut Preferences);
        let choices: [(&str, bool, Toggle); 5] = [
            ("Show row numbers", p.show_row_numbers, |p| {
                p.show_row_numbers = !p.show_row_numbers
            }),
            ("Start row numbering at 1", p.row_number_base == 1, |p| {
                p.row_number_base = 1 - p.row_number_base
            }),
            (
                "Include column names when copying",
                p.copy_includes_headers,
                |p| p.copy_includes_headers = !p.copy_includes_headers,
            ),
            (
                "Remember last export format",
                p.remember_export_format,
                |p| p.remember_export_format = !p.remember_export_format,
            ),
            (
                "Confirm hiding sorted or filtered columns",
                !p.skip_hide_warning,
                |p| p.skip_hide_warning = !p.skip_hide_warning,
            ),
        ];
        for (index, (label, value, toggle)) in choices.into_iter().enumerate() {
            view = view.child(
                base::Checkbox::new(("pref", index))
                    .checked(value)
                    .accessibility_label(label)
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .child(
                        div()
                            .flex_none()
                            .w(px(18.))
                            .h(px(18.))
                            .rounded_sm()
                            .border_1()
                            .border_color(colors.border)
                            .bg(if value {
                                colors.primary
                            } else {
                                colors.background
                            })
                            .text_center()
                            .text_color(colors.primary_foreground)
                            .child(if value { "✓" } else { "" }),
                    )
                    .child(label)
                    .on_change(move |_, _, _, cx| Preferences::update(cx, toggle)),
            );
        }
        view
    }
}
