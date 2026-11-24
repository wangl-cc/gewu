use std::{fmt::Write, fs};

use gpui_kit::{
    Modifiers, MouseButton, MouseDownEvent, Pixels, PlatformInput, Point, ScrollDelta, ScrollEvent,
    TouchPhase, point, px, test::TestWindowExt,
};

use super::{HEADER_HEIGHT, ROW_HEIGHT, ui_fixture::TableWindow};
use crate::selection::{Cell, Selection};

const ROWS: u64 = 256;
const COLUMNS: usize = 16;

fn sample_cell(row: u64, column: usize) -> u64 {
    // Give one column enough digits for a font-size change to affect its width.
    let scale = if column == 2 { 10_000_000 } else { 1 };
    row * column as u64 * scale
}

fn drag(window: &mut TableWindow, from: Point<Pixels>, to: Point<Pixels>) {
    let modifiers = Modifiers::default();
    window
        .ui
        .simulate_mouse_down(from, MouseButton::Left, modifiers);
    window
        .ui
        .simulate_mouse_move(to, MouseButton::Left, modifiers);
    window
        .ui
        .simulate_mouse_up(to, MouseButton::Left, modifiers);
    window.settle();
}

fn wheel(window: &mut TableWindow, dx: f32, dy: f32) {
    window.ui.simulate_event(ScrollEvent {
        position: window.cell(
            window.read(|t| t.scroll.anchor().row),
            window.read(|t| t.layout.columns[0].0),
        ),
        delta: ScrollDelta::Pixels(point(px(dx), px(dy))),
        touch_phase: TouchPhase::Moved,
        ..Default::default()
    });
}

fn open() -> (tempfile::TempDir, TableWindow) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gestures.csv");
    let mut text = (1..=COLUMNS)
        .map(|c| format!("c{c}"))
        .collect::<Vec<_>>()
        .join(",");
    for r in 1..=ROWS {
        text.push('\n');
        for c in 1..=COLUMNS {
            if c > 1 {
                text.push(',');
            }
            write!(text, "{}", sample_cell(r, c)).unwrap();
        }
    }
    fs::write(&path, text).unwrap();
    let window = TableWindow::open(&path);
    (directory, window)
}

#[test]
fn jumped_frames_show_the_ordered_values_and_source_row_numbers_immediately() {
    let (_files, mut w) = open();
    w.click(w.header(0), Modifiers::default());
    w.click(w.header(0), Modifiers::default());
    for row in [129, 5, ROWS - 1, 67, 0, 215] {
        w.app.update(|cx| {
            w.table.update(cx, |t, cx| {
                t.scroll_to_row(row);
                let layout = t.compute_layout(t.layout.bounds);
                let rows = layout.vertical.visible_rows();
                let view = t.data.view().clone();
                let count = (rows.end - rows.start) as u32;
                let expected_rows = view.source_rows(rows.start, count);
                let frame = t.build_frame(layout.into(), cx);
                assert_eq!(frame.gutter.len(), expected_rows.len());
                for ((_, label), source) in frame.gutter.iter().zip(expected_rows) {
                    assert_eq!(
                        label.as_ref(),
                        (source + t.prefs.row_number_base).to_string()
                    );
                }
                for ((column, ..), painted) in frame.layout.columns.iter().zip(frame.columns) {
                    let values = view
                        .format_column(t.data.columns[*column].id, rows.start, count)
                        .unwrap();
                    assert_eq!(painted.cells.len(), count as usize);
                    for (offset, cell) in painted.cells.iter().enumerate() {
                        let value = values.cell(offset as u32);
                        assert_eq!(cell.missing, value.is_none());
                        assert_eq!(cell.text.as_ref(), value.unwrap_or("null"));
                    }
                }
            })
        });
    }
}

#[test]
fn grid_geometry_is_committed_after_drawing_and_used_for_the_next_click() {
    use std::rc::Rc;

    use crate::columns::ColumnWidth;

    let (_files, mut w) = open();
    let previous = w.read(|table| Rc::clone(&table.layout));
    w.ui.update(|window, cx| {
        w.table.update(cx, |table, cx| {
            table.data.columns.set_width(0, ColumnWidth::Unmeasured);
            table.scroll_to_row(128);
            cx.notify();
        });
        window.render_frame(cx);
        {
            let table = w.table.read(cx);
            assert!(Rc::ptr_eq(&previous, &table.layout));
            assert!(table.data.columns.entry(0).width == ColumnWidth::Unmeasured);
        }
        let click = previous.bounds.origin
            + point(
                px(previous.gutter + 10.),
                px(HEADER_HEIGHT + ROW_HEIGHT / 2.),
            );
        // Dispatch within the same app update, before GPUI can flush the deferred commit.
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position: click,
                click_count: 1,
                ..Default::default()
            }),
            cx,
        );
    });
    w.read(|table| {
        assert_eq!(table.layout.vertical.anchor().row, 128);
        assert!(matches!(
            table.data.columns.entry(0).width,
            ColumnWidth::Measured(_)
        ));
        assert_eq!(table.layout.columns[0].2, table.column_width(0));
    });
    assert_eq!(
        w.selection(),
        Some(Selection::single(Cell {
            row: 128,
            column: 0
        }))
    );
}

#[test]
fn find_loads_new_viewport_blocks_without_rescheduling_within_a_block() {
    use std::{cell::Cell as Counter, rc::Rc};

    use super::layout::FindViewportChanged;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("matches.csv");
    fs::write(&path, format!("label\n{}", "needle\n".repeat(160))).unwrap();
    let mut w = TableWindow::open(&path);
    w.keys("cmd-f");
    w.type_in_find("needle");
    let column = w.read(|t| t.data.columns[0].id);
    let matched = |w: &TableWindow, row| {
        w.app
            .read(|cx| w.table.read(cx).find.read(cx).matched(column, row))
    };
    assert!(matched(&w, 0));
    assert!(!matched(&w, 128));
    let changes = Rc::new(Counter::new(0));
    let _subscription = w.app.update(|cx| {
        let changes = Rc::clone(&changes);
        cx.subscribe(&w.table, move |_, _: &FindViewportChanged, _| {
            changes.set(changes.get() + 1)
        })
    });
    for (row, expected_changes) in [(1, 0), (128, 1), (130, 1)] {
        w.app.update(|cx| {
            w.table.update(cx, |t, cx| {
                t.scroll_to_row(row);
                cx.notify();
            })
        });
        w.settle();
        assert_eq!(changes.get(), expected_changes);
        assert!(matched(&w, row));
    }
}

#[test]
fn select_copy_and_reveal_through_the_window() {
    let (_files, mut w) = open();
    w.click(w.cell(2, 0), Modifiers::default());
    assert_eq!(
        w.selection(),
        Some(Selection::single(Cell { row: 2, column: 0 }))
    );
    w.click(w.cell(5, 2), Modifiers {
        shift: true,
        ..Default::default()
    });
    assert_eq!(
        w.selection().map(|s| (s.rows(), s.columns())),
        Some((2..6, 0..3))
    );
    let from = w.cell(1, 0);
    let to = w.cell(4, 1);
    drag(&mut w, from, to);
    assert_eq!(
        w.selection().map(|s| (s.rows(), s.columns())),
        Some((1..5, 0..2))
    );
    w.keys("shift-down shift-down shift-right");
    assert_eq!(
        w.selection().map(|s| (s.rows(), s.columns())),
        Some((1..7, 0..3))
    );

    w.keys("cmd-c");
    let expected = (2..=7)
        .map(|r| {
            (1..=COLUMNS)
                .map(|c| sample_cell(r, c).to_string())
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n";
    assert_eq!(w.clipboard().unwrap(), expected);
    w.keys("down");
    assert_eq!(
        w.selection(),
        Some(Selection::single(Cell { row: 7, column: 2 }))
    );
    w.keys("pagedown");
    w.read(|t| {
        let row = t.selection.unwrap().head.row;
        assert_eq!(row, 7 + t.scroll.page_rows());
        assert!(t.scroll.visible_rows().contains(&row));
    });
    w.keys("end");
    assert_eq!(w.selection().unwrap().head.row, ROWS - 1);
    assert_eq!(w.read(|t| t.scroll.fraction()), 1.);
    w.keys("shift-home");
    assert_eq!(w.selection().unwrap().rows(), 0..ROWS);
    assert_eq!(w.read(|t| t.scroll.fraction()), 0.);
    w.keys("cmd-a");
    assert_eq!(
        w.selection().map(|s| (s.rows(), s.columns())),
        Some((0..ROWS, 0..COLUMNS))
    );
}

#[test]
fn resize_scroll_and_sort_through_the_window() {
    let (_files, mut w) = open();
    let normal = Modifiers::default();
    let initial = w.read(|t| t.column_width(0));
    let divider = w.read(|t| {
        t.layout.bounds.origin
            + point(
                px(t.column_left(0).unwrap() + initial),
                px(HEADER_HEIGHT / 2.),
            )
    });
    let end = divider + point(px(60.), px(0.));
    drag(&mut w, divider, end);
    assert_eq!(w.read(|t| t.column_width(0)), initial + 60.);
    let automatic = w.read(|t| t.column_width(1));
    w.app
        .update(|cx| crate::preferences::Preferences::update(cx, |p| p.font_size = 20.));
    w.settle();
    assert_eq!(w.read(|t| t.column_width(0)), initial + 60.);
    assert!(w.read(|t| t.column_width(1)) > automatic);
    w.app
        .update(|cx| crate::preferences::Preferences::update(cx, |p| p.font_size = 12.));
    w.settle();
    w.ui.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: end,
        modifiers: normal,
        click_count: 2,
        first_mouse: false,
    });
    w.ui.simulate_mouse_up(end, MouseButton::Left, normal);
    w.settle();
    assert!(w.read(|t| t.column_width(0)) < initial + 60.);
    for _ in 0..40 {
        wheel(&mut w, 0., -0.5);
    }
    w.settle();
    let offset = w.read(|t| t.scroll.position()) * f64::from(ROW_HEIGHT);
    assert!((offset - 20.).abs() < 1e-6);

    let (thumb, bottom, middle) = w.read(|t| {
        let thumb = t.layout.bounds.origin + t.thumb_bounds().center();
        let bottom = t.layout.bounds.origin.y + px(t.layout.body_bottom());
        (
            thumb,
            point(thumb.x, bottom),
            point(
                thumb.x,
                (bottom + t.layout.bounds.origin.y + px(HEADER_HEIGHT)) / 2.,
            ),
        )
    });
    drag(&mut w, thumb, bottom);
    assert_eq!(w.read(|t| t.scroll.fraction()), 1.);
    for _ in 0..10 {
        wheel(&mut w, 0., 0.5);
    }
    w.settle();
    let delta =
        w.read(|t| (t.scroll.position() - t.scroll.max_position_rows()) * f64::from(ROW_HEIGHT));
    assert!((delta + 5.).abs() < 1e-6);
    let thumb = w.read(|t| t.layout.bounds.origin + t.thumb_bounds().center());
    drag(&mut w, thumb, middle);
    assert!((w.read(|t| t.scroll.fraction()) - 0.5).abs() < 0.01);

    assert!(w.read(|t| t.column_left(COLUMNS - 1)).is_none());
    wheel(&mut w, -10_000., 0.);
    w.settle();
    let header = w.header(COLUMNS - 1);
    w.click(header, normal);
    assert_eq!(
        w.read(|t| t.data.query().sorts[0].column),
        w.read(|t| t.data.columns[COLUMNS - 1].id)
    );
    w.click(header, normal);
    assert_eq!(
        w.read(|t| t.data.view().source_row_at(gewu_core::ViewRow(0))),
        Some(gewu_core::SourceRow(ROWS - 1))
    );
}
