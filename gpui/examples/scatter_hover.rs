//! Replay pointer moves against the production table Inspector and verify each plotted row.
//! Pass --display to dispatch on native frame callbacks in an active window.
use std::{
    io::Write,
    sync::Arc,
    time::{Duration, Instant},
};

use gewu_core::{AxisScale, Dataset};
use gewu_gpui::TableView;
use gpui_kit::{test::TestWindowExt, *};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const POINTS: usize = 2048;
const STEPS: usize = 360;
const ROWS: usize = 8192;

mod support;

struct Measurement {
    sampled_rows: Vec<u64>,
    bounds: Bounds<Pixels>,
    latency: Vec<Duration>,
    events: Vec<Instant>,
    step: usize,
}

impl Measurement {
    fn step(&mut self, w: &mut Window, cx: &mut App) -> Result<()> {
        let row = self.sampled_rows[(self.step * 47 + 31) % POINTS];
        let lo = self.sampled_rows[0];
        let hi = self.sampled_rows[POINTS - 1];
        let f = (row - lo) as f32 / (hi - lo) as f32;
        let b = self.bounds;
        let position = point(
            b.left() + px(50.) + (b.size.width - px(58.)) * f,
            b.top() + px(8.) + (b.size.height - px(30.)) * (1. - f),
        );
        let start = Instant::now();
        w.dispatch_event(
            MouseMoveEvent {
                position,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        w.render_frame(cx);
        let elapsed = start.elapsed();
        let readout = w
            .try_find("scatter-readout")
            .ok_or("hover produced no readout")?;
        let expected = format!("Row {row} · X {row} · Y {row}");
        if readout.label() != Some(expected.as_str()) {
            return Err(format!(
                "at step {} expected {expected}, got {:?}",
                self.step,
                readout.label()
            )
            .into());
        }
        // Warm caches before collecting timings; every hover must still update the exact row.
        if self.step >= 20 {
            self.latency.push(elapsed);
            self.events.push(start);
        }
        self.step += 1;
        Ok(())
    }

    fn report(&mut self, display: bool) {
        println!(
            "source_rows={ROWS} points={POINTS} verified={} measured={} dispatch_and_draw_ms_p50_p95_p99_max={}",
            self.step,
            self.latency.len(),
            percentiles(&mut self.latency)
        );
        if display {
            let mut intervals = self
                .events
                .windows(2)
                .map(|v| v[1] - v[0])
                .collect::<Vec<_>>();
            println!(
                "display_callback_interval_ms_p50_p95_p99_max={}",
                percentiles(&mut intervals)
            );
        }
    }

    fn schedule(mut self, w: &mut Window) {
        w.on_next_frame(move |w, cx| {
            if !w.is_window_active() {
                support::fail("display-paced hover requires an active window", cx);
                return;
            }
            if let Err(e) = self.step(w, cx) {
                support::fail(e, cx);
                return;
            }
            if self.step == STEPS {
                self.report(true);
                cx.quit();
            } else {
                self.schedule(w);
            }
        });
    }
}

fn percentiles(data: &mut [Duration]) -> String {
    data.sort_unstable();
    [50, 95, 99, 100]
        .map(|p| {
            format!(
                "{:.3}",
                data[(data.len() - 1) * p / 100].as_secs_f64() * 1000.
            )
        })
        .join("/")
}

async fn run(display: bool, sampled_rows: Vec<u64>, cx: &mut AsyncWindowContext) -> Result<()> {
    cx.update(|w, cx| -> Result<()> {
        w.render_frame(cx);
        let action = cx.build_action("gewu::ShowInspector", None)?;
        w.dispatch_action(action, cx);
        Ok(())
    })??;
    let mut tab = false;
    let mut geometry = None;
    for _ in 0..100 {
        cx.background_executor()
            .timer(Duration::from_millis(80))
            .await;
        geometry = cx.update(|w, cx| -> Result<_> {
            w.render_frame(cx);
            if !tab && w.try_find("Chart").is_some() {
                w.click("Chart", cx);
                tab = true;
            }
            let Some(plot) = w.try_find("scatter-plot") else {
                return Ok(None);
            };
            if plot.label() != Some("Scatter plot, 2048 points") {
                return Err(format!("unexpected sample {:?}", plot.label()).into());
            }
            println!(
                "mode={} viewport={:?} plot={:?} scale={} active={} refresh_interval={:?}",
                if display { "display" } else { "replay" },
                w.viewport_size(),
                plot.bounds().size,
                w.scale_factor(),
                w.is_window_active(),
                w.refresh_interval(cx)
            );
            if display && !w.is_window_active() {
                return Err("display-paced hover requires an active window".into());
            }
            Ok(Some(plot.bounds()))
        })??;
        if geometry.is_some() {
            break;
        }
    }
    let mut measurement = Measurement {
        sampled_rows,
        bounds: geometry.ok_or("scatter plot did not appear")?,
        latency: Vec::new(),
        events: Vec::new(),
        step: 0,
    };
    if display {
        cx.update(|w, _| measurement.schedule(w))?;
        cx.background_executor()
            .timer(Duration::from_secs(25))
            .await;
        Err("display callbacks timed out".into())
    } else {
        cx.update(|w, cx| -> Result<()> {
            for _ in 0..STEPS {
                measurement.step(w, cx)?;
            }
            let image = w.render_to_image()?;
            println!("final_metal_texture={}x{}", image.width(), image.height());
            measurement.report(false);
            cx.quit();
            Ok(())
        })??;
        Ok(())
    }
}

fn main() -> Result<()> {
    let display = match std::env::args().nth(1).as_deref() {
        None => false,
        Some("--display") => true,
        _ => return Err("usage: scatter_hover [--display]".into()),
    };
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("points.csv");
    let mut csv = std::io::BufWriter::new(std::fs::File::create(&path)?);
    writeln!(csv, "number")?;
    for i in 0..ROWS {
        writeln!(csv, "{i}")?;
    }
    csv.flush()?;
    let dataset = Arc::new(Dataset::open(path)?);
    let sample = dataset.unfiltered().scatter(
        None,
        dataset.info().columns[0].id,
        POINTS as u32,
        AxisScale::Linear,
        AxisScale::Linear,
    )?;
    if sample.eligible_rows != ROWS as u64 || sample.points.len() != POINTS {
        return Err("unexpected number of eligible or sampled rows".into());
    }
    let mut sampled_rows: Vec<_> = sample.points.iter().map(|p| p.source_row).collect();
    sampled_rows.sort_unstable();
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        cx.set_activation_policy(ActivationPolicy::Regular);
        cx.activate(true);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1280.), px(800.)),
                cx,
            ))),
            ..Default::default()
        };
        if let Err(e) = gpui_kit::open_window(options, cx, |w, cx| {
            w.set_window_title("Gewu scatter hover benchmark");
            let table = cx.new(|cx| TableView::from_dataset(dataset.clone(), w, cx));
            let focus = table.read(cx).focus.clone();
            w.focus(&focus, cx);
            w.spawn(cx, async move |cx| {
                if let Err(e) = run(display, sampled_rows, cx).await {
                    AsyncApp::update(cx, |cx| support::fail(e, cx));
                }
            })
            .detach();
            table
        }) {
            support::fail(e, cx);
        } else {
            cx.activate(true);
        }
    });
    support::result()
}
