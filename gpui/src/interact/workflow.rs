//! Live-window acceptance test over tests/workflow.csv.
use std::time::{Duration, Instant};

use gpui_kit::*;

use super::{at, down, key, up};
use crate::{
    selection::{Cell, Selection},
    table::{HEADER_HEIGHT, ROW_HEIGHT, TableView},
};

#[derive(Clone, Copy, Debug)]
enum Step {
    Select,
    Filter,
    Sort,
    OpenFind,
    TypeQuery,
    Submit,
    Next,
    Close,
    CopyAndClear,
    CheckCopy,
    TypeMiss,
    SubmitMiss,
    CheckMiss,
}

pub fn start(table: Entity<TableView>, window: &mut Window) {
    schedule(Step::Select, table, Instant::now(), window);
}

fn schedule(step: Step, table: Entity<TableView>, since: Instant, window: &mut Window) {
    window.on_next_frame(move |window, cx| {
        table.update(cx, |_, cx| cx.notify());
        if since.elapsed() > Duration::from_secs(10) {
            let view = table.read(cx);
            eprintln!(
                "rows={}, selection={:?}, error={:?}, find={:?}, open={}, text={:?}, input_focused={}, focused={}, active={}",
                view.data.row_count(),
                view.selection,
                view.query_error,
                view.find.message,
                view.find.open, view.find.text(cx), view.find.input_focused(window,cx),
                view.focus.is_focused(window),
                window.is_window_active()
            );
            fail(step, "timed out waiting for the UI");
        }
        let next = advance(step, &table, window, cx);
        // Repeat the step until its observable state is ready.
        let since = if next.is_some() {
            Instant::now()
        } else {
            since
        };
        schedule(next.unwrap_or(step), table, since, window);
    });
}

fn fail(step: Step, message: &str) -> ! {
    eprintln!("workflow-test failed at {step:?}: {message}");
    std::process::exit(1)
}

fn type_query(window: &mut Window, cx: &mut App, text: &str) {
    // Paint the focused input before dispatching IME text, as on the platform.
    window.refresh();
    window.draw(cx).clear(cx);
    key(window, cx, "cmd-a");
    for c in text.chars() {
        let text = c.to_string();
        let stroke = Keystroke {
            key: text.clone(),
            key_char: Some(text),
            ..Default::default()
        };
        window.dispatch_keystroke(stroke.clone(), cx);
        window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke: stroke }), cx);
        window.refresh();
        window.draw(cx).clear(cx);
    }
}

// Dispatch Return through the native key binding; simulated IME also inserts a newline.
fn submit(window: &mut Window, cx: &mut App) {
    let stroke = Keystroke::parse("enter").unwrap();
    window.dispatch_event(
        PlatformInput::KeyDown(KeyDownEvent {
            keystroke: stroke.clone(),
            is_held: false,
            prefer_character_input: false,
        }),
        cx,
    );
    window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke: stroke }), cx);
}

fn advance(
    step: Step,
    table: &Entity<TableView>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Step> {
    use Step::*;
    let current = table.read(cx);
    let selected = |row, column| Some(Selection::single(Cell { row, column }));
    match step {
        Select => {
            if !window.is_window_active() {
                return None;
            }
            let x = current.column_left(0)? + 10.;
            if current.data.row_count() != 5 {
                fail(step, "use tests/workflow.csv");
            }
            // A click below the five visible rows must not select the first row.
            let blank = at(x, HEADER_HEIGHT + 6. * ROW_HEIGHT);
            down(window, cx, blank, false, 1);
            up(window, cx, blank);
            if table.read(cx).selection.is_some() {
                fail(step, "clicking below the table selected a cell");
            }
            let p = at(x, HEADER_HEIGHT + ROW_HEIGHT / 2.);
            down(window, cx, p, false, 1);
            up(window, cx, p);
            if table.read(cx).selection != selected(0, 0) {
                return None;
            }
            Some(Filter)
        }
        Filter => {
            key(window, cx, "cmd-shift-l");
            Some(Sort)
        }
        Sort => {
            if current.sorting.is_some() || current.data.row_count() != 3 {
                return None;
            }
            let p = at(current.column_left(1)? + 10., HEADER_HEIGHT / 2.);
            // The second click precedes publication of the first sort.
            for _ in 0..2 {
                down(window, cx, p, false, 1);
                up(window, cx, p);
            }
            Some(OpenFind)
        }
        OpenFind => {
            if current.sorting.is_some() {
                return None;
            }
            if (0..3).map(|row| current.data.source_row(row)).ne([4, 2, 0]) {
                fail(
                    step,
                    "expected only the three keep rows, in descending score order",
                );
            }
            key(window, cx, "cmd-f");
            Some(TypeQuery)
        }
        TypeQuery => {
            if !current.find.input_focused(window, cx) {
                return None;
            }
            type_query(window, cx, "marker");
            Some(Submit)
        }
        Submit => {
            if &*current.find.text(cx) != "marker" {
                fail(step, "query was not typed into Find");
            }
            submit(window, cx);
            Some(Next)
        }
        Next => {
            let note = current.data.columns[2].id;
            if current.find.pending
                || current.selection != selected(0, 2)
                || !current.find.matched(note, 0)
                || !current.find.matched(note, 1)
            {
                return None;
            }
            if current.find.matched(note, 2) {
                fail(step, "nonmatching cell highlighted");
            }
            key(window, cx, "cmd-g");
            Some(Close)
        }
        Close => {
            if current.find.pending || current.selection != selected(1, 2) {
                return None;
            }
            key(window, cx, "escape");
            Some(CopyAndClear)
        }
        CopyAndClear => {
            key(window, cx, "shift-left");
            key(window, cx, "shift-left");
            key(window, cx, "cmd-c");
            key(window, cx, "cmd-alt-l");
            Some(CheckCopy)
        }
        CheckCopy => {
            if current.last_copy.is_none()
                || current.sorting.is_some()
                || current.data.row_count() != 5
            {
                return None;
            }
            let pasted = cx.read_from_clipboard().and_then(|item| item.text());
            if pasted.as_deref().map(str::trim_end) != Some("keep\t5\tmarker-low") {
                fail(
                    step,
                    "copied TSV does not match the selected filtered, sorted row",
                );
            }
            key(window, cx, "cmd-f");
            Some(TypeMiss)
        }
        TypeMiss => {
            if !current.find.input_focused(window, cx) {
                return None;
            }
            type_query(window, cx, "absent");
            Some(SubmitMiss)
        }
        SubmitMiss => {
            if &*current.find.text(cx) != "absent" {
                fail(step, "query was not replaced in Find");
            }
            submit(window, cx);
            Some(CheckMiss)
        }
        CheckMiss => {
            if current.find.pending || current.find.message != "No matches" {
                return None;
            }
            if current.data.row_count() != 5 {
                fail(step, "Find changed the displayed rows");
            }
            println!(
                "PASS filter, rapid sort, Find navigation, copy displayed snapshot, clear filters, and no matches"
            );
            println!("workflow-test: passed");
            std::process::exit(0)
        }
    }
}
