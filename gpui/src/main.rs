//! gewu-gpui: a gpui table over gewu-core, called directly from Rust.
//!
//!     gewu-gpui <file> [--bench SECONDS] [--sort-bench COLUMN]
//!     gewu-gpui <file> --fetch-bench
//!     gewu-gpui <file> --selftest
//!
//! `--bench` drives scrolling once per frame in the three modes of the earlier
//! benchmarks — (a) 1 row, (b) 37 rows, (c) random jump; 1 s warmup each —
//! and prints frame intervals, render-to-paint time, time spent in Core and
//! cells without text. `--sort-bench` then sorts by COLUMN while scrolling
//! one row per frame, and reports the sort time and the frames meanwhile.

mod bench;
mod data;
mod find;
mod interact;
mod scroll;
mod selection;
mod table;

use std::time::Instant;

use gpui_kit::*;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first().cloned() else {
        eprintln!("Usage: gewu-gpui FILE [--bench SECONDS | --workflow-test]");
        std::process::exit(2);
    };
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    if let Some(column) = flag("--sort-check") {
        bench::sort_check(&path, &column);
        return;
    }
    if args.iter().any(|a| a == "--fetch-bench") {
        bench::fetch_bench(&path);
        return;
    }
    let bench_secs: Option<f64> = flag("--bench").and_then(|s| s.parse().ok());
    let sort_column = flag("--sort-bench");
    let workflow = args.iter().any(|a| a == "--workflow-test");
    let selftest = args.iter().any(|a| a == "--selftest");
    // Window content size in logical points, e.g. `--window 1920x1020`.
    let (win_w, win_h) = flag("--window")
        .and_then(|s| {
            s.split_once('x')
                .map(|(w, h)| (w.parse().ok(), h.parse().ok()))
        })
        .and_then(|(w, h)| Some((w?, h?)))
        .unwrap_or((1280f32, 800f32));

    let t = Instant::now();
    let data = data::TableData::open(&path).unwrap_or_else(|error| {
        eprintln!("Could not open {path}: {error}");
        std::process::exit(1);
    });
    println!(
        "opened {path}: {} rows x {} columns in {:?}",
        data.row_count(),
        data.columns.len(),
        t.elapsed()
    );

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            #[cfg(not(feature = "gpui-fast"))]
            window_bounds: Some(WindowBounds::centered(size(px(win_w), px(win_h)), cx)),
            #[cfg(feature = "gpui-fast")]
            window_bounds: Some(WindowBounds::centered(None, size(px(win_w), px(win_h)), cx)),
            // Launched from a shell the window may not become key; measure unthrottled.
            inactive_frame_interval: None,
            focus: true,
            ..Default::default()
        };
        cx.activate(true);
        gpui_kit::open_window(options, cx, move |window, cx| {
            window.set_window_title(
                "Gewu  ·  ⌘F Find  ·  ⇧⌘L Filter selection  ·  ⌥⌘L Clear filters",
            );
            let table = cx.new(|cx| table::TableView::new(data, cx));
            let focus = table.read(cx).focus.clone();
            window.focus(&focus, cx);
            if workflow {
                interact::start_workflow(table.clone(), window);
            } else if selftest {
                interact::start(table.clone(), window);
            }
            if let Some(secs) = bench_secs {
                bench::start(table.clone(), secs, sort_column.clone(), window, cx);
            }
            table
        })
        .expect("open window");
    });
}
