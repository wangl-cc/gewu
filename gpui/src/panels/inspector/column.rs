use gpui_kit::prelude::FluentBuilder;

use super::*;

fn show_storage(col: &ColumnInfo) -> bool {
    if col.technical_type.eq_ignore_ascii_case(&col.display_type) {
        return false;
    }
    !matches!(
        (col.display_type.as_str(), col.technical_type.as_str()),
        ("bool", "Boolean") | ("string", "Utf8")
    )
}

fn format_integer_statistic(text: &str) -> String {
    let (sign, digits) = text
        .strip_prefix('-')
        .map_or(("", text), |rest| ("-", rest));
    let (whole, fraction) = digits
        .find('.')
        .map_or((digits, ""), |dot| digits.split_at(dot));
    if whole.len() > 3
        && whole.bytes().all(|b| b.is_ascii_digit())
        && (fraction.is_empty()
            || (fraction.len() > 1 && fraction[1..].bytes().all(|b| b.is_ascii_digit())))
    {
        let mut formatted = String::with_capacity(text.len() + whole.len() / 3);
        formatted.push_str(sign);
        for (offset, byte) in whole.bytes().enumerate() {
            if offset > 0 && (whole.len() - offset).is_multiple_of(3) {
                formatted.push(',');
            }
            formatted.push(byte as char);
        }
        formatted.push_str(fraction);
        return formatted;
    }
    text.to_owned()
}

fn format_numeric_statistic(value: f64, digits: usize) -> String {
    if !value.is_finite() || value == 0. {
        return value.to_string();
    }
    let exp = value.abs().log10().floor() as i32;
    let digits = digits.clamp(1, 17);
    if (-4..9).contains(&exp) {
        if exp >= digits as i32 - 1 {
            let scale = 10f64.powi(exp - digits as i32 + 1);
            let rounded = (value / scale).round() * scale;
            return format_integer_statistic(&format!("{rounded:.0}"));
        }
        let decimals = (digits as i32 - exp - 1) as usize;
        let text = format!("{value:.decimals$}");
        let trimmed = if decimals == 0 {
            text.as_str()
        } else {
            text.trim_end_matches('0').trim_end_matches('.')
        };
        return format_integer_statistic(trimmed);
    }
    charts::significant_digits(value, digits)
}

fn metric(label: impl Into<SharedString>, value: impl Into<SharedString>, cx: &App) -> Div {
    let colors = crate::theme::GewuPalette::get(cx);
    div()
        .w_full()
        .flex()
        .items_baseline()
        .justify_between()
        .gap_2()
        .child(
            div()
                .flex_none()
                .text_color(colors.muted_foreground)
                .child(label.into()),
        )
        .child(
            div()
                .min_w_0()
                .text_right()
                .font_features(crate::theme::tabular_digits())
                .child(value.into()),
        )
}

impl Inspector {
    pub(super) fn column_view(
        &self,
        results: &ColumnResults,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let colors = crate::theme::GewuPalette::get(cx);
        let mut content = div().flex().flex_col().gap_3().flex_none();
        for (index, (section, result)) in results.sections.iter().enumerate() {
            let section = *section;
            let expanded = result.is_some();
            let header = base::Button::new(("inspect-section", section as usize))
                .accessibility_label(section.title())
                .flex_1()
                .min_w_0()
                .flex()
                .justify_start()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .text_color(colors.foreground)
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(controls::chevron(expanded, colors.muted_foreground))
                .child(section.title())
                .aria_expanded(expanded)
                .on_click(cx.listener(move |panel, _, _, cx| {
                    panel.settings.expansion[section as usize] = Some(!expanded);
                    panel.load(cx);
                    cx.notify();
                }));
            let options = match result {
                Some(Ok(SectionData::Distribution(_))) => Some(self.histogram_options_popover(
                    results.column.capabilities.logarithmic,
                    window,
                    cx,
                )),
                Some(Ok(SectionData::Lengths(_))) => {
                    Some(self.histogram_options_popover(false, window, cx))
                }
                _ => None,
            };
            let mut group = div()
                .flex()
                .flex_col()
                .gap_2()
                .when(index > 0, |group| {
                    group
                        .pt_3()
                        .border_t(px(1. / window.scale_factor()))
                        .border_color(colors.border)
                })
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_1()
                        .child(header)
                        .children(options),
                );
            if let Some(result) = result {
                group = match result {
                    Ok(data) => group.child(self.section_data(data, &results.column, window, cx)),
                    Err(error) => group
                        .child(
                            crate::errors::inline_warning(crate::errors::message(error))
                                .text_color(colors.danger_text),
                        )
                        .child(
                            button(("retry-section", section as usize), "Retry", false, cx)
                                .on_click(cx.listener(|p, _, _, cx| {
                                    p.load(cx);
                                    cx.notify();
                                })),
                        ),
                };
            }
            content = content.child(group);
        }
        content
    }

    fn section_data(
        &self,
        data: &SectionData,
        col: &ColumnInfo,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        use crate::messages::format_count;

        let digits = crate::preferences::Preferences::get(cx).statistics_digits;
        let count = self.snapshot.filtered().row_count();
        let mut body = div().flex().flex_col().gap_1();
        match data {
            SectionData::Overview(missing) => {
                body = body.child(metric("Type", col.display_type.clone(), cx));
                if show_storage(col) {
                    body = body.child(metric("Storage", col.technical_type.clone(), cx));
                }
                if !col.nullable {
                    body = body.child(metric("Nullable", "No", cx));
                }
                body.child(metric("Rows", format_count(count), cx))
                    .child(metric("Missing", format_count(*missing), cx))
            }
            SectionData::Structure(fields) => {
                for f in fields {
                    body = body.child(format!(
                        "{} · {}{} · {}",
                        f.name,
                        f.display_type,
                        if f.nullable { " · nullable" } else { "" },
                        f.technical_type
                    ));
                }
                body
            }
            SectionData::Statistics(s) => {
                for (name, value) in [
                    ("Minimum", &s.min_label),
                    ("Median", &s.median_label),
                    ("Maximum", &s.max_label),
                ] {
                    if let Some(value) = value {
                        let displayed = if col.floating_point {
                            value
                                .parse::<f64>()
                                .ok()
                                .map(|v| format_numeric_statistic(v, digits))
                                .unwrap_or_else(|| value.clone())
                        } else if col.value_category == gewu_core::ValueCategory::Number {
                            format_integer_statistic(value)
                        } else {
                            value.clone()
                        };
                        let exact = value.clone();
                        body = body.child(metric(name, displayed, cx).id(name).tooltip(
                            move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(exact.clone())
                                    .build(window, cx)
                            },
                        ));
                    }
                }
                if s.min_label.is_none() && s.max_label.is_none() {
                    body = body.child("No numeric values");
                }
                let exact_mean = s
                    .mean_label
                    .clone()
                    .or_else(|| s.mean.map(|v| v.to_string()));
                let mean = s
                    .mean_label
                    .as_ref()
                    .map(|label| format!("≈ {label}"))
                    .or_else(|| s.mean.map(|v| format_numeric_statistic(v, digits)));
                if let (Some(mean), Some(exact)) = (mean, exact_mean) {
                    body = body.child(metric("Mean", mean, cx).id("Mean").tooltip(
                        move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(exact.clone())
                                .build(window, cx)
                        },
                    ));
                }
                if col.value_category == gewu_core::ValueCategory::Number
                    && let Some(sd) = s.std_dev
                {
                    let exact = sd.to_string();
                    body = body.child(
                        metric(
                            "Standard deviation",
                            format_numeric_statistic(sd, digits),
                            cx,
                        )
                        .id("Standard deviation")
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(exact.clone())
                                .build(window, cx)
                        }),
                    );
                }
                if s.nan_count + s.infinite_count > 0 {
                    body = body
                        .child(metric("NaN", format_count(s.nan_count), cx))
                        .child(metric("Infinite", format_count(s.infinite_count), cx));
                }
                body
            }
            SectionData::Distribution(h) => body.child(charts::histogram(
                &h.histogram,
                self.settings.log_counts,
                |i| charts::bin_interval(&h.ranges, i, col.floating_point),
                window,
                cx,
            )),
            SectionData::Lengths(s) => {
                let units = match s.unit {
                    gewu_core::LengthUnit::Characters => "Unicode scalars",
                    gewu_core::LengthUnit::Bytes => "bytes",
                    gewu_core::LengthUnit::Elements => "elements",
                };
                body = body.child(metric("Units", units, cx)).child(metric(
                    "Empty",
                    format_count(s.empty_count),
                    cx,
                ));
                if let Some(v) = s.min {
                    body = body.child(metric("Minimum", format_count(v), cx));
                }
                if let Some(v) = s.max {
                    body = body.child(metric("Maximum", format_count(v), cx));
                }
                if let Some(v) = s.mean {
                    body = body.child(metric("Mean", format_numeric_statistic(v, digits), cx));
                }
                body.child(charts::histogram(
                    &s.histogram,
                    self.settings.log_counts,
                    |i| match s.bin_range(i) {
                        Some(range) if range.start() == range.end() => {
                            format!("Length {} ({units})", range.start())
                        }
                        Some(range) => {
                            format!("Length {} – {} ({units})", range.start(), range.end())
                        }
                        None => "No lengths".into(),
                    },
                    window,
                    cx,
                ))
            }
            SectionData::Values { distinct, rows } => {
                let colors = crate::theme::GewuPalette::get(cx);
                body = body.child(metric("Estimated distinct", format_count(*distinct), cx));
                if !rows.is_empty() {
                    body = body.child(
                        div()
                            .mt_1()
                            .text_size(px(11.))
                            .text_color(colors.muted_foreground)
                            .child("Common values · estimated"),
                    );
                }
                for (label, frequency) in rows {
                    let part = *frequency as f64 / count.max(1) as f64;
                    body = body
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .items_baseline()
                                .gap_2()
                                .child(div().min_w_0().flex_1().truncate().child(label.clone()))
                                .child(
                                    div()
                                        .flex_none()
                                        .text_size(px(12.))
                                        .text_color(colors.muted_foreground)
                                        .font_features(crate::theme::tabular_digits())
                                        .child(format!(
                                            "{} ({:.1}%)",
                                            format_count(*frequency),
                                            part * 100.,
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .h(px(4.))
                                .w_full()
                                .rounded_full()
                                .bg(colors.bar_track)
                                .child(
                                    div()
                                        .h_full()
                                        .rounded_full()
                                        .w(relative(part.clamp(0., 1.) as f32))
                                        .bg(colors.chart_primary),
                                ),
                        );
                }
                body
            }
        }
    }

    fn histogram_options_popover(
        &self,
        supports_log: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> base::Popover {
        let colors = crate::theme::GewuPalette::get(cx);
        let custom = self.settings.bins != 40
            || self.settings.log_counts
            || (supports_log && self.settings.bin_scale == AxisScale::Log10);
        let trigger = crate::panels::options_trigger(
            "histogram-options",
            custom,
            self.chart_options_open,
            cx,
        );
        let content = self.chart_options_open.then(|| {
            let mut choices =
                crate::panels::segmented_radios("histogram-bins", "Number of bins", window, cx);
            for bins in [10u32, 20, 40, 60, 100] {
                let selected = self.settings.bins == bins;
                let pick = cx.listener(move |panel, _, _, cx| {
                    panel.settings.bins = bins;
                    panel.load(cx);
                    cx.notify();
                });
                let radio = base::Radio::new(("bins", bins as usize))
                    .checked(selected)
                    .accessibility_label(format!("{bins} bins"))
                    .on_change(move |_, ev, w, cx| pick(ev, w, cx));
                choices = choices.child(segment(radio, bins.to_string(), selected, cx));
            }
            let mut body = crate::panels::elevated(div(), window, cx)
                .w(px(236.))
                .p_3()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.muted_foreground)
                                .child("Bins"),
                        )
                        .child(choices),
                );
            let mut flags = div().flex().flex_col().gap_2();
            if supports_log {
                flags = flags.child(crate::panels::checkbox(
                    "inspector-log-bins",
                    "Log bins",
                    self.settings.bin_scale == AxisScale::Log10,
                    cx,
                    cx.listener(|panel, _, _, cx| {
                        panel.settings.bin_scale = charts::toggle_scale(panel.settings.bin_scale);
                        panel.load(cx);
                        cx.notify();
                    }),
                ));
            }
            flags = flags.child(crate::panels::checkbox(
                "inspector-log-counts",
                "Log frequency",
                self.settings.log_counts,
                cx,
                cx.listener(|panel, _, _, cx| {
                    panel.settings.log_counts = !panel.settings.log_counts;
                    cx.notify();
                }),
            ));
            body = body.child(flags);
            body
        });
        base::Popover::new("histogram-options-popover")
            .anchor(Anchor::TopRight)
            .offset(px(4.))
            .open(self.chart_options_open)
            .trigger(trigger)
            .on_open_change(cx.listener(|panel, open: &bool, _, cx| {
                panel.chart_options_open = *open;
                cx.notify();
            }))
            .content(move |_, _, _| div().children(content))
    }
}

#[cfg(test)]
mod tests {
    use super::{format_integer_statistic, format_numeric_statistic};

    #[test]
    fn groups_integer_statistics_including_half_step_medians() {
        assert_eq!(format_integer_statistic("999999.5"), "999,999.5");
        assert_eq!(format_integer_statistic("-12345.5"), "-12,345.5");
        assert_eq!(format_integer_statistic("1999999"), "1,999,999");
        assert_eq!(format_integer_statistic("999.5"), "999.5");
        assert_eq!(format_integer_statistic("2024-01-01"), "2024-01-01");
    }

    #[test]
    fn formats_numeric_statistics_with_thousands_separators_before_scientific_notation() {
        assert_eq!(format_numeric_statistic(999_999.5, 5), "1,000,000");
        assert_eq!(format_numeric_statistic(577_350.413, 5), "577,350");
        assert_eq!(format_numeric_statistic(12_345.6, 6), "12,345.6");
        assert_eq!(format_numeric_statistic(-12_345.5, 6), "-12,345.5");
        assert_eq!(format_numeric_statistic(1.2345e-9, 4), "1.235e-9");
        assert_eq!(format_numeric_statistic(1.2345e12, 4), "1.234e12");
        assert_eq!(format_numeric_statistic(1.2346e12, 4), "1.235e12");
    }
}
