use std::path::Path;

use gpui_kit::{InputEvent, Modifiers, MouseButton, MouseDownEvent, px, size, test::TestWindowExt};

use super::ui_fixture::TableWindow;

fn open() -> TableWindow {
    TableWindow::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/workflow.csv"))
}

fn open_many_distinct_values() -> TableWindow {
    use std::io::Write;

    let mut file = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    writeln!(file, "tag,id\n,0").unwrap();
    for i in 1..=300 {
        writeln!(file, "station{i},{i}").unwrap();
    }
    let mut w = TableWindow::open(file.path());
    w.ui.simulate_resize(size(px(980.), px(860.)));
    w.settle();
    w
}

#[test]
fn reapplying_an_exact_filter_without_a_value_list_preserves_its_rows() {
    let mut w = open_many_distinct_values();
    w.click(w.cell(1, 0), Modifiers::default());
    w.keys("cmd-alt-shift-l");
    assert_eq!(w.source_rows(), [1]);
    w.keys("cmd-alt-f");
    w.ui.update(|window, cx| window.click("apply-filter", cx));
    w.settle();
    assert_eq!(w.source_rows(), [1]);

    w.keys("cmd-alt-f");
    w.ui.update(|window, cx| {
        assert_eq!(window.find("Current selection").selected(), Some(true));
        window.click("Text", cx);
        window.click("only-missing", cx);
        window.click("Current selection", cx);
        assert_eq!(window.find("include-missing").checked(), Some(false));
        window.click("include-missing", cx);
        window.click("apply-filter", cx);
    });
    w.settle();
    assert_eq!(w.source_rows(), [0, 1]);

    w.keys("cmd-alt-f");
    w.ui.update(|window, cx| {
        window.click("reset-filter", cx);
        window.click("apply-filter", cx);
    });
    w.settle();
    assert_eq!(w.source_rows().len(), 301);
}

#[test]
fn resetting_a_numeric_exact_filter_loads_its_range_preview() {
    let mut w = open_many_distinct_values();
    w.click(w.cell(1, 1), Modifiers::default());
    w.keys("cmd-alt-shift-l");
    w.click(w.cell(0, 1), Modifiers::default());
    w.keys("cmd-alt-f");
    w.ui.update(|window, cx| {
        assert_eq!(window.find("Current selection").selected(), Some(true));
        window.click("reset-filter", cx);
    });
    w.settle();
    w.ui.update(|window, _| assert!(window.try_find(("range-bin", 0usize)).is_some()));
    assert_eq!(w.source_rows(), [1]);
}

#[test]
fn only_missing_survives_a_round_trip_through_the_values_checklist() {
    let file = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    std::fs::write(file.path(), "number,id\n10,0\n,1\n20,2\n").unwrap();
    let mut w = TableWindow::open(file.path());
    w.ui.simulate_resize(size(px(980.), px(860.)));
    w.keys("cmd-alt-f");
    w.ui.update(|window, cx| {
        window.click("Range", cx);
        window.click("only-missing", cx);
        window.click("Values", cx);
        assert_eq!(window.find(("facet", 0usize)).checked(), Some(false));
        assert_eq!(window.find("include-missing").checked(), Some(true));
        window.click("Range", cx);
        assert_eq!(window.find("only-missing").checked(), Some(true));
        window.click("apply-filter", cx);
    });
    w.settle();
    assert_eq!(w.source_rows(), [1]);
}

#[test]
fn clicking_include_missing_after_only_missing_includes_rows_of_both_kinds() {
    use std::io::Write;

    let mut file = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    writeln!(file, "tag,id\nfound,1\n,2").unwrap();
    let mut w = TableWindow::open(file.path());
    w.ui.simulate_resize(size(px(980.), px(860.)));
    w.settle();
    w.keys("cmd-alt-f");
    w.ui.update(|window, cx| {
        window.click("Text", cx);
        window.click("only-missing", cx);
        assert_eq!(window.find("include-missing").checked(), Some(false));
        window.click("include-missing", cx);
        assert_eq!(window.find("include-missing").checked(), Some(true));
        assert_eq!(window.find("only-missing").checked(), Some(false));
        window.click("apply-filter", cx);
    });
    w.settle();
    assert_eq!(w.source_rows(), [0, 1]);
}

#[test]
fn invalid_filter_preserves_the_displayed_query_and_recovers_on_edit() {
    use std::sync::Arc;

    use gewu_core::{ColumnFilter, Filter};

    let mut w = open();
    let original = w.read(|t| t.data.view().clone());
    w.app.update(|cx| {
        w.table.update(cx, |table, cx| {
            let score = table.data.columns[1].id;
            table.edit_query(cx, |query| {
                query.filters.push(ColumnFilter {
                    column: score,
                    filter: Filter::range(Some("20".into()), Some("3".into()), false),
                });
            });
        })
    });
    w.settle();
    w.read(|t| {
        assert!(Arc::ptr_eq(&original, t.data.view()));
        assert!(t.requested_query().filters.is_empty());
        assert_eq!(
            t.data
                .queries()
                .error()
                .map(crate::errors::message)
                .as_deref(),
            Some("The minimum 20 is greater than the maximum 3.")
        );
        assert!(!t.data.queries().is_running());
    });
    w.click(w.cell(0, 1), Modifiers::default());
    w.keys("cmd-alt-shift-l");
    assert!(w.read(|t| t.data.queries().error().is_none() && t.data.row_count() < 5));
}

#[test]
fn a_copy_failure_survives_query_changes_and_clears_after_a_successful_copy() {
    let mut w = open();
    w.app.update(|cx| {
        w.table.update(cx, |table, cx| {
            table.copy_finished(Some("Could not write to the clipboard".into()), cx);
            table.cycle_sort(1, false, cx);
        })
    });
    w.settle();
    assert!(w.ui.update(|window, _| window.try_find("operation-notice").is_some()));
    w.click(w.cell(0, 0), Modifiers::default());
    w.keys("cmd-c");
    assert!(w.clipboard().is_some());
    assert!(w.ui.update(|window, _| window.try_find("operation-notice").is_none()));
}

#[test]
fn copy_honors_the_header_preference() {
    let mut w = open();
    w.app.update(|cx| {
        crate::preferences::Preferences::update(cx, |p| p.copy_includes_headers = true)
    });
    w.click(w.cell(1, 1), Modifiers::default());
    w.keys("cmd-c");
    assert_eq!(
        w.clipboard().as_deref(),
        Some("group\tscore\tnote\r\ndrop\t99\tmarker-hidden\r\n")
    );
}

#[test]
fn a_filter_that_keeps_every_row_stays_in_the_summary_until_its_replacement_applies() {
    use gewu_core::{ColumnFilter, Filter};

    let mut w = open();
    let score = w.read(|t| t.data.columns[1].id);
    let raw = score.into_raw() as usize;
    w.app.update(|cx| {
        w.table.update(cx, |t, cx| {
            t.edit_query(cx, |q| {
                q.filters.push(ColumnFilter {
                    column: score,
                    filter: Filter::range(Some("0".into()), None, false),
                })
            });
        })
    });
    w.settle();
    assert_eq!(w.source_rows().len(), 5);
    w.ui.update(|window, cx| {
        assert_eq!(window.find("filtered-count").label(), Some("1 filtered"));
        window.click("filtered-count", cx);
    });
    w.settle();
    w.ui.update(|window, cx| window.click(("locate-filter", raw), cx));
    w.settle();
    assert_eq!(w.selection().unwrap().head.column, 1);
    w.ui.update(|window, cx| window.click("filtered-count", cx));
    w.settle();
    w.ui.update(|window, cx| {
        window.click(("clear-filter", raw), cx);
        let table = w.table.read(cx);
        assert!(table.data.queries().is_running());
        assert!(table.requested_query().filters.is_empty());
        assert_eq!(table.data.query().filters.len(), 1);
        window.render_frame(cx);
        assert_eq!(window.find("filtered-count").label(), Some("1 filtered"));
    });
    w.settle();
    assert!(w.ui.update(|window, _| window.try_find("filtered-count").is_none()));
    assert_eq!(w.source_rows().len(), 5);
}

#[test]
fn export_panel_writes_each_format_and_selected_parquet_codec() {
    let mut w = open();
    w.click(w.cell(0, 0), Modifiers::default());
    w.keys("cmd-alt-shift-l");
    let directory = tempfile::tempdir().unwrap();
    for (format, option) in [
        ("csv", None),
        ("tsv", None),
        ("xlsx", None),
        ("parquet", Some(("Zstd", "ZSTD"))),
        ("parquet", Some(("Snappy", "SNAPPY"))),
        ("parquet", Some(("Gzip", "GZIP"))),
        ("parquet", Some(("Brotli", "BROTLI"))),
        ("parquet", Some(("LZ4", "LZ4_RAW"))),
        ("parquet", Some(("Uncompressed", "UNCOMPRESSED"))),
    ] {
        w.keys("cmd-shift-e");
        w.ui.update(|window, cx| window.click("export-format", cx));
        w.settle();
        let label = if format == "parquet" {
            "Parquet".to_owned()
        } else {
            format.to_uppercase()
        };
        w.ui.update(|window, cx| window.click(label, cx));
        w.settle();
        if let Some((label, _)) = option {
            w.ui.update(|window, cx| window.click("export-compression", cx));
            w.settle();
            w.ui.update(|window, cx| window.click(label, cx));
            w.settle();
        }
        w.ui.update(|window, cx| window.click("choose-export-path", cx));
        w.settle();
        assert!(w.app.did_prompt_for_new_path());
        let dest = directory.path().join(format!(
            "filtered-{}.{format}",
            option.map_or("rows", |o| o.0)
        ));
        w.app.simulate_new_path_selection(|_| Some(dest.clone()));
        w.settle();
        let result = gewu_core::Dataset::open(dest).unwrap();
        assert_eq!(result.n_rows(), 3);
        let info = result.info();
        assert_eq!(info.columns.len(), 3);
        assert!(info.format.to_string().eq_ignore_ascii_case(format));
        if let Some((_, expected)) = option {
            let gewu_core::FileMetadata::Parquet { compression, .. } = info.metadata else {
                panic!("missing Parquet metadata");
            };
            assert_eq!(compression.len(), 1);
            assert_eq!(compression[0].split('(').next(), Some(expected));
        }
    }
}

#[test]
fn copying_a_cell_supersedes_an_unfinished_row_copy() {
    let mut w = open();
    w.click(w.cell(1, 2), Modifiers::default());
    w.ui.update(|window, cx| {
        window.press("cmd-c", cx);
        w.table.update(cx, |t, cx| {
            assert!(t.copy_task.is_some());
            let cell = t.selection.unwrap().head;
            let text = t.data.cell(cell.column, cell.row).unwrap().unwrap();
            t.copy_cell_text(text.to_string(), cx);
        });
    });
    w.settle();
    assert_eq!(w.clipboard().as_deref(), Some("marker-hidden"));
}

#[test]
fn copy_and_cut_in_the_find_field_supersede_pending_row_copies() {
    let mut w = open();
    w.click(w.cell(1, 2), Modifiers::default());
    w.keys("cmd-f");
    w.type_in_find("marker");
    for command in ["cmd-c", "cmd-x"] {
        w.keys("cmd-a");
        w.ui.update(|window, cx| {
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("previous".into()));
            w.table.update(cx, |table, cx| table.copy(window, cx));
            window.press(command, cx);
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().as_deref(),
                Some("marker")
            );
        });
        w.settle();
        assert_eq!(w.clipboard().as_deref(), Some("marker"));
    }
    // Cut left no text selected. A subsequent no-op Copy must let the row copy finish.
    w.ui.update(|window, cx| {
        w.table.update(cx, |table, cx| table.copy(window, cx));
        window.press("cmd-c", cx);
    });
    w.settle();
    assert_eq!(
        w.clipboard().as_deref(),
        Some("drop\t99\tmarker-hidden\r\n")
    );
}

#[test]
fn the_newest_copy_wins_across_document_windows() {
    use gpui_kit::AppContext;

    let mut w = open();
    w.click(w.cell(1, 2), Modifiers::default());
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/workflow.csv");
    let (_, other) = w.app.update(|cx| {
        let data = crate::data::TableData::open(path.to_str().unwrap()).unwrap();
        gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| super::TableView::new(data, window, cx))
        })
        .unwrap()
    });
    w.ui.update(|window, cx| {
        window.press("cmd-c", cx);
        assert!(w.table.read(cx).copy_task.is_some());
        other.update(cx, |t, cx| {
            t.copy_cell_text("newer cell".to_string(), cx);
        });
    });
    w.settle();
    assert_eq!(w.clipboard().as_deref(), Some("newer cell"));
    w.keys("cmd-c");
    assert_eq!(
        w.clipboard().as_deref(),
        Some("drop\t99\tmarker-hidden\r\n")
    );
}

#[test]
fn empty_filtered_grid_recovers_only_when_the_reset_draft_is_applied() {
    let mut w = open();
    let edit = |w: &mut TableWindow| {
        w.ui.update(|window, cx| window.dispatch_action(Box::new(crate::actions::EditFilter), cx));
        w.settle();
    };
    let click = |w: &mut TableWindow, id: &'static str| {
        w.ui.update(|window, cx| window.click(id, cx));
        w.settle();
    };
    edit(&mut w);
    click(&mut w, "no-values");
    click(&mut w, "apply-filter");
    assert!(w.source_rows().is_empty());
    w.keys("cmd-a down end");
    assert_eq!(w.selection(), None);
    w.keys("cmd-f");
    w.type_in_find("marker");
    assert_eq!(
        w.app.read(|cx| w.table.read(cx).find.read(cx).message(cx)),
        "No matches"
    );
    w.keys("escape");
    click(&mut w, "empty-edit-filter");
    click(&mut w, "reset-filter");
    click(&mut w, "cancel-filter");
    assert!(w.source_rows().is_empty());
    edit(&mut w);
    click(&mut w, "reset-filter");
    click(&mut w, "apply-filter");
    assert_eq!(w.source_rows(), [0, 1, 2, 3, 4]);
    edit(&mut w);
    click(&mut w, "no-values");
    click(&mut w, "apply-filter");
    click(&mut w, "empty-clear-filters");
    assert_eq!(w.source_rows(), [0, 1, 2, 3, 4]);
}

#[test]
fn resizing_below_the_header_and_gutter_restores_the_same_selection() {
    let mut w = open();
    w.click(w.cell(2, 1), Modifiers::default());
    let selected = w.selection();
    w.ui.simulate_resize(size(px(14.), px(14.)));
    w.settle();
    w.ui.simulate_resize(size(px(680.), px(420.)));
    w.settle();
    assert_eq!(w.selection(), selected);
    w.keys("cmd-c");
    assert_eq!(w.clipboard().unwrap().lines().count(), 1);
}

#[test]
fn reversing_a_pending_sort_restores_find_on_the_original_view() {
    let mut w = open();
    w.keys("cmd-f");
    w.type_in_find("marker");
    let header = w.header(1);
    let original = w.read(|t| t.data.view().clone());
    w.ui.update(|window, cx| {
        for _ in 0..3 {
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position: header,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
        }
    });
    w.settle();
    assert!(w.read(|t| std::sync::Arc::ptr_eq(&original, t.data.view())));
    w.keys("cmd-g");
    assert_eq!(w.selection().unwrap().head.row, 2);
}

fn grid_frame(w: &mut TableWindow) -> super::Frame {
    w.ui.update(|window, cx| {
        w.table.update(cx, |t, cx| {
            let geometry = t.prepare_grid_layout(t.layout.bounds, window);
            t.build_frame(geometry.layout, cx)
        })
    })
}

#[test]
fn find_marks_the_preview_ellipsis_for_hits_after_a_newline_or_the_byte_limit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hidden.csv");
    let long = "α".repeat(crate::data::GRID_PREVIEW_BYTES);
    std::fs::write(
        &path,
        format!("note\n{long}needle\n\"prefix\nneedle\"\nplain\n"),
    )
    .unwrap();
    let mut w = TableWindow::open(&path);
    w.keys("cmd-f");
    w.type_in_find("needle");
    let frame = grid_frame(&mut w);
    let cells = &frame.columns[0].cells;
    assert_eq!(cells.len(), 3);
    for cell in &cells[..2] {
        let spans = &frame.text_matches[cell.spans.clone()];
        assert_eq!(spans.len(), 1);
        assert_eq!(&cell.text[spans[0].clone()], "…");
    }
    assert!(cells[0].current_match);
    assert!(!cells[1].current_match);
    assert!(cells[2].spans.is_empty());
}

#[test]
fn find_outline_tracks_the_current_result_independently_of_the_grid_selection() {
    let mut w = open();
    w.keys("cmd-f");
    w.type_in_find("marker");
    let initial = grid_frame(&mut w);
    assert_eq!(initial.text_matches.len(), 3);
    let first = initial.current_match.unwrap();
    assert_eq!(initial.head, Some(first));

    w.click(w.cell(0, 0), Modifiers::default());
    let after_click = grid_frame(&mut w);
    assert_eq!(after_click.current_match, Some(first));
    assert_ne!(after_click.head, after_click.current_match);
    assert_eq!(after_click.text_matches, initial.text_matches);

    w.keys("cmd-g");
    let next = grid_frame(&mut w);
    let second_y = initial.columns[2].cells[2].y;
    assert_eq!(next.current_match.unwrap().origin.y, px(second_y));
    assert_eq!(next.head, next.current_match);
    w.keys("escape");
    let closed = grid_frame(&mut w);
    assert!(closed.current_match.is_none());
    assert!(closed.text_matches.is_empty());
}

#[test]
fn typing_replaces_a_pending_search_and_reordering_keeps_the_current_match() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("matches.csv");
    std::fs::write(
        &path,
        "a,b,note\n1,x,plain\n2,x,marker\n3,marker,marker marker\n",
    )
    .unwrap();
    let mut w = TableWindow::open(&path);
    w.keys("cmd-f");
    w.ui.update(|window, cx| window.input("mar", cx));
    w.ui.run_until_parked();
    assert_eq!(w.selection(), None);
    w.type_in_find("ker");
    assert_eq!(w.selection().unwrap().head.row, 1);
    w.keys("cmd-g cmd-g");
    assert_eq!(w.selection().unwrap().head.row, 2);
    w.ui.update(|window, _| assert_eq!(window.find("find-count").label(), Some("Match 3 of 3")));
    let note = w.read(|t| t.data.columns[2].id.into_raw() as usize);
    w.ui.update(|window, cx| window.dispatch_action(Box::new(crate::actions::ManageColumns), cx));
    w.settle();
    w.ui.update(|window, cx| window.click(("column-earlier", note), cx));
    w.settle();
    let head = w.selection().unwrap().head;
    assert_eq!((head.row, head.column), (2, 1));
    w.ui.update(|window, _| assert_eq!(window.find("find-count").label(), Some("Match 2 of 3")));
    w.ui.update(|window, cx| window.click("close-table-drawer", cx));
    w.keys("cmd-g");
    let head = w.selection().unwrap().head;
    assert_eq!((head.row, head.column), (2, 2));
}

#[test]
fn hiding_columns_rebuilds_live_find_and_zero_columns_have_no_selection() {
    let mut w = open();
    w.keys("cmd-f");
    w.type_in_find("marker");
    let (group, note) = w.read(|t| {
        (
            t.data.columns[0].id.into_raw() as usize,
            t.data.columns[2].id,
        )
    });
    w.ui.update(|window, cx| window.dispatch_action(Box::new(crate::actions::ManageColumns), cx));
    w.settle();
    w.ui.update(|window, cx| window.click(("toggle-column", group), cx));
    w.settle();
    w.app.read(|cx| {
        assert_eq!(
            w.table.read(cx).find.read(cx).cached_match(note, 1),
            Some(true)
        )
    });
    w.ui.update(|window, cx| window.click("hide-matching", cx));
    w.settle();
    assert_eq!(w.read(|t| t.data.columns.len()), 0);
    assert_eq!(w.selection(), None);
    w.ui.update(|window, cx| window.click("close-table-drawer", cx));
    w.keys("escape cmd-a down cmd-c");
    assert_eq!(w.selection(), None);
}
