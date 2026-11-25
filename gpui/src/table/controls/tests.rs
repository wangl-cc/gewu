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
    let data = crate::panels::scatter_data_bounds(plot);
    let x = data.left() + data.size.width * (24. / 118.);
    let hit = point(x, data.top());
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
    w.ui.update(|window, _| {
        assert_eq!(
            window.find("scatter-plot").label(),
            Some("Scatter plot, 60 points, row 25 selected")
        );
    });
    action(&mut w, ManageColumns);
    click(&mut w, ("column-earlier", 1usize));
    click(&mut w, "close-table-drawer");
    assert_eq!(w.selection().unwrap().head.column, 0);
    w.ui.update(|window, _| {
        assert_eq!(
            window.find("scatter-plot").label(),
            Some("Scatter plot, 60 points, row 25 selected")
        );
    });
    w.keys("up");
    assert_eq!(w.selection().unwrap().head.row, 58);
    w.ui.update(|window, _| {
        assert_eq!(
            window.find("scatter-plot").label(),
            Some("Scatter plot, 60 points")
        );
    });
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
    let text = format!(
        "{}\nsecond line, end",
        "資料, one row with long preview text\n".repeat(90)
    );
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
    w.ui.update(|window, _| {
        let bounds = window.find("popup-menu").bounds();
        let mut menu = window.within("popup-menu");
        assert!(
            bounds.top() > position.y,
            "cell menu should open below the clicked line ({bounds:?} vs {position:?})"
        );
        let first = menu.find(0usize);
        assert_eq!(
            menu.within(0usize).find("menu-content").value(),
            Some("Copy Cell")
        );
        assert_eq!(
            menu.within(2usize).find("menu-content").value(),
            Some("View Cell…")
        );
        assert!((gpui_kit::px(24.)..=gpui_kit::px(26.)).contains(&first.bounds().size.height));
    });
    w.keys("down enter");
    assert_eq!(w.clipboard().as_deref(), Some(text.as_str()));

    open_menu(&mut w);
    w.ui.update(|window, cx| window.within("popup-menu").click(2usize, cx));
    w.settle();
    assert!(w.read(|table| matches!(
        &table.controls.drawer,
        Some(super::Drawer::Cell { value: Some(value), .. }) if value.as_ref() == text
    )));
    assert_eq!(w.selection().unwrap().head.column, 1);
    let row_bottom = w.read(|table| {
        table.layout.bounds.top()
            + gpui_kit::px(crate::table::HEADER_HEIGHT + table.layout.vertical.row_top(1) as f32)
    });
    let long = w.ui.update(|window, cx| {
        let preview = window.find("cell-preview").bounds();
        assert!(preview.top() >= row_bottom + gpui_kit::px(3.));
        let copy = window.find("copy-cell");
        assert!(copy.visible() && copy.bounds().center().y < window.viewport_size().height);
        assert!(window.try_find("close-table-drawer").is_none());
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("previous".into()));
        preview.size
    });
    click(&mut w, "copy-cell");
    assert_eq!(w.clipboard().as_deref(), Some(text.as_str()));
    w.keys("escape");
    assert!(w.read(|table| table.controls.drawer.is_none()));

    w.ui.simulate_event(gpui_kit::MouseDownEvent {
        button: gpui_kit::MouseButton::Left,
        position: w.cell(0, 0),
        modifiers: Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    w.settle();
    let short =
        w.ui.update(|window, _| window.find("cell-preview").bounds().size);
    assert!(
        short.width >= gpui_kit::px(160.) && short.width < long.width && short.height < long.height,
        "short {short:?}, long {long:?}"
    );
    w.click(w.header(0), Modifiers::default());
    assert!(w.read(|table| table.controls.drawer.is_none()));
}

fn open_header_menu(w: &mut TableWindow) {
    let point = w.header(0);
    w.ui.simulate_event(gpui_kit::MouseDownEvent {
        button: gpui_kit::MouseButton::Right,
        position: point,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    w.settle();
}

#[test]
fn pressing_anywhere_in_a_menu_row_paints_the_full_row() {
    use std::{cell::Cell, rc::Rc};

    use gpui_kit::{
        MouseButton, MouseMoveEvent, TestAppContext, VisualTestContext, WindowOptions,
        component::menu::PopupMenu, point, px,
    };

    // Mount the production PopupMenu without the document's rounded clipping layer.
    // Fast's painted_quads() exposes the top-level scene only.
    let app = TestAppContext::single();
    let activations = Rc::new(Cell::new(0));
    let (handle, _) = app.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let activations = activations.clone();
            PopupMenu::build(window, cx, |menu, _, _| {
                menu.max_w(px(194.)).item(
                    super::menus::menu_item("Sort Ascending", true, None)
                        .on_click(move |_, _, _| activations.set(activations.get() + 1)),
                )
            })
        })
        .unwrap()
    });
    let mut ui = VisualTestContext::from_window(handle, &app);
    for (i, inset) in [2., 18.].into_iter().enumerate() {
        let bounds = ui.update(|window, cx| {
            window.render_frame(cx);
            window.within("popup-menu").find(0usize).bounds()
        });
        let press = point(bounds.left() + px(inset), bounds.center().y);
        ui.simulate_event(MouseMoveEvent {
            position: press,
            pressed_button: None,
            modifiers: Modifiers::default(),
        });
        ui.run_until_parked();
        ui.simulate_mouse_down(press, MouseButton::Left, Modifiers::default());
        ui.update(|window, cx| {
            window.render_frame(cx);
            let fill = crate::theme::GewuPalette::get(cx).menu_pressed();
            let row = bounds.scale(window.scale_factor());
            assert!(
                window
                    .painted_quads()
                    .iter()
                    .any(|q| q.bounds == row && q.background.as_solid() == Some(fill)),
                "press {inset}pt from the row's left edge"
            );
        });
        ui.simulate_mouse_up(press, MouseButton::Left, Modifiers::default());
        ui.run_until_parked();
        assert_eq!(activations.get(), i + 1);
    }
    app.quit();
}

#[test]
fn unavailable_menu_commands_keep_their_names_and_are_skipped_by_keyboard_navigation() {
    let mut w = window();
    open_header_menu(&mut w);
    let header_bottom =
        w.read(|table| table.layout.bounds.top() + gpui_kit::px(crate::table::HEADER_HEIGHT));
    w.ui.update(|window, _| {
        assert!(window.find("popup-menu").bounds().top() >= header_bottom);
        let binding = gpui_kit::component::kbd::Kbd::format(
            &gpui_kit::Keystroke::parse("cmd-alt-f").unwrap(),
        );
        let hints = gpui_kit::base::test_support::texts(window);
        let filter = window.within("popup-menu").find(5usize).bounds();
        assert!(
            hints
                .iter()
                .any(|hint| hint.text() == binding && filter.contains(&hint.bounds().center())),
            "Filter Column must display its document shortcut"
        );
        let label = window
            .within("popup-menu")
            .within(2usize)
            .find("menu-content");
        assert_eq!(label.role(), Some(gpui_kit::Role::Label));
        assert_eq!(label.value(), Some("Clear Column Sort"));
    });
    // The first two rows offer sorting. The next available command is Filter Column,
    // beyond both unavailable clear commands and the separator.
    w.keys("down down down enter");
    assert!(w.read(|table| matches!(table.controls.drawer, Some(super::Drawer::Filter { .. }))));
    assert!(w.read(|table| table.requested_query().sorts.is_empty()));
}

#[test]
fn leaving_the_window_dismisses_a_context_menu_and_restores_its_action_context() {
    let mut w = window();
    w.ui.update(|window, _| window.activate_window());
    w.settle();
    w.ui.update(|window, _| assert!(window.is_window_active()));
    open_header_menu(&mut w);
    assert!(w.read(|t| t.controls.menu.is_some()));
    w.ui.deactivate_window();
    w.settle();
    w.ui.update(|window, _| assert!(!window.is_window_active()));
    assert!(w.read(|t| t.controls.menu.is_none()));
    w.ui.update(|window, _| window.activate_window());
    w.settle();
    let grid = w.read(|t| t.focus.clone());
    w.ui.update(|window, _| assert!(grid.is_focused(window)));
}

#[test]
fn leaving_the_window_closes_the_sort_picker_without_changing_its_draft() {
    let mut w = window();
    w.ui.update(|window, _| window.activate_window());
    w.settle();
    w.ui.update(|window, _| assert!(window.is_window_active()));
    action(&mut w, EditSort);
    click(&mut w, "add-sort");
    w.ui.update(|window, cx| window.input("score", cx));
    w.settle();
    w.ui.update(|window, _| assert_eq!(window.find("add-sort").expanded(), Some(true)));
    w.ui.deactivate_window();
    w.settle();
    w.ui.update(|window, _| assert!(!window.is_window_active()));
    w.ui.update(|window, _| assert_eq!(window.find("add-sort").expanded(), Some(false)));
    assert!(w.read(|t| t.requested_query().sorts.is_empty()));
    w.ui.update(|window, _| window.activate_window());
    w.settle();
    click(&mut w, "add-sort");
    w.ui.update(|window, _| assert!(window.find("group").visible()));
}

#[test]
fn a_failed_pending_filter_keeps_its_conditioned_column_visible() {
    use gpui_kit::{px, size};

    let mut w = window();
    w.ui.simulate_resize(size(px(960.), px(840.)));
    w.settle();
    let group = w.read(|t| t.data.columns[0].id);
    w.click(w.cell(0, 0), Modifiers::default());
    w.keys("cmd-alt-shift-l");
    assert_eq!(w.source_rows(), [0, 2, 4]);
    w.click(w.cell(0, 1), Modifiers::default());
    w.keys("cmd-alt-f");
    click(&mut w, "Range");
    w.ui.update(|window, cx| {
        window.click("Minimum", cx);
        window.input("not-a-number", cx);
        window.click("apply-filter", cx);
    });
    // Confirm before the in-flight query has returned its error.
    w.ui.update(|window, cx| window.dispatch_action(Box::new(ManageColumns), cx));
    w.ui.update(|window, cx| window.click(("toggle-column", group.into_raw() as usize), cx));
    w.ui.update(|window, cx| window.click("confirm-hide", cx));
    assert!(w.read(|t| t.data.queries().is_running() && t.data.columns.position(group).is_some()));
    w.settle();
    assert!(w.read(|t| t.data.queries().error().is_some()));
    assert!(w.read(|t| t.data.columns.position(group).is_some()));
    assert_eq!(w.source_rows(), [0, 2, 4]);
    click(&mut w, "close-table-drawer");
    click(&mut w, "filtered-count");
    w.ui.update(|window, _| {
        assert!(
            window
                .find(("locate-filter", group.into_raw() as usize))
                .visible()
        );
    });
}

#[test]
fn finishing_a_hide_closes_the_filter_editor_anchored_to_that_column() {
    for sort_already_displayed in [false, true] {
        let mut w = window();
        let group = w.read(|t| t.data.columns[0].id);
        w.ui.update(|_, cx| {
            w.table
                .update(cx, |table, cx| table.cycle_sort(0, false, cx))
        });
        if sort_already_displayed {
            w.settle();
        }
        assert_eq!(
            w.read(|t| !t.data.query().sorts.is_empty()),
            sort_already_displayed
        );
        w.ui.update(|window, cx| window.dispatch_action(Box::new(ManageColumns), cx));
        w.ui.update(|window, cx| window.click(("toggle-column", group.into_raw() as usize), cx));
        w.ui.update(|window, cx| window.click("confirm-hide", cx));
        w.ui.update(|window, cx| window.dispatch_action(Box::new(EditFilter), cx));
        assert!(
            w.read(|t| t.data.queries().is_running() && t.filter_panel_column() == Some(group))
        );
        w.settle();
        assert!(w.read(|t| t.data.columns.position(group).is_none()));
        assert_eq!(w.read(|t| t.filter_panel_column()), None);
    }
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
    w.ui.update(|window, _| {
        assert_eq!(
            window.find("table-drawer-title").label(),
            Some("Hide “group”?")
        );
        assert_eq!(
            window.find("hide-explanation").label(),
            Some("This removes filtering on this column.")
        )
    });
    w.keys("escape");
    assert_eq!(w.source_rows(), [0, 2, 4]);
    click(&mut w, ("toggle-column", group));
    w.keys("enter");
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
fn restoring_column_order_preserves_the_query_selection_widths_and_visibility() {
    use gpui_kit::{Role, point, px, test::TestQueryExt};

    let mut w = window();
    let ids = w.read(|t| t.data.column_ids(0..3));
    let group = ids[0].into_raw() as usize;
    let score = ids[1].into_raw() as usize;
    let note = ids[2].into_raw() as usize;
    w.click(w.cell(0, 0), Modifiers::default());
    w.keys("cmd-alt-shift-l");
    w.click(w.header(1), Modifiers::default());
    let score_divider = w.read(|t| {
        t.layout.bounds.origin
            + point(
                px(t.column_left(1).unwrap() + t.data.columns.width(1)),
                px(16.),
            )
    });
    w.ui.update(|window, cx| {
        window.drag(score_divider, score_divider + point(px(80.), px(0.)), cx)
    });
    w.settle();
    w.click(w.cell(0, 1), Modifiers::default());
    let view = w.read(|t| t.data.view().clone());
    let width = w.read(|t| t.data.columns.width(1));
    assert!(w.read(|t| {
        matches!(
            t.data.columns.entry(1).width,
            crate::columns::ColumnWidth::Manual(_)
        )
    }));

    action(&mut w, ManageColumns);
    w.ui.update(|window, _| {
        assert_eq!(
            window.find_by_label("Find column").role(),
            Some(Role::SearchInput)
        );
        let type_start = window.find(("toggle-column", score)).bounds().right();
        for raw in [group, note] {
            assert_eq!(
                window.find(("toggle-column", raw)).bounds().right(),
                type_start
            );
        }
        assert_eq!(
            window.find(("column-earlier", group)).disabled(),
            Some(true)
        );
        assert_eq!(window.find(("column-later", note)).disabled(), Some(true));
    });
    click(&mut w, ("toggle-column", note));
    w.ui.update(|window, _| {
        assert_eq!(
            window.find(("toggle-column", note)).bounds().right(),
            window.find(("toggle-column", score)).bounds().right(),
        );
    });
    click(&mut w, ("column-earlier", score));
    assert_eq!(w.read(|t| t.data.column_ids(0..2)), [ids[1], ids[0]]);
    click(&mut w, "restore-column-order");
    assert_eq!(w.read(|t| t.data.column_ids(0..2)), ids[..2]);
    w.ui.update(|window, _| {
        assert_eq!(window.find("restore-column-order").disabled(), Some(true));
    });
    assert!(w.read(|t| {
        Arc::ptr_eq(&view, t.data.view())
            && t.data.columns.len() == 2
            && t.data.columns.width(1) == width
            && t.selection
                .is_some_and(|s| s.head.column == 1 && s.head.row == 0)
    }));
}

#[test]
fn dragging_the_last_of_28_columns_to_the_top_scrolls_and_reorders_it() {
    use gpui_kit::{
        MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollDelta, point, px, size,
    };

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wide.csv");
    let names = (0..28)
        .map(|i| format!("measurement_column_{i:02}"))
        .collect::<Vec<_>>();
    std::fs::write(
        &path,
        format!("{}\n{}\n", names.join(","), vec!["12"; 28].join(",")),
    )
    .unwrap();
    let mut w = TableWindow::open(&path);
    w.ui.simulate_resize(size(px(320.), px(300.)));
    action(&mut w, ManageColumns);
    let ids = w.read(|table| table.data.column_ids(0..28));
    w.ui.update(|window, cx| {
        window.scroll(
            "column-list",
            ScrollDelta::Pixels(point(px(0.), px(-1000.))),
            cx,
        )
    });
    w.settle();
    let (from, to) = w.ui.update(|window, _| {
        let list = window.find("column-list").bounds();
        let panel = window.find("table-drawer").bounds();
        assert!(panel.left() >= px(0.) && panel.right() <= window.viewport_size().width);
        assert!(panel.top() >= px(0.) && panel.bottom() <= window.viewport_size().height);
        let from = window
            .find(("column-grip", ids[27].into_raw() as usize))
            .bounds()
            .center();
        assert!(list.contains(&from), "grip {from:?} outside {list:?}");
        (from, point(from.x, list.top() + px(3.)))
    });
    let modifiers = Modifiers::default();
    w.ui.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: from,
        modifiers,
        click_count: 1,
        first_mouse: false,
    });
    for at in [point(from.x, from.y - px(8.)), to] {
        w.ui.simulate_event(MouseMoveEvent {
            position: at,
            pressed_button: Some(MouseButton::Left),
            modifiers,
        });
        w.settle();
    }
    for _ in 0..60 {
        w.app
            .executor()
            .advance_clock(std::time::Duration::from_millis(16));
        w.settle();
    }
    w.ui.update(|window, _| {
        assert!(
            window
                .find(("column-grip", ids[0].into_raw() as usize))
                .visible()
        )
    });
    w.ui.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: to,
        modifiers,
        click_count: 1,
    });
    w.settle();
    assert_eq!(w.read(|table| table.data.column_ids(0..2)), [
        ids[27], ids[0]
    ]);
}

#[test]
fn filter_popover_centers_on_its_column_and_stays_inside_the_window() {
    use gpui_kit::{ScrollDelta, ScrollEvent, TouchPhase, point, px, size};

    use crate::table::HEADER_HEIGHT;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("columns.csv");
    std::fs::write(&path, "a,b,c,d,e,f,g,h,i,j\n1,2,3,4,5,6,7,8,9,10\n").unwrap();
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
    w.ui.update(|window, _| {
        let title = window.find("table-drawer-title").bounds();
        let locate = window
            .find(("locate-filter", ids[0].into_raw() as usize))
            .bounds();
        assert!((locate.left() - title.left()).abs() <= gpui_kit::px(5.));
        assert!(
            window.find("table-drawer").bounds().bottom()
                <= window.find("filtered-count").bounds().top()
        );
        assert!(window.try_find("clear-all-filters").is_none());
        assert!(window.try_find("close-table-drawer").is_none());
    });
    w.keys("escape");
    w.click(w.cell(0, 1), Modifiers::default());
    w.keys("cmd-alt-shift-l");
    click(&mut w, "filtered-count");
    w.ui.update(|window, _| {
        assert!(window.try_find("close-table-drawer").is_none());
        assert!(window.try_find("clear-all-filters").is_some());
    });
    click(&mut w, "clear-all-filters");
    assert_eq!(w.source_rows(), [0, 1, 2, 3, 4]);
    action(&mut w, EditSort);
    click(&mut w, "add-sort");
    w.ui.update(|window, cx| window.input("group", cx));
    w.settle();
    w.ui.update(|window, _| assert_eq!(window.find("group").label(), Some("group")));
    w.keys("enter");
    w.ui.update(|window, _| {
        assert!(
            window
                .try_find(("earlier-sort", ids[0].into_raw() as usize))
                .is_none()
        );
        assert!(
            window
                .try_find(("later-sort", ids[0].into_raw() as usize))
                .is_none()
        );
    });
    click(&mut w, "add-sort");
    w.ui.update(|window, cx| window.input("score", cx));
    w.settle();
    w.ui.update(|window, _| assert_eq!(window.find("score").label(), Some("score")));
    w.keys("enter");
    w.ui.update(|window, _| {
        let select = window
            .within(("sort-column", 0usize))
            .find("input")
            .bounds();
        let direction = window
            .find(("sort-direction", ids[0].into_raw() as usize))
            .bounds();
        let select_height = select.size.height + gpui_kit::px(2. / window.scale_factor());
        assert_eq!(select_height, gpui_kit::px(28.));
        assert_eq!(direction.size.height, select_height);
        assert_eq!(select.center().y, direction.center().y);
        assert!(
            window
                .try_find(("earlier-sort", ids[0].into_raw() as usize))
                .is_some()
        );
        assert!(
            window
                .try_find(("later-sort", ids[0].into_raw() as usize))
                .is_some()
        );
    });
    w.ui.update(|window, cx| window.right_click(("sort-column", 1usize), cx));
    w.settle();
    w.ui.update(|window, _| {
        assert_eq!(
            window
                .within("popup-menu")
                .within(0usize)
                .find("menu-content")
                .value(),
            Some("Move Up")
        )
    });
    w.keys("escape");
    let focus = w.read(|table| table.controls.panel_focus.clone().unwrap());
    w.ui.update(|window, cx| assert!(focus.contains_focused(window, cx)));
    w.ui.update(|window, cx| window.right_click(("sort-column", 1usize), cx));
    w.settle();
    w.keys("down enter");
    w.ui.update(|window, cx| assert!(focus.contains_focused(window, cx)));
    click(&mut w, ("sort-direction", ids[1].into_raw() as usize));
    click(&mut w, ("sort-column", 0usize));
    click(&mut w, "note");
    w.ui.update(|window, cx| {
        assert!(
            focus.contains_focused(window, cx),
            "sort column replacement lost focus"
        )
    });
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
    w.ui.update(|window, _| {
        assert!(
            window.find("table-drawer").bounds().bottom()
                <= window.find("sorted-count").bounds().top()
        );
    });
    click(&mut w, "clear-sorts");
    assert_eq!(w.source_rows(), [0, 1, 2, 3, 4]);
}

#[test]
fn export_uses_the_displayed_snapshot_from_before_the_save_panel() {
    let mut w = window();
    w.click(w.cell(0, 0), Modifiers::default());
    w.keys("cmd-alt-shift-l");
    action(&mut w, ExportTable);
    click(&mut w, "export-format");
    click(&mut w, "CSV");
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
fn return_starts_export_from_the_dialog_and_commits_a_focused_choice_locally() {
    let mut w = window();
    action(&mut w, ExportTable);
    click(&mut w, "export-format");
    w.keys("down enter");
    assert!(!w.app.did_prompt_for_new_path());
    assert!(w.read(|table| matches!(table.controls.drawer, Some(super::Drawer::Export(_)))));
    w.keys("escape");
    action(&mut w, ExportTable);
    w.keys("enter");
    assert!(w.app.did_prompt_for_new_path());
}

#[test]
fn export_announces_the_selected_format_and_compression() {
    let mut w = window();
    action(&mut w, ExportTable);
    click(&mut w, "export-compression");
    click(&mut w, "Brotli");
    w.ui.update(|window, _| {
        for (id, value) in [
            ("export-format", "Parquet"),
            ("export-compression", "Brotli"),
        ] {
            let control = window.find(id);
            assert_eq!(control.role(), Some(gpui_kit::Role::ComboBox));
            assert_eq!(control.value(), Some(value));
        }
        let save = window.find("choose-export-path");
        assert_eq!(save.role(), Some(gpui_kit::Role::Button));
        assert_eq!(save.checked(), None);
    });
}
