use super::*;

fn metric(label: impl Into<SharedString>, value: impl Into<SharedString>, cx: &App) -> Div {
    let colors = base::Theme::global(cx).tokens.colors;
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
        col: &ColumnInfo,
        results: &ColumnResults,
        cx: &mut Context<Self>,
    ) -> Div {
        let colors = base::Theme::global(cx).tokens.colors;
        let mut content = div().flex().flex_col().gap_4().flex_none();
        for (section, result) in &results.sections {
            let section = *section;
            let expanded = self.settings.expanded(section, col);
            let mut group = div().flex().flex_col().gap_2().child(
                base::Button::new(("inspect-section", section as usize))
                    .accessibility_label(section.title())
                    .flex()
                    .items_center()
                    .gap_1()
                    .cursor_pointer()
                    .text_color(colors.muted_foreground)
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(controls::chevron(expanded, colors.muted_foreground))
                    .child(section.title())
                    .aria_expanded(expanded)
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        panel.settings.expansion[section as usize] = Some(!expanded);
                        panel.load(cx);
                        cx.notify();
                    })),
            );
            if let Some(result) = result {
                group = match result {
                    Ok(data) => group.child(self.section_data(data, col, cx)),
                    Err(error) => group.child(crate::errors::message(error)).child(
                        button(("retry-section", section as usize), "Retry", false, cx).on_click(
                            cx.listener(|p, _, _, cx| {
                                p.load(cx);
                                cx.notify();
                            }),
                        ),
                    ),
                };
            }
            content = content.child(group);
        }
        content
    }

    fn section_data(&self, data: &SectionData, col: &ColumnInfo, cx: &mut Context<Self>) -> Div {
        let digits = crate::preferences::Preferences::get(cx).statistics_digits;
        let count = self.snapshot.filtered().row_count();
        let mut body = div().flex().flex_col().gap_1();
        match data {
            SectionData::Overview(missing) => body
                .child(metric("Type", col.display_type.clone(), cx))
                .child(metric("Storage", col.technical_type.clone(), cx))
                .child(metric(
                    "Nullable",
                    if col.nullable { "Yes" } else { "No" },
                    cx,
                ))
                .child(metric("Rows", count.to_string(), cx))
                .child(metric("Missing", missing.to_string(), cx)),
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
                        body = body.child(metric(name, value.clone(), cx));
                    }
                }
                if s.min_label.is_none() && s.max_label.is_none() {
                    body = body.child("No numeric values");
                }
                let mean = s
                    .mean_label
                    .as_ref()
                    .map(|label| format!("≈ {label}"))
                    .or_else(|| s.mean.map(|v| charts::significant_digits(v, digits)));
                if let Some(mean) = mean {
                    body = body.child(metric("Mean", mean, cx));
                }
                if col.value_category == gewu_core::ValueCategory::Number
                    && let Some(sd) = s.std_dev
                {
                    body = body.child(metric(
                        "Standard deviation",
                        charts::significant_digits(sd, digits),
                        cx,
                    ));
                }
                if s.nan_count + s.infinite_count > 0 {
                    body = body
                        .child(metric("NaN", s.nan_count.to_string(), cx))
                        .child(metric("Infinite", s.infinite_count.to_string(), cx));
                }
                body
            }
            SectionData::Distribution(h) => body
                .child(self.histogram_options(col.capabilities.logarithmic, cx))
                .child(charts::histogram(h, self.settings.log_counts, cx)),
            SectionData::Lengths(s) => {
                let units = match s.unit {
                    gewu_core::LengthUnit::Characters => "Unicode scalars",
                    gewu_core::LengthUnit::Bytes => "bytes",
                    gewu_core::LengthUnit::Elements => "elements",
                };
                body = body.child(metric("Units", units, cx)).child(metric(
                    "Empty",
                    s.empty_count.to_string(),
                    cx,
                ));
                if let Some(v) = s.min {
                    body = body.child(metric("Minimum", v.to_string(), cx));
                }
                if let Some(v) = s.max {
                    body = body.child(metric("Maximum", v.to_string(), cx));
                }
                if let Some(v) = s.mean {
                    body = body.child(metric("Mean", charts::significant_digits(v, digits), cx));
                }
                body.child(self.histogram_options(false, cx))
                    .child(charts::histogram(
                        &s.histogram,
                        self.settings.log_counts,
                        cx,
                    ))
            }
            SectionData::Values { distinct, rows } => {
                body = body.child(metric("Estimated distinct", distinct.to_string(), cx));
                if !rows.is_empty() {
                    body = body.child("Common values · estimated");
                }
                for (label, frequency) in rows {
                    let part = *frequency as f64 / count.max(1) as f64;
                    body = body
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .gap_2()
                                .child(div().min_w_0().flex_1().truncate().child(label.clone()))
                                .child(
                                    div()
                                        .flex_none()
                                        .font_features(crate::theme::tabular_digits())
                                        .child(format!("{frequency} ({:.1}%)", part * 100.)),
                                ),
                        )
                        .child(
                            div()
                                .h(px(3.))
                                .w(relative(part.clamp(0., 1.) as f32))
                                .bg(base::Theme::global(cx).tokens.colors.primary),
                        );
                }
                body
            }
        }
    }

    fn histogram_options(&self, supports_log: bool, cx: &mut Context<Self>) -> Div {
        let mut controls = row().child(
            button(
                "histogram-options",
                "Chart display options",
                self.chart_options_open,
                cx,
            )
            .aria_expanded(self.chart_options_open)
            .on_click(cx.listener(|p, _, _, cx| {
                p.chart_options_open = !p.chart_options_open;
                cx.notify();
            })),
        );
        if !self.chart_options_open {
            return controls;
        }
        let mut choices = crate::panels::radio_group("histogram-bins", "Number of bins");
        for bins in [10, 20, 40, 60, 100] {
            choices = choices.child(crate::panels::radio(
                ("bins", bins as usize),
                format!("{bins} bins"),
                self.settings.bins == bins,
                cx,
                cx.listener(move |panel, _, _, cx| {
                    panel.settings.bins = bins;
                    panel.load(cx);
                    cx.notify();
                }),
            ));
        }
        controls = controls.child(choices);
        if supports_log {
            controls = controls.child(crate::panels::toggle(
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
        controls.child(crate::panels::toggle(
            "inspector-log-counts",
            "Log frequency",
            self.settings.log_counts,
            cx,
            cx.listener(|panel, _, _, cx| {
                panel.settings.log_counts = !panel.settings.log_counts;
                cx.notify();
            }),
        ))
    }
}
