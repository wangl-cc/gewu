use std::sync::Arc;

use gewu_core::{AxisScale, ScatterResult};
use gpui_kit::{
    base::{self, TestSupportExt},
    *,
};

use super::axis::{Axis, seconds};
use crate::{actions::RevealScatterPoint, preferences::Preferences};

struct Plot {
    x: Axis,
    y: Axis,
    points: Vec<(f32, f32)>,
}

pub(in crate::panels) struct ScatterChart {
    sample: Arc<ScatterResult>,
    source_x: bool,
    x_title: SharedString,
    y_title: SharedString,
    plot: Option<Arc<Plot>>,
    hover: Option<usize>,
    press: Option<(usize, Point<Pixels>)>,
    focus: FocusHandle,
}

fn plot_bounds(bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    let left = px(50.).min(bounds.size.width);
    let top = px(8.).min(bounds.size.height);
    Bounds::new(
        bounds.origin + point(left, top),
        size(
            (bounds.size.width - left - px(8.)).max(px(0.)),
            (bounds.size.height - top - px(22.)).max(px(0.)),
        ),
    )
}

impl ScatterChart {
    pub fn new(
        sample: Arc<ScatterResult>,
        x_scale: AxisScale,
        y_scale: AxisScale,
        x_name: Option<&str>,
        y_name: &str,
        cx: &App,
    ) -> Self {
        let source_x = x_name.is_none();
        let xs: Vec<_> = sample
            .points
            .iter()
            .map(|p| {
                if source_x {
                    p.source_row as f64
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
            3,
        );
        let y = Axis::for_points(
            &ys,
            |i| sample.points[i].y_label.clone(),
            sample.y_axis_kind,
            y_scale,
            false,
            4,
        );
        let title = |axis: &str, name: &str, model: &Option<Axis>| {
            let context = model.as_ref().and_then(|a| a.context.as_deref());
            match context {
                Some(context) => format!("{axis} · {name} · {context}"),
                None => format!("{axis} · {name}"),
            }
            .into()
        };
        let x_title = title("X", x_name.unwrap_or("Source row"), &x);
        let y_title = title("Y", y_name, &y);
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
            x_title,
            y_title,
            plot,
            hover: None,
            press: None,
            focus: cx.focus_handle().tab_stop(true),
        }
    }

    fn reveal(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.dispatch_action(
            &RevealScatterPoint {
                chart: cx.entity_id(),
                source_row: gewu_core::SourceRow(self.sample.points[index].source_row),
            },
            window,
            cx,
        );
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

    fn nearest(&self, position: Point<Pixels>, bounds: Bounds<Pixels>) -> Option<usize> {
        if !bounds.contains(&position) {
            return None;
        }
        let mut nearest = None;
        let mut best = 10f32.powi(2);
        for (i, xy) in self.points.iter().enumerate() {
            let p = Self::point(bounds, *xy);
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = base::Theme::global(cx).tokens.colors;
        let base = Preferences::get(cx).row_number_base;
        let mut content = div()
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
                .child(format!(
                    "{} nonpositive pairs omitted",
                    self.sample.non_positive_rows
                ));
        };
        let handle = cx.weak_entity();
        let hovered = self.hover;
        let chart = canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal).id,
            move |bounds, hitbox, window, cx| {
                let drawing = plot_bounds(bounds);
                let font = window.text_style().font();
                let rule = px(1. / window.scale_factor());
                let draw_tick = |label: String,
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
                        .shape_line(label.into(), px(10.), &[run], None);
                    let left = if vertical {
                        x - px(3.) - line.width
                    } else {
                        x - line.width * 0.5
                    };
                    let tx = left
                        .max(bounds.left())
                        .min((bounds.right() - line.width).max(bounds.left()));
                    let ty = if vertical { y - px(7.) } else { y + px(2.) };
                    let _ = line.paint(point(tx, ty), px(14.), TextAlign::Left, None, window, cx);
                };
                for tick in &plot.x.ticks {
                    let x = drawing.left() + drawing.size.width * tick.fraction;
                    window.paint_quad(fill(
                        Bounds::new(point(x, drawing.top()), size(rule, drawing.size.height)),
                        colors.border,
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
                    let y = drawing.bottom() - drawing.size.height * tick.fraction;
                    window.paint_quad(fill(
                        Bounds::new(point(drawing.left(), y), size(drawing.size.width, rule)),
                        colors.border,
                    ));
                    draw_tick(tick.label.text(base), drawing.left(), y, true, window, cx);
                }
                for (i, &xy) in plot.points.iter().enumerate() {
                    let p = Plot::point(drawing, xy);
                    let active = hovered == Some(i);
                    let r = px(if active { 3.5 } else { 2.5 });
                    window.paint_quad(
                        fill(
                            Bounds::new(point(p.x - r, p.y - r), size(r * 2., r * 2.)),
                            if active {
                                colors.primary
                            } else {
                                colors.primary.opacity(0.60)
                            },
                        )
                        .corner_radii(r),
                    );
                }
                Self::bind_pointer(handle, plot, drawing, hitbox, window);
            },
        )
        .w_full()
        .aspect_ratio(4. / 3.);
        let mut figure = div()
                .id("scatter-plot")
                .w_full().flex_none()
                .track_focus(&self.focus)
                .focus_visible(move |style| style.border_1().border_color(colors.ring))
                .role(Role::Image)
                .aria_label(format!("Scatter plot, {} points", self.sample.points.len()))
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
        let readout = self.hover.map(|i| {
            let point = &self.sample.points[i];
            let row = point.source_row.saturating_add(base);
            let x = if self.source_x {
                row.to_string()
            } else {
                point.x_label.clone()
            };
            SharedString::from(format!("Row {row} · X {x} · Y {}", point.y_label))
        });
        if let Some(text) = &readout {
            figure = figure.aria_value(text.clone());
        }
        content = content
            .child(div().w_full().truncate().child(self.y_title.clone()))
            .child(figure.child(chart).test_support())
            .child(
                div()
                    .w_full()
                    .truncate()
                    .text_center()
                    .child(self.x_title.clone()),
            );
        content = content.child(format!(
            "{} sampled of {} pairs",
            self.sample.points.len(),
            self.sample.eligible_rows
        ));
        if self.sample.non_positive_rows > 0 {
            content = content.child(format!(
                "{} nonpositive pairs omitted",
                self.sample.non_positive_rows
            ));
        }
        if let Some(readout) = readout {
            content = content.child(
                div()
                    .id("scatter-readout")
                    .text_color(colors.foreground)
                    .aria_label(readout.clone())
                    .child(readout)
                    .test_support(),
            );
        }
        content
    }
}

impl ScatterChart {
    fn bind_pointer(
        entity: WeakEntity<Self>,
        plot: Arc<Plot>,
        bounds: Bounds<Pixels>,
        hitbox: HitboxId,
        window: &mut Window,
    ) {
        let nearest = move |position, window: &Window| {
            hitbox
                .is_hovered(window)
                .then(|| plot.nearest(position, bounds))
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
