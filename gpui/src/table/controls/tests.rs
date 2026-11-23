use std::{path::Path, sync::Arc};

use gpui_kit::{Action, ElementId, Modifiers, test::TestWindowExt};

use crate::{
    actions::{EditFilter, EditSort, ExportTable, ManageColumns},
    table::ui_fixture::TableWindow,
};

fn window() -> TableWindow {
    TableWindow::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/workflow.csv"))
}

fn action(w: &mut TableWindow, action: impl Action) {
    w.ui.update(|window, cx| window.dispatch_action(Box::new(action), cx));
    w.settle();
}

fn click(w: &mut TableWindow, id: impl Into<ElementId>) {
    let id = id.into();
    w.ui.update(|window, cx| {
        let target = window.find(id.clone());
        assert!(
            target.visible(),
            "{id:?} bounds {:?} in {:?} window",
            target.bounds(),
            window.viewport_size()
        );
        window.click(id, cx)
    });
    w.settle();
}

#[test]
fn clicking_a_scatter_point_reveals_its_source_row_in_the_filtered_sort_order() {
    use std::fmt::Write as _;

    use gpui_kit::{point, px, size};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("measurements.csv");
    let mut csv = String::from("group,score\n");
    for row in 0..120 {
        let group = if row % 2 == 0 { "even" } else { "odd" };
        let score = if row == 24 { 10_000 } else { row };
        writeln!(csv, "{group},{score}").unwrap();
    }
    std::fs::write(&path, csv).unwrap();
    let mut w = TableWindow::open(&path);
    w.ui.simulate_resize(size(px(800.), px(720.)));
    w.settle();
    w.click(w.cell(0, 0), Modifiers::default());
    w.keys("cmd-alt-shift-l");
    w.click(w.header(1), Modifiers::default());
    assert_eq!(w.source_rows().len(), 60);
    assert_eq!(w.source_rows()[59], 24);
    action(&mut w, crate::actions::ShowChartInspector);
    let plot = w.ui.update(|win, _| win.find("scatter-plot").bounds());
    let x = plot.left() + px(50.) + (plot.size.width - px(58.)) * (24. / 118.);
    let hit = point(x, plot.top() + px(9.));
    let miss = plot.center();
    let before = w.selection();
    w.click(miss, Modifiers::default());
    assert_eq!(w.selection(), before);
    w.ui.update(|win, cx| win.drag(miss, hit, cx));
    w.settle();
    assert_eq!(w.selection(), before);
    w.ui.update(|win, cx| win.drag(hit, miss, cx));
    w.settle();
    assert_eq!(w.selection(), before);
    w.click(hit, Modifiers::default());
    let head = w.selection().unwrap().head;
    assert_eq!((head.row, head.column), (59, 1));
    assert!(w.read(|table| table.layout.vertical.visible_rows().contains(&59)));
    w.keys("up");
    assert_eq!(w.selection().unwrap().head.row, 58);
}

#[test]
fn inspector_drag_clamps_width_and_keeps_the_table_usable_in_a_narrow_window() {
    use gpui_kit::{point, px, size};

    let mut w = window();
    w.ui.simulate_resize(size(px(900.), px(600.)));
    action(&mut w, crate::actions::ShowChartInspector);
    let width = |w: &mut TableWindow| {
        w.ui.update(|win, _| win.find("inspector").bounds().size.width)
    };
    assert_eq!(width(&mut w), px(300.));
    for (target, expected) in [(100., 420.), (850., 240.)] {
        w.ui.update(|win, cx| {
            let handle = win.find("inspector-resize").bounds().center();
            win.drag(handle, point(px(target), handle.y), cx);
        });
        w.settle();
        assert_eq!(width(&mut w), px(expected));
    }
    w.ui.simulate_resize(size(px(300.), px(600.)));
    w.settle();
    assert!(width(&mut w) <= px(180.));
    assert!(w.read(|table| table.layout.bounds.size.width >= px(120.)));
}

#[test]
fn cell_context_menu_copies_the_clicked_value_with_embedded_line_breaks() {
    let files = tempfile::tempdir().unwrap();
    let path = files.path().join("notes.csv");
    let text = format!("{}\nsecond line, end", "資料, one row\n".repeat(90));
    std::fs::write(&path, format!("key,note\nother,\"{text}\"\n")).unwrap();
    let mut w = TableWindow::open(&path);
    w.click(w.cell(0, 0), Modifiers::default());
    let position = w.cell(0, 1);

    let open_menu = |w: &mut TableWindow| {
        w.ui.simulate_event(gpui_kit::MouseDownEvent {
            button: gpui_kit::MouseButton::Right,
            position,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        });
        w.settle();
    };
    open_menu(&mut w);
    click(&mut w, "Copy Cell");
    assert_eq!(w.clipboard().as_deref(), Some(text.as_str()));

    open_menu(&mut w);
    click(&mut w, "View Cell…");
    assert!(w.read(|table| matches!(
        &table.controls.drawer,
        Some(super::Drawer::Cell { name, value: Some(value), .. })
            if name == "note" && value.as_ref() == text
    )));
    w.ui.update(|window, cx| {
        let copy = window.find("copy-cell");
        assert!(copy.visible() && copy.bounds().center().y < window.viewport_size().height);
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("previous".into()));
    });
    click(&mut w, "copy-cell");
    assert_eq!(w.clipboard().as_deref(), Some(text.as_str()));
}

#[test]
fn confirmed_hide_clears_conditions_and_reorders_without_replacing_rows() {
    let mut w = window();
    let ids = w.read(|t| t.data.column_ids(0..3));
    let group = ids[0].into_raw() as usize;
    let score = ids[1].into_raw() as usize;
    w.click(w.cell(0, 0), Modifiers::default());
    w.keys("cmd-alt-shift-l");
    assert_eq!(w.source_rows(), [0, 2, 4]);
    action(&mut w, ManageColumns);
    click(&mut w, ("toggle-column", group));
    click(&mut w, "cancel-hide");
    assert_eq!(w.source_rows(), [0, 2, 4]);
    click(&mut w, ("toggle-column", group));
    click(&mut w, "confirm-hide");
    assert_eq!(w.source_rows(), [0, 1, 2, 3, 4]);
    let view = w.read(|t| t.data.view().clone());
    click(&mut w, ("column-later", score));
    click(&mut w, "show-all-columns");
    assert!(w.read(|t| Arc::ptr_eq(&view, t.data.view())));
    assert_eq!(w.read(|t| t.data.column_ids(0..3)), [
        ids[2], ids[1], ids[0]
    ]);
    click(&mut w, "close-table-drawer");
    w.click(w.cell(0, 0), Modifiers::default());
    w.keys("cmd-c");
    assert_eq!(
        w.clipboard().unwrap().lines().next().unwrap(),
        "plain\t2\tkeep"
    );
}

#[test]
fn filter_popover_centers_on_its_column_and_stays_inside_the_window() {
    use gpui_kit::{ScrollDelta, ScrollEvent, TouchPhase, point, px, size};

    use crate::table::HEADER_HEIGHT;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("columns.csv");
    std::fs::write(&path, "a,b,c,d,e,f,g,h\n1,2,3,4,5,6,7,8\n").unwrap();
    let mut w = TableWindow::open(&path);
    w.ui.simulate_resize(size(px(640.), px(720.)));
    w.settle();

    for col in [3, 0, 6] {
        let (center, right, bottom) = w.read(|t| {
            let left = t.layout.bounds.left() + px(t.column_left(col).unwrap());
            let width = px(t.column_width(col));
            (
                left + width / 2.,
                left + width,
                t.layout.bounds.top() + px(HEADER_HEIGHT),
            )
        });
        w.click(
            point(right - px(11.), bottom - px(16.)),
            Modifiers::default(),
        );
        w.ui.update(|window, _| {
            let panel = window.find("filter-editor").bounds();
            let width = window.viewport_size().width;
            let expected = (center - panel.size.width / 2.).clamp(px(0.), width - panel.size.width);
            assert!(
                (panel.left() - expected).abs() <= px(1.),
                "column {col}: panel left {}, expected {expected}",
                panel.left()
            );
            assert!((panel.top() - bottom).abs() <= px(1.));
        });
        click(&mut w, "cancel-filter");
    }

    // Keep an open editor attached when the grid scrolls under it.
    w.click(w.cell(0, 3), Modifiers::default());
    action(&mut w, EditFilter);
    let before =
        w.ui.update(|window, _| window.find("filter-editor").bounds());
    w.ui.simulate_event(ScrollEvent {
        position: point(px(90.), px(380.)),
        delta: ScrollDelta::Pixels(point(px(-80.), px(0.))),
        touch_phase: TouchPhase::Moved,
        ..Default::default()
    });
    w.settle();
    let header =
        w.read(|t| t.layout.bounds.left() + px(t.column_left(3).unwrap() + t.column_width(3) / 2.));
    let after =
        w.ui.update(|window, _| window.find("filter-editor").bounds());
    assert!(after.left() < before.left());
    assert!((after.center().x - header).abs() <= px(1.));

    w.ui.simulate_resize(size(px(300.), px(720.)));
    w.settle();
    w.ui.update(|window, _| {
        let panel = window.find("filter-editor").bounds();
        assert!(panel.left() >= px(0.) && panel.right() <= window.viewport_size().width);
    });
}

#[test]
fn filter_and_sort_summaries_follow_applied_rows_and_rule_priority() {
    let mut w = window();
    let ids = w.read(|t| t.data.column_ids(0..3));
    action(&mut w, EditFilter);
    click(&mut w, "no-values");
    click(&mut w, ("facet", 0usize));
    click(&mut w, "apply-filter");
    assert!(w.read(|t| t.data.row_count() < 5));
    click(&mut w, "filtered-count");
    click(&mut w, "clear-all-filters");
    assert_eq!(w.source_rows(), [0, 1, 2, 3, 4]);
    action(&mut w, EditSort);
    for id in [ids[0], ids[1]] {
        click(&mut w, ("add-sort", id.into_raw() as usize));
    }
    w.ui.update(|window, cx| window.right_click(("sort-column", ids[1].into_raw() as usize), cx));
    w.settle();
    click(&mut w, "Move Up");
    click(&mut w, ("direction", ids[1].into_raw() as usize));
    click(&mut w, ("sort-column", ids[1].into_raw() as usize));
    click(&mut w, ("replace-sort", ids[2].into_raw() as usize));
    let actual = w.read(|t| {
        t.data
            .query()
            .sorts
            .iter()
            .map(|s| (s.column, s.descending))
            .collect::<Vec<_>>()
    });
    assert_eq!(actual, [(ids[2], true), (ids[0], false)]);
    click(&mut w, "close-table-drawer");
    click(&mut w, "sorted-count");
    click(&mut w, "clear-sorts");
    assert_eq!(w.source_rows(), [0, 1, 2, 3, 4]);
}

#[test]
fn export_uses_the_displayed_snapshot_from_before_the_save_panel() {
    let mut w = window();
    w.click(w.cell(0, 0), Modifiers::default());
    w.keys("cmd-alt-shift-l");
    action(&mut w, ExportTable);
    click(&mut w, "csv");
    click(&mut w, "choose-export-path");
    assert!(w.app.did_prompt_for_new_path());
    w.ui.update(|window, cx| assert!(!window.is_action_available(&ExportTable, cx)));
    w.keys("cmd-alt-l");
    assert_eq!(w.source_rows(), [0, 1, 2, 3, 4]);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("filtered.csv");
    w.app.simulate_new_path_selection(|_| Some(dest.clone()));
    w.settle();
    w.ui.update(|window, cx| assert!(window.is_action_available(&ExportTable, cx)));
    assert_eq!(
        std::fs::read_to_string(dest)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            "group,score,note",
            "keep,2,plain",
            "keep,5,marker-low",
            "keep,9,marker-high"
        ]
    );
}

#[test]
fn export_announces_the_selected_format_and_compression() {
    let mut w = window();
    action(&mut w, ExportTable);
    click(&mut w, "parquet");
    click(&mut w, "Brotli");
    w.ui.update(|window, _| {
        for (id, checked) in [
            ("parquet", true),
            ("csv", false),
            ("Brotli", true),
            ("Zstd", false),
        ] {
            let control = window.find(id);
            assert_eq!(control.role(), Some(gpui_kit::Role::RadioButton));
            assert_eq!(control.checked(), Some(checked));
        }
        let save = window.find("choose-export-path");
        assert_eq!(save.role(), Some(gpui_kit::Role::Button));
        assert_eq!(save.checked(), None);
    });
}
