mod axis;
mod labels;
mod scatter;
use gewu_core::{AxisScale, Histogram, RangeBounds};
use gpui_kit::{base, *};
pub(super) use labels::significant_digits;
pub(super) use scatter::ScatterChart;

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

pub(super) fn bin_interval(bounds: &RangeBounds) -> String {
    match (&bounds.min, &bounds.max) {
        (Some(a), Some(b)) if a.label == b.label => a.label.clone(),
        (Some(a), Some(b)) => format!("{} – {}", a.label, b.label),
        (Some(a), None) => format!("≥ {}", a.label),
        (None, Some(b)) => format!("≤ {}", b.label),
        (None, None) => "All plotted values".into(),
    }
}

pub(super) fn histogram(
    histogram: &Histogram,
    log_counts: bool,
    interval: impl Fn(usize) -> String,
    cx: &App,
) -> Div {
    let colors = base::Theme::global(cx).tokens.colors;
    if histogram.lo.is_none() || histogram.hi.is_none() {
        let mut empty = div().child(if histogram.scale == AxisScale::Log10 {
            "No positive finite values to plot"
        } else {
            "No values to plot"
        });
        if histogram.non_positive_count + histogram.infinite_count > 0 {
            empty = empty.child(format!(
                "{} nonpositive, {} infinite omitted",
                histogram.non_positive_count, histogram.infinite_count
            ));
        }
        return empty;
    }
    let mut plot = div()
        .relative()
        .w_full()
        .aspect_ratio(4. / 3.)
        .border_b_1()
        .border_color(colors.border);
    for fraction in [0.25, 0.5, 0.75] {
        plot = plot.child(
            div()
                .absolute()
                .top(relative(fraction))
                .w_full()
                .h(px(1.))
                .bg(colors.border.opacity(0.5)),
        );
    }
    let peak = histogram.counts.iter().copied().max().unwrap_or(0);
    for (index, &count) in histogram.counts.iter().enumerate() {
        let (start, end) = bin_fraction(histogram, index);
        let hint = format!("{} · {}", interval(index), crate::counted(count, "row"));
        let height = bar_height(count, peak, log_counts);
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
                .pr(px(0.5))
                .role(Role::Image)
                .aria_label(hint.clone())
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(hint.clone()).build(window, cx)
                })
                .child(
                    div()
                        .w_full()
                        .h(relative(height))
                        .bg(colors.primary.opacity(0.5))
                        .group_hover(group, move |style| style.bg(colors.primary)),
                )
                .test_support(),
        );
    }
    let mut chart = div()
        .flex()
        .flex_col()
        .gap_1()
        .font_features(crate::theme::tabular_digits())
        .text_color(colors.muted_foreground)
        .text_size(px(11.))
        .child(plot)
        .child(histogram_axis(histogram));
    if histogram.non_positive_count > 0 || histogram.infinite_count > 0 {
        chart = chart.child(format!(
            "{} nonpositive, {} infinite omitted",
            histogram.non_positive_count, histogram.infinite_count
        ));
    }
    chart
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
    let mut axis = div().flex().flex_col().gap_1().text_xs();
    if let (Some(a), Some(b)) = (&h.min_label, &h.max_label) {
        let endpoints = labels::endpoints(a, b);
        axis = axis.child(
            div()
                .flex()
                .justify_between()
                .child(endpoints.lower)
                .child(endpoints.upper),
        );
        if let Some(context) = endpoints.context {
            axis = axis.child(context);
        }
    }
    axis
}
