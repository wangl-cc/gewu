//! Compare display-paced scrolling and independently replay the grid into a Metal texture.
use std::{
    cell::{Cell, RefCell},
    error::Error,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gewu_core::Dataset;
use gewu_gpui::examples::Document;
use gpui_kit::{test::TestWindowExt, *};

#[path = "scroll/frame.rs"]
mod frame;
mod support;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const MODES: [&str; 3] = ["slow", "fast", "jump"];
const WARMUP: Duration = Duration::from_secs(1);

struct Options {
    path: PathBuf,
    count: u32,
    grid: Option<Size<Pixels>>,
    verify: bool,
    capture: Option<PathBuf>,
}

impl Options {
    fn read() -> Result<Self> {
        let mut args = std::env::args_os().skip(1).peekable();
        let verify = args.next_if(|v| v == "--verify").is_some();
        let path = args.next().ok_or("expected a dataset path")?.into();
        let number = |v: std::ffi::OsString| -> Result<u32> {
            Ok(v.to_str().ok_or("invalid number")?.parse()?)
        };
        let count = args
            .next()
            .map(number)
            .transpose()?
            .unwrap_or(if verify { 60 } else { 10 });
        if count == 0 {
            return Err("seconds or frames per mode must be positive".into());
        }
        let grid = if args.next_if(|v| v == "fullscreen").is_some() {
            None
        } else {
            let width = args.next().map(number).transpose()?.unwrap_or(1280);
            let height = args.next().map(number).transpose()?.unwrap_or(800);
            Some(size(px(width as f32), px(height as f32)))
        };
        let capture = args.next().map(PathBuf::from);
        if args.next().is_some() || (!verify && capture.is_some()) {
            return Err(
                "usage: scroll [--verify] FILE [COUNT [WIDTH HEIGHT | fullscreen [PNG]]]".into(),
            );
        }
        Ok(Self {
            path,
            count,
            grid,
            verify,
            capture,
        })
    }
}

struct Workload {
    row: u64,
    random: u64,
    limit: u64,
}

impl Workload {
    fn new(rows: u64) -> Self {
        Self {
            row: 0,
            random: 0x1234_5678,
            limit: rows.saturating_sub(30).max(1),
        }
    }

    fn advance(&mut self, mode: usize) -> u64 {
        self.row = match mode {
            0 => (self.row + 1) % self.limit,
            1 => (self.row + 37) % self.limit,
            _ => {
                self.random = self.random.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut z = self.random;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                (z ^ (z >> 31)) % self.limit
            }
        };
        self.row
    }
}

#[derive(Default)]
struct Samples {
    pending: Cell<bool>,
    paints: RefCell<Vec<(Instant, Duration)>>,
}

struct Driver {
    document: Document,
    content_inset: Size<Pixels>,
    samples: Rc<Samples>,
    phase: usize,
    submitted: usize,
    start: Instant,
    duration: Duration,
    workload: Workload,
    viewport: Size<Pixels>,
    scale: f32,
    interrupted: bool,
}

impl Driver {
    fn new(document: Document, rows: u64, duration: Duration) -> Self {
        Self {
            document,
            content_inset: Size::default(),
            samples: Rc::default(),
            phase: 0,
            submitted: 0,
            start: Instant::now(),
            duration,
            workload: Workload::new(rows),
            viewport: Size::default(),
            scale: 1.,
            interrupted: false,
        }
    }

    fn fit_grid(
        &mut self,
        grid: Size<Pixels>,
        target: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Keep the native content size in whole points. The production document's
        // chrome can occupy half a point on Retina, so the host absorbs the remainder.
        let viewport = window.viewport_size();
        let document = viewport - self.content_inset + target - grid;
        let outer = |needed: Pixels, actual: Pixels| {
            if actual >= needed && actual - needed < px(1.) {
                actual
            } else {
                px(needed.as_f32().ceil())
            }
        };
        let window_size = size(
            outer(document.width, viewport.width),
            outer(document.height, viewport.height),
        );
        let inset = window_size - document;
        if self.content_inset != inset {
            self.content_inset = inset;
            cx.notify();
        }
        if viewport != window_size {
            window.resize(window_size);
        }
    }

    fn schedule(this: Entity<Self>, window: &mut Window) {
        window.on_next_frame(move |window, cx| {
            let again = this.update(cx, |driver, cx| match driver.tick(window, cx) {
                Ok(again) => again,
                Err(error) => {
                    support::fail(error, cx);
                    false
                }
            });
            if again {
                Self::schedule(this, window);
            }
        });
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<bool> {
        if self.interrupted
            || !window.is_window_active()
            || !window.is_visible()
            || window.viewport_size() != self.viewport
            || window.scale_factor() != self.scale
        {
            return Err("window lost focus or changed size/scale during the workload".into());
        }
        if self.samples.pending.get() {
            return Err(
                "a display callback arrived before the preceding scroll was painted".into(),
            );
        }
        let now = Instant::now();
        if now - self.start >= self.duration + WARMUP {
            let paints = self.samples.paints.take();
            if paints.len() < 2 {
                return Err("fewer than two measured paints".into());
            }
            let mut intervals = paints
                .windows(2)
                .map(|p| p[1].0 - p[0].0)
                .collect::<Vec<_>>();
            let measured = (now - self.start - WARMUP).as_secs_f64();
            let fps = paints.len() as f64 / measured;
            let mut render = paints.iter().map(|p| p.1).collect::<Vec<_>>();
            println!(
                "mode={} frames={} submitted={} fps={fps:.2} interval_ms={} render_ms={}",
                MODES[self.phase],
                paints.len(),
                self.submitted,
                percentiles(&mut intervals),
                percentiles(&mut render)
            );
            self.phase += 1;
            if self.phase == MODES.len() {
                cx.quit();
                return Ok(false);
            }
            self.start = now;
            self.workload.row = 0;
            self.submitted = 0;
        }
        self.samples.pending.set(now - self.start >= WARMUP);
        let row = self.workload.advance(self.phase);
        self.submitted += 1;
        self.document.scroll_to_row(row, cx);
        cx.notify();
        Ok(true)
    }
}

fn percentiles(durations: &mut [Duration]) -> String {
    durations.sort_unstable();
    [50, 95, 99, 100]
        .map(|p| {
            format!(
                "{:.2}",
                durations[(durations.len() - 1) * p / 100].as_secs_f64() * 1000.
            )
        })
        .join("/")
}

impl Render for Driver {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let start = Instant::now();
        let samples = self.samples.clone();
        div()
            .size_full()
            .pr(self.content_inset.width)
            .pb(self.content_inset.height)
            .relative()
            .child(self.document.view())
            .child(
                canvas(
                    |_, _, _| (),
                    move |_, _, _, _| {
                        if samples.pending.replace(false) {
                            let now = Instant::now();
                            samples.paints.borrow_mut().push((now, now - start));
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
    }
}

async fn run(
    opts: Options,
    dataset: Arc<Dataset>,
    driver: Entity<Driver>,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let pause = Duration::from_millis(120);
    for _ in 0..500 {
        if driver.read_with(cx, |d, cx| d.document.is_loaded(cx)) {
            break;
        }
        cx.background_executor().timer(pause).await;
    }
    if opts.grid.is_none() {
        cx.update(|window, _| window.toggle_simple_fullscreen())?;
    }
    let mut geometry = None;
    let mut observed = String::new();
    for _ in 0..40 {
        cx.background_executor().timer(pause).await;
        geometry = cx.update(|window, cx| -> Result<_> {
            let viewport = window.viewport_size();
            let active = window.is_window_active();
            let visible = window.is_visible();
            let ready = opts.verify || active && visible;
            let grid = if ready {
                window.render_frame(cx);
                frame::Grid::read(window)
            } else {
                Err("waiting for an active visible window".into())
            };
            let detail = match grid.as_ref() {
                Ok(g) => format!("{:?}", g.size),
                Err(error) => error.to_string(),
            };
            let last = format!(
                "grid={detail} window={viewport:?} active={active} visible={visible} scale={}",
                window.scale_factor()
            );
            if observed != last {
                println!("setup {last}");
                observed = last;
            }
            let Ok(g) = grid else {
                if !ready {
                    window.activate_window();
                }
                return Ok(None);
            };
            if let Some(target) = opts.grid {
                if g.size != target {
                    driver.update(cx, |d, cx| d.fit_grid(g.size, target, window, cx));
                    return Ok(None);
                }
            } else {
                let display = window
                    .display(cx)
                    .ok_or("no display assigned to the window")?;
                if !window.is_simple_fullscreen() || window.viewport_size() != display.bounds().size
                {
                    return Ok(None);
                }
            }
            Ok(Some(g))
        })??;
        if geometry.is_some() {
            break;
        }
    }
    let geometry = geometry.ok_or_else(|| format!(
        "could not obtain an active visible window at the requested grid size {:?} or fullscreen mode: {observed}", opts.grid
    ))?;
    let rows = dataset.n_rows() as u64;
    cx.update(|w,cx| println!("pid={} backend={} dataset={}x{} grid={:?} window={:?} scale={} fullscreen={} refresh_interval={:?} row_height={} full_cells={}x{}",
        std::process::id(), if cfg!(feature="gpui-fast") {"gpui-fast"} else {"gpui-pre"}, rows, dataset.schema().fields().len(),
        geometry.size, w.viewport_size(), w.scale_factor(), w.is_simple_fullscreen(), w.refresh_interval(cx),
        geometry.row_height as f32 / w.scale_factor(), geometry.full_rows(), geometry.columns.len()))?;
    if opts.verify {
        let view = dataset.unfiltered().unsorted();
        let jump_capture = opts.capture.as_ref().map(|p| p.with_extension("jump.png"));
        for (mode, label) in MODES.iter().enumerate() {
            let mut workload = Workload::new(rows);
            let mut checked = 0;
            let mut draw_cost = Vec::new();
            for index in 0..opts.count {
                let row = workload.advance(mode);
                let capture = match (mode, index) {
                    (0, 0) => opts.capture.as_deref(),
                    (2, 0) => jump_capture.as_deref(),
                    _ => None,
                };
                let (cells, cost) = cx.update(|window, cx| -> Result<_> {
                    let start = Instant::now();
                    driver.update(cx, |d, cx| d.document.scroll_to_row(row, cx));
                    window.render_frame(cx);
                    let cost = start.elapsed();
                    let cells = geometry.verify(window, &view, row, capture)?;
                    Ok((cells, cost))
                })??;
                checked += cells;
                draw_cost.push(cost);
                if let Some(path) = capture {
                    println!("capture={} first_requested_row={row}", path.display());
                }
            }
            println!(
                "mode={label} replay_frames={} inspected_cells={checked} unexpected_blanks=0 forced_draw_ms={}",
                opts.count,
                percentiles(&mut draw_cost)
            );
        }
        if let Some(path) = opts.capture {
            cx.update(|window, cx| -> Result<()> {
                driver.update(cx, |d, cx| d.document.scroll_to_row(1, cx));
                window.render_frame(cx);
                geometry.capture_drag(window, cx, &path.with_extension("selection.png"))
            })??;
        }
        cx.update(|_, cx| cx.quit())?;
    } else {
        println!("settling seconds=5");
        cx.background_executor().timer(Duration::from_secs(5)).await;
        cx.update(|w, cx| -> Result<()> {
            if !w.is_window_active() || !w.is_visible() {
                return Err("benchmark window is inactive".into());
            }
            driver.update(cx, |d, cx| {
                d.viewport = w.viewport_size();
                d.scale = w.scale_factor();
                cx.observe_window_activation(w, |d, w, _| d.interrupted |= !w.is_window_active())
                    .detach();
                cx.observe_window_visibility(w, |d, state, _, _| {
                    d.interrupted |= !state.is_visible()
                })
                .detach();
                cx.observe_window_bounds(w, |d, w, _| {
                    d.interrupted |= w.viewport_size() != d.viewport || w.scale_factor() != d.scale
                })
                .detach();
            });
            Ok(())
        })??;
        println!("idle seconds={}", opts.count);
        cx.background_executor()
            .timer(Duration::from_secs(u64::from(opts.count)))
            .await;
        cx.update(|w, cx| {
            driver.update(cx, |d, _| {
                d.start = Instant::now();
            });
            println!("bench: ready");
            Driver::schedule(driver.clone(), w);
        })?;
        cx.background_executor()
            .timer(
                (Duration::from_secs(u64::from(opts.count)) + WARMUP) * 3 + Duration::from_secs(5),
            )
            .await;
        return Err("timed out waiting for native display callbacks".into());
    }
    Ok(())
}

fn main() -> Result<()> {
    let opts = Options::read()?;
    let dataset = Arc::new(Dataset::open(&opts.path)?);
    if dataset.n_rows() < 100 || dataset.schema().fields().len() < 2 {
        return Err("scrolling benchmark requires at least 100 rows and two columns".into());
    }
    gpui_kit::application().run(move |cx| {
        gewu_gpui::examples::init(cx);
        cx.set_activation_policy(ActivationPolicy::Regular);
        cx.activate(true);
        let bounds = Bounds::centered(None, opts.grid.unwrap_or(size(px(1280.), px(800.))), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let opened = gpui_kit::open_window(options, cx, |window, cx| {
            let document = Document::open(opts.path.clone(), window, cx);
            let driver = cx.new(|_| {
                Driver::new(
                    document,
                    dataset.n_rows() as u64,
                    Duration::from_secs(u64::from(opts.count)),
                )
            });
            let task = driver.clone();
            window
                .spawn(cx, async move |cx| {
                    if let Err(error) = run(opts, dataset, task, cx).await {
                        AsyncApp::update(cx, |cx| support::fail(error, cx));
                    }
                })
                .detach();
            driver
        });
        if let Err(error) = opened {
            support::fail(error, cx);
        } else {
            cx.activate(true);
        }
    });
    support::result()
}

#[cfg(test)]
#[path = "scroll/tests.rs"]
mod tests;
