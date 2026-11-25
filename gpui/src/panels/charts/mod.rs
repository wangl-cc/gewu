mod axis;
mod labels;
mod scatter;
use gewu_core::{AxisScale, Histogram, RangeBounds};
use gpui_kit::*;
pub(super) use labels::significant_digits;
pub(super) use scatter::ScatterChart;
#[cfg(test)]
pub(crate) use scatter::scatter_data_bounds;

pub(super) fn toggle_scale(scale: AxisScale) -> AxisScale {
    match scale {
        AxisScale::Linear => AxisScale::Log10,
        AxisScale::Log10 => AxisScale::Linear,
    }
}

pub(super) fn bar_height(count: u64, peak: u64, logarithmic: bool) -> f32 {
    if count == 0 || peak == 0 {
        return 0.;
    }
    if logarithmic {
        // Leave the zero baseline empty and keep equal frequency ratios equally spaced.
        ((1. + (count as f64).log10()) / (1. + (peak as f64).log10())) as f32
    } else {
        (count as f64 / peak as f64) as f32
    }
}

pub(super) fn bin_interval(ranges: &[RangeBounds], index: usize, floating_point: bool) -> String {
    let bounds = &ranges[index];
    if floating_point
        && let Some(upper) = &bounds.max
        && let Some(cut) = ranges.get(index + 1).and_then(|next| next.min.as_ref())
        && cut.coordinate.next_down() == upper.coordinate
    {
        // The next bin starts at the true cut. Showing it exclusively avoids exposing
        // the float predecessor that encodes this same interval in a filter.
        return match &bounds.min {
            Some(lower) => format!("[{}, {})", lower.label, cut.label),
            None => format!("< {}", cut.label),
        };
    }
    match (&bounds.min, &bounds.max) {
        (Some(a), Some(b)) if a.label == b.label => a.label.clone(),
        (Some(a), Some(b)) => format!("{} – {}", a.label, b.label),
        (Some(a), None) => format!("≥ {}", a.label),
        (None, Some(b)) => format!("≤ {}", b.label),
        (None, None) => "All plotted values".into(),
    }
}

pub(super) fn histogram_peak_label(peak: u64, log_counts: bool) -> String {
    format!(
        "max {}{}",
        crate::messages::format_count(peak),
        if log_counts { " · log" } else { "" }
    )
}

pub(super) fn histogram(
    histogram: &Histogram,
    log_counts: bool,
    interval: impl Fn(usize) -> String,
    window: &Window,
    cx: &App,
) -> Div {
    let colors = crate::theme::GewuPalette::get(cx);
    if histogram.lo.is_none() || histogram.hi.is_none() {
        return div()
            .child(if histogram.scale == AxisScale::Log10 {
                "No positive finite values to plot"
            } else {
                "No values to plot"
            })
            .children(crate::messages::omitted_values(
                histogram.non_positive_count,
                histogram.infinite_count,
            ));
    }
    let hairline = px(1. / window.scale_factor());
    let mut plot = div()
        .relative()
        .w_full()
        .h(px(96.))
        .border_b(hairline)
        .border_color(colors.chart_grid)
        .child(
            div()
                .absolute()
                .top(relative(0.55))
                .w_full()
                .h(hairline)
                .bg(colors.chart_grid),
        );
    let peak = histogram.counts.iter().copied().max().unwrap_or(0);
    for (index, &count) in histogram.counts.iter().enumerate() {
        let (start, end) = bin_fraction(histogram, index);
        let hint = format!("{} · {}", interval(index), crate::counted(count, "row"));
        let height = bar_height(count, peak, log_counts) * 0.90;
        let group = "inspector-histogram-bin";
        plot = plot.child(
            div()
                .id((group, index))
                .group(group)
                .absolute()
                .left(relative(start))
                .w(relative(end - start))
                .h_full()
                .flex()
                .items_end()
                .pr(hairline)
                .role(Role::Image)
                .aria_label(hint.clone())
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(hint.clone()).build(window, cx)
                })
                .child(
                    div()
                        .w_full()
                        .h(relative(height))
                        .rounded_t(px(1.))
                        .bg(colors.chart_primary)
                        .group_hover(group, move |style| style.bg(colors.chart_primary_hover)),
                )
                .test_support(),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap_1()
        .font_features(crate::theme::tabular_digits())
        .text_color(colors.muted_foreground)
        .text_size(px(11.))
        .children((peak > 0).then(|| div().w_full().child(histogram_peak_label(peak, log_counts))))
        .child(plot)
        .child(histogram_axis(histogram))
        .children(crate::messages::omitted_values(
            histogram.non_positive_count,
            histogram.infinite_count,
        ))
}

pub(super) fn bin_fraction(h: &Histogram, i: usize) -> (f32, f32) {
    let n = h.counts.len();
    if h.bin_edges.len() == n + 1 && h.bin_edges[n] > h.bin_edges[0] {
        let lo = h.bin_edges[0];
        let span = h.bin_edges[n] - lo;
        (
            ((h.bin_edges[i] - lo) / span) as f32,
            ((h.bin_edges[i + 1] - lo) / span) as f32,
        )
    } else {
        (i as f32 / n as f32, (i + 1) as f32 / n as f32)
    }
}

pub(super) fn histogram_axis(h: &Histogram) -> Div {
    let mut axis = div().flex().flex_col().gap_1().text_size(px(11.));
    if let (Some(a), Some(b)) = (&h.min_label, &h.max_label) {
        let endpoints = labels::endpoints(a, b);
        axis = axis.child(
            div()
                .relative()
                .flex()
                .justify_between()
                .child(histogram_label(&endpoints.lower))
                .child(histogram_label(&endpoints.upper))
                .children(middle_tick(h, a, b).map(|(at, text)| {
                    div()
                        .absolute()
                        .left(relative(at))
                        .ml(px(-44.))
                        .w(px(88.))
                        .text_center()
                        .truncate()
                        .child(text)
                })),
        );
        if let Some(context) = endpoints.context {
            axis = axis.child(context);
        }
    }
    axis
}

fn middle_tick(h: &Histogram, minimum: &str, maximum: &str) -> Option<(f32, String)> {
    let (low, high) = (h.lo?, h.hi?);
    // Histogram coordinates for dates and times use native units. Keep those labels in
    // their original format until the chart can format positions in that same unit.
    let (min_value, max_value) = (minimum.parse::<f64>().ok()?, maximum.parse::<f64>().ok()?);
    let span = high - low;
    if min_value >= max_value || !span.is_finite() || span <= 0. {
        return None;
    }
    let tick = if h.scale == AxisScale::Log10 {
        (low + span / 2.).round()
    } else {
        axis::numeric_ticks(low, high, 4, h.integer)
            .into_iter()
            .min_by(|a, b| {
                (a - low - span / 2.)
                    .abs()
                    .total_cmp(&(b - low - span / 2.).abs())
            })?
    };
    let fraction = ((tick - low) / span) as f32;
    if !(0.3..=0.7).contains(&fraction) {
        return None;
    }
    let value = if h.scale == AxisScale::Log10 {
        10f64.powf(tick)
    } else {
        tick
    };
    (min_value < value && value < max_value).then(|| {
        let mut label = value.to_string();
        if value.fract() != 0. && label.len() > 12 {
            // Axis coordinates are approximate, and generating a tick can introduce a
            // floating-point tail. Keep ordinary tick labels clear of that noise.
            label = significant_digits(value, 12);
        }
        (fraction, histogram_label(&label))
    })
}

fn histogram_label(text: &str) -> String {
    let (sign, digits) = text
        .strip_prefix('-')
        .map_or(("", text), |rest| ("-", rest));
    let (whole, fraction) = digits
        .find('.')
        .map_or((digits, ""), |dot| digits.split_at(dot));
    if text.len() <= 12
        && let Ok(whole) = whole.parse::<u64>()
    {
        return format!("{sign}{}{fraction}", crate::messages::format_count(whole));
    }
    labels::compact(text)
}

#[cfg(test)]
#[path = "histogram_labels_tests.rs"]
mod histogram_labels_tests;
