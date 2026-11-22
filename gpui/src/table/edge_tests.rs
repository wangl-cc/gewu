use std::path::Path;

use gpui_kit::{InputEvent, Modifiers, MouseButton, MouseDownEvent, px, size, test::TestWindowExt};

use super::ui_fixture::TableWindow;

fn open() -> TableWindow {
    TableWindow::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/workflow.csv"))
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
        w.ui.update(|window, cx| window.click(format, cx));
        w.settle();
        if let Some((label, _)) = option {
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
        w.app
            .read(|cx| w.table.read(cx).find.read(cx).message.clone()),
        "No matches"
    );
    w.keys("escape");
    edit(&mut w);
    click(&mut w, "reset-filter");
    click(&mut w, "cancel-filter");
    assert!(w.source_rows().is_empty());
    edit(&mut w);
    click(&mut w, "reset-filter");
    click(&mut w, "apply-filter");
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

#[test]
fn find_outline_tracks_the_current_result_independently_of_the_grid_selection() {
    let mut w = open();
    let frame = |w: &mut TableWindow| {
        w.app.update(|cx| {
            w.table.update(cx, |t, cx| {
                let layout = t.compute_layout(t.layout.bounds);
                t.build_frame(layout.into(), cx)
            })
        })
    };

    w.keys("cmd-f");
    w.type_in_find("marker");
    let initial = frame(&mut w);
    assert_eq!(initial.matches.len(), 3);
    let first = initial.current_match.unwrap();
    assert_eq!(initial.head, Some(first));

    w.click(w.cell(0, 0), Modifiers::default());
    let after_click = frame(&mut w);
    assert_eq!(after_click.current_match, Some(first));
    assert_ne!(after_click.head, after_click.current_match);
    assert_eq!(after_click.matches, initial.matches);

    w.keys("cmd-g");
    let next = frame(&mut w);
    assert_eq!(next.current_match, Some(initial.matches[1]));
    assert_eq!(next.head, next.current_match);
    w.keys("escape");
    let closed = frame(&mut w);
    assert!(closed.current_match.is_none());
    assert!(closed.matches.is_empty());
}

#[test]
fn typing_replaces_a_pending_search_and_reordering_keeps_the_current_match() {
    let mut w = open();
    w.keys("cmd-f");
    w.ui.update(|window, cx| window.input("mar", cx));
    w.ui.run_until_parked();
    assert_eq!(w.selection(), None);
    w.type_in_find("ker");
    assert_eq!(w.selection().unwrap().head.row, 1);
    w.keys("cmd-g");
    assert_eq!(w.selection().unwrap().head.row, 2);
    let note = w.read(|t| t.data.columns[2].id.into_raw() as usize);
    w.ui.update(|window, cx| window.dispatch_action(Box::new(crate::actions::ManageColumns), cx));
    w.settle();
    w.ui.update(|window, cx| window.click(("column-earlier", note), cx));
    w.settle();
    let head = w.selection().unwrap().head;
    assert_eq!((head.row, head.column), (2, 1));
    w.ui.update(|window, cx| window.click("close-table-drawer", cx));
    w.keys("cmd-g");
    let head = w.selection().unwrap().head;
    assert_eq!((head.row, head.column), (4, 1));
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
    w.app
        .read(|cx| assert!(w.table.read(cx).find.read(cx).matched(note, 1)));
    w.ui.update(|window, cx| window.click("hide-matching", cx));
    w.settle();
    assert_eq!(w.read(|t| t.data.columns.len()), 0);
    assert_eq!(w.selection(), None);
    w.ui.update(|window, cx| window.click("close-table-drawer", cx));
    w.keys("escape cmd-a down cmd-c");
    assert_eq!(w.selection(), None);
}
