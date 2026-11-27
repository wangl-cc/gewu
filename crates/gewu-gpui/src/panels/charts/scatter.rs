use std::sync::Arc;

use gewu_core::{AxisScale, ScatterResult, SourceRow};
use gpui_kit::{base::TestSupportExt, *};

use super::axis::{Axis, seconds};
use crate::{actions::RevealScatterPoint, preferences::Preferences};

const TICK_LINE_HEIGHT: Pixels = px(14.);
const TICK_LABEL_GAP: Pixels = px(4.);
const PLOT_INSET_FRACTION: f32 = 0.045;

struct Plot {
    x: Axis,
    y: Axis,
    points: Vec<(f32, f32)>,
}

pub(in crate::panels) struct ScatterChart {
    sample: Arc<ScatterResult>,
    source_x: bool,
    axis_context: Option<SharedString>,
    plot: Option<Arc<Plot>>,
    hover: Option<usize>,
    selected: Option<usize>,
    press: Option<(usize, Point<Pixels>)>,
    focus: FocusHandle,
}

fn tick_label_top(tick: Pixels, vertical: bool) -> Pixels {
    if vertical {
        tick - TICK_LINE_HEIGHT * 0.5
    } else {
        tick + TICK_LINE_HEIGHT * 0.5 + TICK_LABEL_GAP
    }
}

fn plot_bounds(bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    let left = px(50.).min(bounds.size.width);
    let top = px(8.).min(bounds.size.height);
    // The lowest Y label straddles its tick; the X labels sit below it.
    let bottom = tick_label_top(px(0.), false) + TICK_LINE_HEIGHT;
    Bounds::new(
        bounds.origin + point(left, top),
        size(
            (bounds.size.width - left - px(8.)).max(px(0.)),
            (bounds.size.height - top - bottom).max(px(0.)),
        ),
    )
}

fn data_bounds(drawing: Bounds<Pixels>) -> Bounds<Pixels> {
    let pad_x = (drawing.size.width * PLOT_INSET_FRACTION)
        .max(px(6.))
        .min(drawing.size.width * 0.25);
    let pad_y = (drawing.size.height * PLOT_INSET_FRACTION)
        .max(px(6.))
        .min(drawing.size.height * 0.25);
    Bounds::new(
        drawing.origin + point(pad_x, pad_y),
        size(
            (drawing.size.width - pad_x * 2.).max(px(0.)),
            (drawing.size.height - pad_y * 2.).max(px(0.)),
        ),
    )
}

#[cfg(test)]
pub(crate) fn scatter_data_bounds(canvas: Bounds<Pixels>) -> Bounds<Pixels> {
    data_bounds(plot_bounds(canvas))
}

impl ScatterChart {
    pub fn new(
        sample: Arc<ScatterResult>,
        x_scale: AxisScale,
        y_scale: AxisScale,
        source_x: bool,
        cx: &App,
    ) -> Self {
        let xs: Vec<_> = sample
            .points
            .iter()
            .map(|p| {
                if source_x {
                    p.source_row.0 as f64
                } else {
                    seconds(p.x, sample.x_axis_kind)
                }
            })
            .collect();
        let ys: Vec<_> = sample
            .points
            .iter()
            .map(|p| seconds(p.y, sample.y_axis_kind))
            .collect();
        let x = Axis::for_points(
            &xs,
            |i| sample.points[i].x_label.clone(),
            sample.x_axis_kind,
            x_scale,
            source_x,
            4,
        );
        let y = Axis::for_points(
            &ys,
            |i| sample.points[i].y_label.clone(),
            sample.y_axis_kind,
            y_scale,
            false,
            4,
        );
        let axis_context = match (
            x.as_ref().and_then(|axis| axis.context.as_deref()),
            y.as_ref().and_then(|axis| axis.context.as_deref()),
        ) {
            (Some(xc), Some(yc)) if xc == yc => Some(xc.to_owned().into()),
            (Some(xc), Some(yc)) => Some(format!("X: {xc} · Y: {yc}").into()),
            (Some(xc), None) => Some(format!("X: {xc}").into()),
            (None, Some(yc)) => Some(format!("Y: {yc}").into()),
            (None, None) => None,
        };
        let plot = x.zip(y).map(|(x, y)| {
            let points = xs
                .into_iter()
                .zip(ys)
                .map(|(a, b)| (x.fraction(a), y.fraction(b)))
                .collect();
            Arc::new(Plot { x, y, points })
        });
        Self {
            sample,
            source_x,
            axis_context,
            plot,
            hover: None,
            selected: None,
            press: None,
            focus: cx.focus_handle().tab_stop(true),
        }
    }

    fn reveal(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.dispatch_action(
            &RevealScatterPoint {
                chart: cx.entity_id(),
                source_row: self.sample.points[index].source_row,
            },
            window,
            cx,
        );
    }

    pub(in crate::panels) fn mark(&mut self, row: Option<SourceRow>, cx: &mut Context<Self>) {
        self.selected =
            row.and_then(|row| self.sample.points.iter().position(|p| p.source_row == row));
        cx.notify();
    }

    pub(in crate::panels) fn has_plot(&self) -> bool {
        self.plot.is_some()
    }

    pub(in crate::panels) fn readout(&self, base: u64) -> Option<SharedString> {
        self.hover.map(|i| {
            let point = &self.sample.points[i];
            let row = point.source_row.0.saturating_add(base);
            let x = if self.source_x {
                row.to_string()
            } else {
                point.x_label.clone()
            };
            SharedString::from(format!("Row {row} · X {x} · Y {}", point.y_label))
        })
    }

    pub(in crate::panels) fn summary_text(&self) -> String {
        let plotted = self.sample.points.len() as u64;
        if plotted == self.sample.eligible_rows {
            crate::counted(plotted, "point")
        } else {
            format!(
                "{} of {} points",
                crate::ui::messages::format_count(plotted),
                crate::ui::messages::format_count(self.sample.eligible_rows),
            )
        }
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let last = self.sample.points.len() - 1;
        self.hover = Some(match (self.hover, forward) {
            (Some(i), true) => (i + 1).min(last),
            (Some(i), false) => i.saturating_sub(1),
            (None, true) => 0,
            (None, false) => last,
        });
        cx.notify();
    }
}

impl Plot {
    fn point(bounds: Bounds<Pixels>, (x, y): (f32, f32)) -> Point<Pixels> {
        point(
            bounds.left() + bounds.size.width * x,
            bounds.bottom() - bounds.size.height * y,
        )
    }

    fn nearest(
        &self,
        position: Point<Pixels>,
        hit_bounds: Bounds<Pixels>,
        data_bounds: Bounds<Pixels>,
    ) -> Option<usize> {
        if !hit_bounds.contains(&position) {
            return None;
        }
        let mut nearest = None;
        let mut best = 10f32.powi(2);
        for (i, xy) in self.points.iter().enumerate() {
            let p = Self::point(data_bounds, *xy);
            let distance =
                (position.x - p.x).as_f32().powi(2) + (position.y - p.y).as_f32().powi(2);
            if distance < best {
                nearest = Some(i);
                best = distance;
            }
        }
        nearest
    }
}

impl Render for ScatterChart {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = crate::theme::GewuPalette::get(cx);
        let ring_width = crate::theme::VisualCues::get(cx).cell_outline;
        let ring_color = if window.is_window_active() {
            colors.focus_ring
        } else {
            colors.muted_foreground
        };
        let base = Preferences::get(cx).row_number_base;
        let content = div()
            .w_full()
            .flex()
            .flex_none()
            .flex_col()
            .gap_1()
            .text_size(px(11.))
            .text_color(colors.muted_foreground)
            .font_features(crate::theme::tabular_digits());
        let Some(plot) = self.plot.clone() else {
            return content
                .child(if self.sample.non_positive_rows == 0 {
                    "No finite pairs to plot"
                } else {
                    "No finite pairs in the selected log domain"
                })
                .children(crate::ui::messages::omitted_pairs(
                    self.sample.non_positive_rows,
                ));
        };
        let handle = cx.weak_entity();
        let hovered = self.hover;
        let selected = self.selected;
        let chart = canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal).id,
            move |bounds, hitbox, window, cx| {
                let drawing = plot_bounds(bounds);
                let data = data_bounds(drawing);
                let font = window.text_style().font();
                let rule = px(1. / window.scale_factor());
                let mut previous_x_label = None;
                let mut draw_tick = |label: String,
                                     x: Pixels,
                                     y: Pixels,
                                     vertical: bool,
                                     window: &mut Window,
                                     cx: &mut App| {
                    let run = TextRun {
                        len: label.len(),
                        font: font.clone(),
                        color: colors.muted_foreground,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let line = window
                        .text_system()
                        .shape_line(label.into(), px(11.), &[run], None);
                    let left = if vertical {
                        x - px(3.) - line.width
                    } else {
                        x - line.width * 0.5
                    };
                    let tx = left
                        .max(bounds.left())
                        .min((bounds.right() - line.width).max(bounds.left()));
                    if !vertical {
                        if previous_x_label.is_some_and(|previous| tx < previous + TICK_LABEL_GAP) {
                            return;
                        }
                        previous_x_label = Some(tx + line.width);
                    }
                    let ty = tick_label_top(y, vertical);
                    let _ = line.paint(
                        point(tx, ty),
                        TICK_LINE_HEIGHT,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                };
                for tick in &plot.x.ticks {
                    let x = data.left() + data.size.width * tick.fraction;
                    window.paint_quad(fill(
                        Bounds::new(point(x, drawing.top()), size(rule, drawing.size.height)),
                        colors.chart_grid,
                    ));
                    draw_tick(
                        tick.label.text(base),
                        x,
                        drawing.bottom(),
                        false,
                        window,
                        cx,
                    );
                }
                for tick in &plot.y.ticks {
                    let y = data.bottom() - data.size.height * tick.fraction;
                    window.paint_quad(fill(
                        Bounds::new(point(drawing.left(), y), size(drawing.size.width, rule)),
                        colors.chart_grid,
                    ));
                    draw_tick(tick.label.text(base), drawing.left(), y, true, window, cx);
                }
                for (i, &xy) in plot.points.iter().enumerate() {
                    let p = Plot::point(data, xy);
                    let active = hovered == Some(i);
                    let r = px(if active { 3.5 } else { 2.5 });
                    window.paint_quad(
                        fill(
                            Bounds::new(point(p.x - r, p.y - r), size(r * 2., r * 2.)),
                            if active {
                                colors.chart_primary_hover
                            } else {
                                colors.chart_primary
                            },
                        )
                        .corner_radii(r),
                    );
                }
                if let Some(i) = selected {
                    let center = Plot::point(data, plot.points[i]);
                    let halo =
                        Bounds::new(center - point(px(7.5), px(7.5)), size(px(15.), px(15.)));
                    window.paint_quad(fill(halo, colors.canvas).corner_radii(px(7.5)));
                    let ring =
                        Bounds::new(center - point(px(6.5), px(6.5)), size(px(13.), px(13.)));
                    window.paint_quad(
                        outline(ring, ring_color, BorderStyle::Solid)
                            .border_widths(ring_width.max(px(1.75)))
                            .corner_radii(px(6.5)),
                    );
                    let core = Bounds::new(center - point(px(3.), px(3.)), size(px(6.), px(6.)));
                    window.paint_quad(fill(core, ring_color).corner_radii(px(3.)));
                }
                Self::bind_pointer(handle, plot, drawing, data, hitbox, window);
            },
        )
        .w_full()
        .aspect_ratio(4. / 3.);
        let count = crate::counted(self.sample.points.len() as u64, "point");
        let label = if let Some(i) = selected {
            let row = self.sample.points[i].source_row.0.saturating_add(base);
            format!("Scatter plot, {count}, row {row} selected")
        } else {
            format!("Scatter plot, {count}")
        };
        let mut figure = div()
                .id("scatter-plot")
                .w_full().flex_none()
                .track_focus(&self.focus)
                .focus_visible(move |style| style.shadow(colors.focus_shadows()))
                .role(Role::Image)
                .aria_label(label)
                .aria_description("Use Left and Right to inspect points. Press Return to reveal the point in the table.")
                .on_key_down(cx.listener(|chart, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.modifiers.modified() { return; }
                    match event.keystroke.key.as_str() {
                        "left" => chart.step(false, cx),
                        "right" => chart.step(true, cx),
                        "enter" | "space" => chart.reveal(chart.hover.unwrap_or(0), window, cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                }));
        for action in [
            AccessibleAction::Increment,
            AccessibleAction::Decrement,
            AccessibleAction::Click,
        ] {
            let chart = cx.weak_entity();
            figure = figure.on_a11y_action(action, move |_, w, cx| {
                let _ = chart.update(cx, |chart, cx| {
                    if action == AccessibleAction::Click {
                        chart.reveal(chart.hover.unwrap_or(0), w, cx);
                    } else {
                        chart.step(action == AccessibleAction::Increment, cx);
                    }
                });
            });
        }
        let readout = self.readout(base);
        if let Some(text) = &readout {
            figure = figure.aria_value(text.clone());
        }
        content
            .child(figure.child(chart).test_support())
            .children(
                self.axis_context
                    .clone()
                    .map(|context| div().w_full().truncate().child(context)),
            )
            .children(crate::ui::messages::omitted_pairs(
                self.sample.non_positive_rows,
            ))
    }
}

impl ScatterChart {
    fn bind_pointer(
        entity: WeakEntity<Self>,
        plot: Arc<Plot>,
        hit_bounds: Bounds<Pixels>,
        data_bounds: Bounds<Pixels>,
        hitbox: HitboxId,
        window: &mut Window,
    ) {
        let nearest = move |position, window: &Window| {
            hitbox
                .is_hovered(window)
                .then(|| plot.nearest(position, hit_bounds, data_bounds))
                .flatten()
        };
        let hit = nearest.clone();
        let chart = entity.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, w, cx| {
            if phase.bubble() && event.button == MouseButton::Left {
                let press = hit(event.position, w).map(|index| (index, event.position));
                let _ = chart.update(cx, |chart, _| chart.press = press);
            }
        });
        let hit = nearest.clone();
        let chart = entity.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, w, cx| {
            if !phase.bubble() {
                return;
            }
            let index = hit(event.position, w);
            let _ = chart.update(cx, |chart, cx| {
                if let Some((_, down)) = chart.press
                    && (event.position - down).magnitude() > 3.
                {
                    chart.press = None;
                }
                if chart.hover != index {
                    chart.hover = index;
                    cx.notify();
                }
            });
        });
        window.on_mouse_event(move |event: &MouseUpEvent, phase, w, cx| {
            if !phase.bubble() || event.button != MouseButton::Left {
                return;
            }
            let _ = entity.update(cx, |chart, cx| {
                if let Some((index, down)) = chart.press.take()
                    && (event.position - down).magnitude() <= 3.
                    && nearest(event.position, w) == Some(index)
                {
                    chart.reveal(index, w, cx);
                }
            });
        });
    }
}

#[cfg(test)]
#[path = "scatter_tests.rs"]
mod tests;
