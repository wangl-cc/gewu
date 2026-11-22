//! `--selftest`: drive the real window with synthesized platform input
//! (`Window::dispatch_event`), one step per frame, and check the results:
//! selection by click / shift-click / drag / keys, auto-reveal, column
//! resizing, sub-pixel scrolling, the scrollbar, background sorting and Cmd+C
//! (verified through the system pasteboard with `pbpaste`).

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui_kit::*;

use crate::{
    data::copy_tsv,
    selection::{Cell, Selection},
    table::{HEADER_HEIGHT, ROW_HEIGHT, TableView},
};

struct State {
    table: Entity<TableView>,
    step: usize,
    wait_until: Option<Instant>,
    failures: usize,
    sort_frames: Vec<Instant>,
    mark: f64,
}

pub fn start(table: Entity<TableView>, window: &mut Window) {
    let state = Rc::new(RefCell::new(State {
        table,
        step: 0,
        wait_until: Some(Instant::now() + Duration::from_millis(800)),
        failures: 0,
        sort_frames: vec![],
        mark: 0.0,
    }));
    schedule(state, window);
}

fn schedule(state: Rc<RefCell<State>>, window: &mut Window) {
    window.on_next_frame(move |window, cx| {
        if run(&state, window, cx) {
            schedule(state, window);
        }
    });
}

fn check(s: &mut State, ok: bool, what: &str) {
    println!("{} {what}", if ok { "PASS" } else { "FAIL" });
    if !ok {
        s.failures += 1;
    }
}

fn at(x: f32, y: f32) -> Point<Pixels> {
    point(px(x), px(y))
}

fn down(window: &mut Window, cx: &mut App, p: Point<Pixels>, shift: bool, clicks: usize) {
    let modifiers = Modifiers {
        shift,
        ..Default::default()
    };
    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position: p,
            modifiers,
            click_count: clicks,
            first_mouse: false,
        }),
        cx,
    );
}

fn up(window: &mut Window, cx: &mut App, p: Point<Pixels>) {
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position: p,
            modifiers: Modifiers::default(),
            click_count: 1,
        }),
        cx,
    );
}

fn drag_to(window: &mut Window, cx: &mut App, p: Point<Pixels>) {
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position: p,
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::default(),
        }),
        cx,
    );
}

fn key(window: &mut Window, cx: &mut App, k: &'static str) {
    window.dispatch_keystroke(Keystroke::parse(k).unwrap(), cx);
}

fn wheel(window: &mut Window, cx: &mut App, dx: f32, dy: f32) {
    window.dispatch_event(
        PlatformInput::Scroll(ScrollEvent {
            position: at(600., 400.),
            delta: ScrollDelta::Pixels(point(px(dx), px(dy))),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
            ..Default::default()
        }),
        cx,
    );
}

fn run(state: &Rc<RefCell<State>>, window: &mut Window, cx: &mut App) -> bool {
    let mut s = state.borrow_mut();
    let table = s.table.clone();
    table.update(cx, |_, cx| cx.notify());
    if let Some(t) = s.wait_until {
        if Instant::now() < t {
            return true;
        }
        s.wait_until = None;
    }
    let (gutter, rows) = {
        let t = table.read(cx);
        (t.gutter_width(), t.data.row_count())
    };
    let cell_center = |row_on_screen: u64, col_x: f32| {
        at(
            gutter + col_x,
            HEADER_HEIGHT + (row_on_screen as f32 + 0.5) * ROW_HEIGHT,
        )
    };
    let sel = |cx: &App| table.read(cx).selection;
    let anchor = |cx: &App| table.read(cx).scroll.position();
    match s.step {
        0 => {
            println!("selftest on {rows} rows, gutter {gutter}px");
            down(window, cx, cell_center(2, 20.), false, 1);
            up(window, cx, cell_center(2, 20.));
        }
        1 => {
            let ok = sel(cx) == Some(Selection::single(Cell { row: 2, column: 0 }));
            check(&mut s, ok, "click selects one cell (row 2, col 0)");
            down(window, cx, cell_center(5, 2. * 110. + 20.), true, 1);
            up(window, cx, cell_center(5, 2. * 110. + 20.));
        }
        2 => {
            let ok = sel(cx).map(|s| (s.rows(), s.columns())) == Some((2..6, 0..3));
            check(
                &mut s,
                ok,
                "shift-click extends to a rectangle rows 2..6 x cols 0..3",
            );
            down(window, cx, cell_center(1, 20.), false, 1);
            drag_to(window, cx, cell_center(4, 110. + 20.));
            up(window, cx, cell_center(4, 110. + 20.));
        }
        3 => {
            let ok = sel(cx).map(|s| (s.rows(), s.columns())) == Some((1..5, 0..2));
            check(&mut s, ok, "drag selects rows 1..5 x cols 0..2");
            key(window, cx, "shift-down");
            key(window, cx, "shift-down");
            key(window, cx, "shift-right");
        }
        4 => {
            let ok = sel(cx).map(|s| (s.rows(), s.columns())) == Some((1..7, 0..3));
            check(&mut s, ok, "shift+arrows extend to rows 1..7 x cols 0..3");
            key(window, cx, "cmd-c");
            s.wait_until = Some(Instant::now() + Duration::from_millis(400));
        }
        5 => {
            let t = table.read(cx);
            let expected = copy_tsv(t.data.view(), 1..7, &t.data.column_ids(0..3), false).unwrap();
            let pasted = std::process::Command::new("pbpaste")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            println!(
                "  pbpaste ({} bytes):\n{}",
                pasted.len(),
                pasted
                    .lines()
                    .map(|l| format!("    {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            let ok = pasted == expected
                && pasted.lines().count() == 6
                && pasted.lines().all(|l| l.split('\t').count() == 3);
            check(
                &mut s,
                ok,
                "Cmd+C puts the 6x3 selection on the pasteboard as Core's TSV export",
            );
            key(window, cx, "down");
        }
        6 => {
            let ok = sel(cx) == Some(Selection::single(Cell { row: 7, column: 2 }));
            check(
                &mut s,
                ok,
                "arrow collapses and moves the head (row 7, col 2)",
            );
            for _ in 0..3 {
                key(window, cx, "pagedown");
            }
        }
        7 => {
            let t = table.read(cx);
            let page = t.scroll.page_rows();
            let head = t.selection.unwrap().head.row;
            let visible = t.scroll.visible_rows();
            let ok = head == 7 + 3 * page
                && visible.contains(&head)
                && t.scroll.row_top(head) + ROW_HEIGHT as f64 <= t.scroll.viewport();
            check(
                &mut s,
                ok,
                &format!(
                    "3x Page Down moves {page} rows each and reveals the head (row {head}, top {})",
                    t.scroll.anchor().row
                ),
            );
            key(window, cx, "end");
        }
        8 => {
            let t = table.read(cx);
            let ok = t.selection.unwrap().head.row == rows - 1 && t.scroll.fraction() == 1.0;
            check(
                &mut s,
                ok,
                "End jumps to the last row and scrolls to the bottom",
            );
            key(window, cx, "shift-home");
        }
        9 => {
            let t = table.read(cx);
            let ok = t.selection.map(|s| s.rows()) == Some(0..rows) && t.scroll.position() == 0.0;
            check(
                &mut s,
                ok,
                "Shift+Home extends to the first row and scrolls to the top",
            );
            key(window, cx, "cmd-a");
        }
        10 => {
            let ok = sel(cx) == Some(Selection::all(rows, 28));
            check(&mut s, ok, "Cmd+A selects all cells");
            // Resize column 0 by dragging its header divider 60 px right.
            let x = gutter + 110.;
            down(window, cx, at(x, 16.), false, 1);
            drag_to(window, cx, at(x + 60., 16.));
            up(window, cx, at(x + 60., 16.));
        }
        11 => {
            let w = table.read(cx).column_width(0);
            check(
                &mut s,
                w == 170.,
                &format!("divider drag resizes column 0 to {w}px"),
            );
            let x = gutter + 170.;
            down(window, cx, at(x, 16.), false, 2);
            up(window, cx, at(x, 16.));
        }
        12 => {
            let w = table.read(cx).column_width(0);
            check(
                &mut s,
                (40.0..170.0).contains(&w),
                &format!("double-click divider fits column 0 to {w}px"),
            );
            // Sub-pixel trackpad deltas in the middle of the table.
            table.update(cx, |t, _| t.scroll.set_fraction(0.5));
            s.mark = anchor(cx);
            for _ in 0..40 {
                wheel(window, cx, 0., -0.5);
            }
        }
        13 => {
            let moved = (anchor(cx) - s.mark) * ROW_HEIGHT as f64;
            check(
                &mut s,
                (moved - 20.0).abs() < 1e-6,
                &format!("40 x 0.5 px wheel deltas at the middle move {moved} px"),
            );
            table.update(cx, |t, _| t.scroll.set_fraction(1.0));
            for _ in 0..10 {
                wheel(window, cx, 0., 0.5);
            }
            s.mark = anchor(cx);
        }
        14 => {
            let moved = (s.mark - table.read(cx).scroll.max_position_rows()) * ROW_HEIGHT as f64;
            check(
                &mut s,
                (moved + 5.0).abs() < 1e-6,
                &format!("10 x 0.5 px up from the bottom move {moved} px"),
            );
            // Drag the scrollbar thumb to the bottom, then halfway.
            let thumb = table.read(cx).thumb_bounds();
            let start = thumb.center();
            down(window, cx, start, false, 1);
            drag_to(window, cx, at(start.x.as_f32(), 790.));
            up(window, cx, at(start.x.as_f32(), 790.));
        }
        15 => {
            check(
                &mut s,
                table.read(cx).scroll.fraction() == 1.0,
                "dragging the thumb to the end reaches the last row",
            );
            let thumb = table.read(cx).thumb_bounds();
            let track_mid = HEADER_HEIGHT + (800. - HEADER_HEIGHT) / 2.;
            down(window, cx, thumb.center(), false, 1);
            drag_to(window, cx, at(thumb.center().x.as_f32(), track_mid));
            up(window, cx, at(thumb.center().x.as_f32(), track_mid));
        }
        16 => {
            let f = table.read(cx).scroll.fraction();
            check(
                &mut s,
                (f - 0.5).abs() < 0.01,
                &format!("thumb dragged to mid-track maps to row fraction {f:.4}"),
            );
            // Bring the uuid column into view with a horizontal scroll, then click its header.
            table.update(cx, |t, _| t.scroll.set_fraction(0.0));
            let uuid = table
                .read(cx)
                .data
                .columns
                .iter()
                .position(|c| c.name == "uuid")
                .unwrap();
            let left: f32 = (0..uuid).map(|c| table.read(cx).column_width(c)).sum();
            wheel(window, cx, -left, 0.);
        }
        17 => {
            let uuid = table
                .read(cx)
                .data
                .columns
                .iter()
                .position(|c| c.name == "uuid")
                .unwrap();
            let x = table.read(cx).column_left(uuid).expect("uuid visible") + 20.;
            down(window, cx, at(x, 16.), false, 1);
            up(window, cx, at(x, 16.));
            s.sort_frames.clear();
        }
        18 => {
            s.sort_frames.push(Instant::now());
            if table.read(cx).sorting.is_some() {
                return true; // keep stepping while the sort runs
            }
            let uuid = table
                .read(cx)
                .data
                .columns
                .iter()
                .position(|c| c.name == "uuid")
                .unwrap();
            // The new view dropped the cache; fetch the rows checked here (the next render would
            // too).
            table.update(cx, |t, _| t.data.prefetch(&[uuid], 0..5));
            let t = table.read(cx);
            let first: Vec<String> = (0..5)
                .map(|r| {
                    t.data
                        .cell(uuid, r)
                        .flatten()
                        .map(|s| s.to_string())
                        .unwrap_or_default()
                })
                .collect();
            let sorted = !first[0].is_empty() && first.windows(2).all(|w| w[0] <= w[1]);
            let gaps: Vec<f64> = s
                .sort_frames
                .windows(2)
                .map(|w| (w[1] - w[0]).as_secs_f64() * 1e3)
                .collect();
            let max_gap = gaps.iter().cloned().fold(0.0, f64::max);
            println!(
                "  sort took {:?}; {} frames meanwhile, max frame gap {max_gap:.1} ms; first uuids {:?}; source rows {:?}",
                t.last_sort,
                gaps.len(),
                &first[..3],
                (0..3).map(|r| t.data.source_row(r) + 1).collect::<Vec<_>>()
            );
            check(
                &mut s,
                sorted && t.data.query().sorts.len() == 1,
                "header click sorts by uuid ascending in the background",
            );
            check(
                &mut s,
                max_gap < 50.0,
                "UI kept painting during the sort (max frame gap < 50 ms)",
            );
        }
        _ => {
            println!("selftest: {} failure(s)", s.failures);
            cx.quit();
            return false;
        }
    }
    s.step += 1;
    true
}

mod workflow;
pub use workflow::start as start_workflow;
