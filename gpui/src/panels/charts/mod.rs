mod axis;
mod labels;
mod scatter;
use gewu_core::{AxisScale, Histogram};
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

pub(super) fn histogram(histogram: &Histogram, log_counts: bool, cx: &App) -> Div {
    let colors = base::Theme::global(cx).tokens.colors;
    let peak = histogram.counts.iter().copied().max().unwrap_or(0);
    let n = histogram.counts.len();
    let mut bars = Vec::with_capacity(n);
    for (i, &count) in histogram.counts.iter().enumerate() {
        let (start, end) = bin_fraction(histogram, i);
        bars.push((start, end, bar_height(count, peak, log_counts)));
    }
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
    let mut chart = div()
        .flex()
        .flex_col()
        .gap_1()
        .font_features(crate::theme::tabular_digits())
        .text_color(colors.muted_foreground)
        .text_size(px(11.))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let rule = px(1. / window.scale_factor());
                    window.paint_quad(fill(
                        Bounds::new(
                            point(bounds.left(), bounds.bottom() - rule),
                            size(bounds.size.width, rule),
                        ),
                        colors.border,
                    ));
                    for (start, end, height) in bars {
                        let width = bounds.size.width * (end - start);
                        let h = bounds.size.height * height;
                        let origin = point(
                            bounds.left() + bounds.size.width * start,
                            bounds.bottom() - h,
                        );
                        let bar = Bounds::new(origin, size((width - rule).max(width * 0.5), h));
                        window.paint_quad(fill(bar, colors.primary));
                    }
                },
            )
            .w_full()
            .aspect_ratio(4. / 3.),
        )
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
