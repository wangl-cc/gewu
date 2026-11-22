//! Benchmark drivers: scrolling once per frame from `on_next_frame`, and a
//! windowless timing of fetching one screen of text from Core.

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui_kit::*;

use crate::{
    data::TableData,
    table::{COLUMN_WIDTH, HEADER_HEIGHT, ROW_HEIGHT, TableView},
};

const WARMUP: f64 = 1.0;
const MODES: [&str; 3] = ["(a) 1 row/frame", "(b) 37 rows/frame", "(c) random jump"];

#[derive(Default)]
struct Samples {
    intervals: Vec<Duration>,
    render: Vec<Duration>,
    fetch: Vec<Duration>,
    missing: usize,
}

struct State {
    table: Entity<TableView>,
    secs: f64,
    sort_column: Option<String>,
    phase: usize, // 0 settle, 1..=3 modes, 4 sort, 5 done
    phase_start: Instant,
    row: u64,
    rng: u64,
    last_paint: Option<Instant>,
    samples: Samples,
    results: Vec<(String, Samples)>,
    sort_started: Option<Instant>,
}

pub fn start(
    table: Entity<TableView>,
    secs: f64,
    sort_column: Option<String>,
    window: &mut Window,
    cx: &mut App,
) {
    table.read(cx).probe.borrow_mut().enabled = true;
    let state = Rc::new(RefCell::new(State {
        table,
        secs,
        sort_column,
        phase: 0,
        phase_start: Instant::now(),
        row: 0,
        rng: 0x1234_5678,
        last_paint: None,
        samples: Samples::default(),
        results: vec![],
        sort_started: None,
    }));
    schedule(state, window);
}

fn schedule(state: Rc<RefCell<State>>, window: &mut Window) {
    window.on_next_frame(move |window, cx| {
        if step(&state, window, cx) {
            schedule(state, window);
        }
    });
}

fn mix(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// Returns whether to keep driving.
fn step(state: &Rc<RefCell<State>>, window: &mut Window, cx: &mut App) -> bool {
    let mut s = state.borrow_mut();
    let now = Instant::now();
    let table = s.table.clone();
    // Collect the frames painted since the last step.
    let frames = std::mem::take(&mut table.read(cx).probe.borrow_mut().frames);
    let measuring = (1..=4).contains(&s.phase)
        && (s.phase == 4 || now.duration_since(s.phase_start).as_secs_f64() >= WARMUP);
    for (start, end, fetch, missing) in frames {
        if measuring {
            if let Some(prev) = s.last_paint {
                s.samples.intervals.push(end - prev);
            }
            s.samples.render.push(end - start);
            s.samples.fetch.push(fetch);
            s.samples.missing += missing;
        }
        s.last_paint = Some(end);
    }
    let elapsed = now.duration_since(s.phase_start).as_secs_f64();
    match s.phase {
        0 => {
            let key = window.is_window_active();
            if elapsed > 0.5 && (key || elapsed > 10.0) {
                println!(
                    "bench: ready, window active={key}, viewport={:?}",
                    window.viewport_size()
                );
                next_phase(&mut s, 1);
            }
        }
        1..=3 if elapsed >= WARMUP + s.secs => {
            let label = MODES[s.phase - 1].to_string();
            let samples = std::mem::take(&mut s.samples);
            s.results.push((label, samples));
            let next = if s.phase == 3 && s.sort_column.is_none() {
                5
            } else {
                s.phase + 1
            };
            next_phase(&mut s, next);
            if next == 4 {
                let name = s.sort_column.clone().unwrap();
                table.update(cx, |t, cx| {
                    let column = t
                        .data
                        .columns
                        .iter()
                        .position(|c| c.name == name)
                        .expect("sort column");
                    t.scroll_to_row(0);
                    t.cycle_sort(column, cx);
                });
                s.sort_started = Some(Instant::now());
            }
        }
        4 if table.read(cx).sorting.is_none() => {
            let took = table.read(cx).last_sort;
            let label = format!(
                "sort by {} (scroll 1 row/frame meanwhile)",
                s.sort_column.as_deref().unwrap_or("?")
            );
            let samples = std::mem::take(&mut s.samples);
            s.results.push((label, samples));
            println!("sort: total {:?} (request to installed view)", took);
            next_phase(&mut s, 5);
        }
        _ => {}
    }
    if s.phase == 5 {
        report(&s, &table, window, cx);
        cx.quit();
        return false;
    }
    if (1..=4).contains(&s.phase) {
        let rows = table.read(cx).data.row_count();
        let max = rows.saturating_sub(24).max(1);
        s.row = match s.phase {
            1 | 4 => (s.row + 1) % max,
            2 => (s.row + 37) % max,
            _ => mix(&mut s.rng) % max,
        };
        let row = s.row;
        table.update(cx, |t, cx| {
            t.scroll_to_row(row);
            cx.notify();
        });
    } else {
        table.update(cx, |_, cx| cx.notify());
    }
    true
}

fn next_phase(s: &mut State, phase: usize) {
    s.phase = phase;
    s.phase_start = Instant::now();
    s.row = 0;
    s.last_paint = None;
    s.samples = Samples::default();
}

fn q(v: &[Duration], p: f64) -> Duration {
    if v.is_empty() {
        return Duration::ZERO;
    }
    let mut v = v.to_vec();
    v.sort();
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn ms(d: Duration) -> String {
    format!("{:.2}", d.as_secs_f64() * 1e3)
}

fn report(s: &State, table: &Entity<TableView>, window: &Window, cx: &App) {
    let backend = if cfg!(feature = "gpui-fast") {
        "gpui-fast"
    } else {
        "gpui-pre"
    };
    let t = table.read(cx);
    println!(
        "\n== backend={backend} rows={} viewport={:?} scale={} visible rows x cols={:?} measured {}s/mode (after {WARMUP}s warmup), window active at end: {}",
        t.data.row_count(),
        window.viewport_size(),
        window.scale_factor(),
        t.visible_counts(),
        s.secs,
        window.is_window_active()
    );
    println!(
        "{:<44} {:>6} {:>6} | {:>7} {:>7} {:>7} {:>7} | {:>7} {:>7} {:>7} {:>7} | {:>7} {:>7} {:>7} | {:>5} | {:>5}",
        "mode",
        "frames",
        "fps",
        "int p50",
        "int p95",
        "int p99",
        "int max",
        "rnd p50",
        "rnd p95",
        "rnd p99",
        "rnd max",
        "core50",
        "core95",
        "coremax",
        ">20ms",
        "blank"
    );
    for (label, x) in &s.results {
        let total: Duration = x.intervals.iter().sum();
        let fps = x.intervals.len() as f64 / total.as_secs_f64().max(1e-9);
        let slow = x.intervals.iter().filter(|d| d.as_millis() >= 20).count();
        println!(
            "{:<44} {:>6} {:>6.1} | {:>7} {:>7} {:>7} {:>7} | {:>7} {:>7} {:>7} {:>7} | {:>7} {:>7} {:>7} | {:>5} | {:>5}",
            label,
            x.intervals.len(),
            fps,
            ms(q(&x.intervals, 0.5)),
            ms(q(&x.intervals, 0.95)),
            ms(q(&x.intervals, 0.99)),
            ms(q(&x.intervals, 1.0)),
            ms(q(&x.render, 0.5)),
            ms(q(&x.render, 0.95)),
            ms(q(&x.render, 0.99)),
            ms(q(&x.render, 1.0)),
            ms(q(&x.fetch, 0.5)),
            ms(q(&x.fetch, 0.95)),
            ms(q(&x.fetch, 1.0)),
            slow,
            x.missing
        );
    }
    println!(
        "(int = interval between painted frames; rnd = table render -> end of its paint, incl. Core fetch, text shaping, glyph rasterization; core = time in format_column for the frame; blank = visible cells without text, summed)"
    );
}

/// Time fetching one screen (24 rows x 12 columns of 110 px) from Core at random rows.
pub fn fetch_bench(path: &str) {
    let mut data = TableData::open(path).expect("open");
    let rows = data.row_count();
    let visible_rows = ((800. - HEADER_HEIGHT) / ROW_HEIGHT).ceil() as u64 + 1;
    let visible_columns = (1280. / COLUMN_WIDTH).ceil() as usize;
    let mut rng = 42u64;
    for start_column in [0usize, 12] {
        let columns: Vec<usize> =
            (start_column..(start_column + visible_columns).min(data.columns.len())).collect();
        let names: Vec<String> = columns
            .iter()
            .map(|&c| data.columns[c].name.clone())
            .collect();
        // Through the 64-row block cache, cold: each jump formats up to two blocks per column.
        let mut blocks = vec![];
        for _ in 0..300 {
            let row = mix(&mut rng) % (rows.saturating_sub(visible_rows) + 1);
            data.clear_cache();
            blocks.push(data.prefetch(&columns, row..row + visible_rows));
        }
        // Exactly the visible rows, one format_column per column.
        let mut exact = vec![];
        for _ in 0..300 {
            let row = mix(&mut rng) % (rows.saturating_sub(visible_rows) + 1);
            let t = Instant::now();
            for &c in &columns {
                let _ = data
                    .view()
                    .format_column(data.columns[c].id, row, visible_rows as u32)
                    .unwrap();
            }
            exact.push(t.elapsed());
        }
        println!(
            "fetch one screen, {} rows x {} columns {:?}:\n  64-row blocks (cold): p50 {} p95 {} max {} ms\n  exact rows, format_column per column: p50 {} p95 {} max {} ms",
            visible_rows,
            columns.len(),
            names,
            ms(q(&blocks, 0.5)),
            ms(q(&blocks, 0.95)),
            ms(q(&blocks, 1.0)),
            ms(q(&exact, 0.5)),
            ms(q(&exact, 0.95)),
            ms(q(&exact, 1.0)),
        );
    }
}

/// Time a uuid sort without a window and check the whole result is ordered.
pub fn sort_check(path: &str, column: &str) {
    let dataset = std::sync::Arc::new(gewu_core::Dataset::open(path).expect("open"));
    let id = dataset
        .info()
        .columns
        .iter()
        .find(|c| c.name == column)
        .expect("column")
        .id;
    let filtered = dataset.unfiltered();
    for round in 0..3 {
        let t = Instant::now();
        let view = filtered
            .sort(&[gewu_core::ColumnSort {
                column: id,
                descending: false,
            }])
            .unwrap();
        let took = t.elapsed();
        let mut previous: Option<String> = None;
        let mut ordered = true;
        let mut start = 0;
        while start < view.row_count() {
            let segment = view.format_column(id, start, 65536).unwrap();
            for r in 0..segment.row_count() {
                let cell = segment.cell(r).map(str::to_owned);
                if let (Some(p), Some(c)) = (&previous, &cell) {
                    ordered &= p <= c;
                }
                if cell.is_some() {
                    previous = cell;
                }
            }
            start += segment.row_count() as u64;
        }
        println!(
            "sort {column} round {round}: {took:?}, {} rows, fully ordered: {ordered}",
            view.row_count()
        );
    }
}
