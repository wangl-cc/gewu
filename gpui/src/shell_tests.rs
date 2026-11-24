use std::{fs, path::PathBuf};

use gpui_kit::{
    Entity, ExternalPaths, FileDropEvent, InputEvent, TestAppContext, VisualTestContext,
    base::Root, point, px, test::TestWindowExt,
};

use super::{Document, Load};
use crate::shell::{
    ClearRecent, Documents, GetInfo, ImportOptions, OpenFiles, install_file_actions,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

struct Session {
    app: TestAppContext,
    finder: async_channel::Sender<Vec<PathBuf>>,
}

impl Session {
    fn new() -> Self {
        let app = TestAppContext::single();
        let (finder, receive) = async_channel::unbounded();
        app.update(|cx| {
            gpui_kit::init(cx);
            crate::actions::init(cx);
            install_file_actions(
                crate::platform::RecentDocuments::Session(Vec::new()),
                receive,
                cx,
            );
        });
        Self { app, finder }
    }

    fn open(&self, paths: Vec<PathBuf>) {
        self.finder.try_send(paths).unwrap();
        self.app.run_until_parked();
    }

    fn viewer(&self, index: usize) -> (Entity<Document>, VisualTestContext) {
        let handle = self
            .app
            .read(|cx| cx.global::<Documents>().windows[index].1);
        let doc = handle
            .read(&self.app, |root: Entity<Root>, cx| {
                root.read(cx)
                    .view()
                    .clone()
                    .downcast::<Document>()
                    .ok()
                    .unwrap()
            })
            .unwrap();
        (doc, VisualTestContext::from_window(handle, &self.app))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.app.quit();
    }
}

#[test]
fn picker_finder_and_file_drop_share_window_identity() {
    let session = Session::new();
    let file = fixture("multi-sheet.xlsx");
    session.app.update(|cx| cx.dispatch_action(&OpenFiles));
    session.app.run_until_parked();
    assert!(session.app.did_prompt_for_paths());
    session.app.simulate_path_prompt_response(|options| {
        assert!(options.files && options.multiple && !options.directories);
        Some(vec![file.clone()])
    });
    session.app.run_until_parked();
    let folder = tempfile::tempdir().unwrap();
    let alias = folder.path().join("alias.xlsx");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&file, &alias).unwrap();
    #[cfg(not(unix))]
    let alias = file.clone();
    session.open(vec![alias]);
    session.app.read(|cx| assert_eq!(cx.windows().len(), 1));
    let (_, mut window) = session.viewer(0);
    let second = fixture("unicode-sheets.xlsx");
    window.update(|window, cx| {
        window.render_frame(cx);
        let position = point(px(50.), px(70.));
        window.dispatch_event(
            FileDropEvent::Entered {
                position,
                paths: ExternalPaths([second.clone()].into_iter().collect()),
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(FileDropEvent::Submit { position }.to_platform_input(), cx);
    });
    session.app.run_until_parked();
    session.app.read(|cx| {
        assert_eq!(cx.windows().len(), 2);
        assert_eq!(cx.global::<Documents>().recent.paths(), [
            second.canonicalize().unwrap(),
            file.canonicalize().unwrap()
        ]);
    });
}

#[test]
fn picker_header_choice_applies_to_eligible_files_and_parquet_offers_recovery() {
    let folder = tempfile::tempdir().unwrap();
    let csv = folder.path().join("sample.CSV");
    let tsv = folder.path().join("sample.tsv");
    let parquet = folder.path().join("damaged.parquet");
    fs::write(&csv, "first,second\n1,2\n").unwrap();
    fs::write(&tsv, "first\tsecond\n1\t2\n").unwrap();
    fs::write(&parquet, b"PAR1broken footer").unwrap();
    let session = Session::new();
    session.app.update(|cx| {
        crate::shell::open_with_header(
            vec![csv, tsv, fixture("multi-sheet.xlsx"), parquet],
            false,
            cx,
        );
    });
    session.app.run_until_parked();
    for index in 0..3 {
        let (doc, _) = session.viewer(index);
        doc.read_with(&session.app, |doc, cx| {
            assert!(doc.header_available && !doc.displayed.has_header);
            if index < 2 {
                assert_eq!(doc.table.as_ref().unwrap().read(cx).data.row_count(), 2);
            }
        });
    }
    let (doc, mut ui) = session.viewer(3);
    doc.read_with(&session.app, |doc, _| assert!(!doc.header_available));
    ui.update(|w, cx| {
        w.render_frame(cx);
        assert!(!w.is_action_available(&ImportOptions, cx));
        assert!(w.try_find("header-mode").is_none());
        assert!(w.try_find("open-error-text").is_none());
        w.click("open-error-details", cx);
        w.render_frame(cx);
        let details = w.find("open-error-text");
        assert_eq!(
            details.label().unwrap().matches("Parquet error:").count(),
            1
        );
        w.click("open-another-file", cx);
    });
    session.app.run_until_parked();
    assert!(session.app.did_prompt_for_paths());
}

#[test]
fn finder_import_options_warn_before_removing_conditions_and_allow_cancel() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("samples.csv");
    fs::write(&file, "group\nx\ny\nx\n").unwrap();
    let session = Session::new();
    session.open(vec![file]);
    let (doc, mut ui) = session.viewer(0);
    ui.update(|w, cx| {
        w.press("home", cx);
        w.press("cmd-alt-shift-l", cx);
    });
    ui.run_until_parked();
    ui.dispatch_action(ImportOptions);
    ui.run_until_parked();
    let (_, detail) = session.app.pending_prompt().unwrap();
    assert!(detail.contains("clear Find, filters and sorting"));
    session.app.simulate_prompt_answer("Cancel");
    ui.run_until_parked();
    doc.read_with(&session.app, |doc, cx| {
        assert!(doc.displayed.has_header);
        assert_eq!(doc.table.as_ref().unwrap().read(cx).data.row_count(), 2);
    });
    ui.dispatch_action(ImportOptions);
    ui.run_until_parked();
    session.app.simulate_prompt_answer("Read as Data");
    ui.run_until_parked();
    doc.read_with(&session.app, |doc, cx| {
        assert!(!doc.displayed.has_header);
        assert_eq!(doc.table.as_ref().unwrap().read(cx).data.row_count(), 4);
    });
}

#[test]
fn import_menu_follows_the_active_file_format() {
    use gewu_core::{ColumnId, Dataset, FileType};

    let folder = tempfile::tempdir().unwrap();
    let csv = folder.path().join("source.csv");
    let parquet = folder.path().join("typed.parquet");
    fs::write(&csv, "score\n1\n2\n").unwrap();
    std::sync::Arc::new(Dataset::open(&csv).unwrap())
        .unfiltered()
        .unsorted()
        .export(&[ColumnId::from_raw(0)], FileType::Parquet)
        .unwrap()
        .write(&parquet)
        .unwrap();
    let session = Session::new();
    session.open(vec![csv.clone(), parquet.clone()]);
    for (path, available) in [(csv.clone(), true), (parquet, false), (csv, true)] {
        session.open(vec![path]);
        session
            .app
            .update(|cx| assert_eq!(cx.is_action_available(&ImportOptions), available));
    }
}

#[test]
fn reopening_a_document_preserves_its_query_and_other_windows_queries() {
    let folder = tempfile::tempdir().unwrap();
    let first = folder.path().join("first.csv");
    let second = folder.path().join("second.csv");
    fs::write(&first, "group\na\nb\na\n").unwrap();
    fs::write(&second, "group\nx\ny\nz\ny\n").unwrap();
    let session = Session::new();
    session.open(vec![first.clone(), second]);
    let (first_doc, mut first_window) = session.viewer(0);
    let (second_doc, mut second_window) = session.viewer(1);
    let rows = |doc: &Entity<Document>| {
        doc.read_with(&session.app, |doc, cx| {
            doc.table.as_ref().unwrap().read(cx).data.row_count()
        })
    };
    let filter_first_row = |window: &mut VisualTestContext| {
        window.update(|w, cx| {
            w.render_frame(cx);
            w.press("home", cx);
            w.press("cmd-alt-shift-l", cx);
        });
        window.run_until_parked();
    };
    filter_first_row(&mut first_window);
    assert_eq!((rows(&first_doc), rows(&second_doc)), (2, 4));
    filter_first_row(&mut second_window);
    assert_eq!((rows(&first_doc), rows(&second_doc)), (2, 1));
    session.open(vec![first]);
    assert_eq!(session.viewer(0).0, first_doc);
    assert_eq!((rows(&first_doc), rows(&second_doc)), (2, 1));
    first_window.dispatch_action(crate::actions::ClearFilters);
    first_window.run_until_parked();
    assert_eq!((rows(&first_doc), rows(&second_doc)), (3, 1));
}

#[test]
fn sheet_switches_keep_the_shown_table_on_failure_or_cancellation() {
    let session = Session::new();
    session.open(vec![fixture("multi-sheet.xlsx")]);
    let (doc, mut ui) = session.viewer(0);
    let original = doc.read_with(&session.app, |doc, _| {
        assert_eq!(doc.displayed.sheet.as_deref(), Some("First"));
        doc.table.clone().unwrap()
    });
    let index = |sheet| {
        doc.read_with(&session.app, |doc, _| {
            doc.sheets.iter().position(|s| s == sheet).unwrap()
        })
    };
    let (bad, last, first) = (index("Bad"), index("Last"), index("First"));
    ui.update(|w, cx| w.click(("sheet", bad), cx));
    ui.run_until_parked();
    doc.read_with(&session.app, |doc, _| {
        assert!(
            matches!(&doc.load, Load::Failed(_, error) if error.to_string().contains("#DIV/0!"))
        );
        assert_eq!(doc.table.as_ref(), Some(&original));
    });
    ui.update(|w, cx| {
        w.click(("sheet", last), cx);
        w.click(("sheet", first), cx);
    });
    ui.run_until_parked();
    doc.read_with(&session.app, |doc, _| {
        assert!(matches!(doc.load, Load::Ready));
        assert_eq!(doc.table.as_ref(), Some(&original));
    });
    ui.update(|w, cx| w.click(("sheet", last), cx));
    ui.run_until_parked();
    doc.read_with(&session.app, |doc, _| {
        assert_eq!(doc.displayed.sheet.as_deref(), Some("Last"));
        assert_ne!(doc.table.as_ref(), Some(&original));
    });
}

#[test]
fn a_narrow_workbook_shares_one_footer_between_worksheets_and_status() {
    let session = Session::new();
    session.open(vec![fixture("multi-sheet.xlsx")]);
    let (doc, mut ui) = session.viewer(0);
    ui.simulate_resize(gpui_kit::size(px(340.), px(440.)));
    let last = doc.read_with(&session.app, |doc, _| {
        doc.sheets.iter().position(|s| s == "Last").unwrap()
    });
    ui.update(|w, cx| {
        w.render_frame(cx);
        let footer = w.find("document-footer").bounds();
        let tabs = w
            .within("document-footer")
            .find("worksheet-options")
            .bounds();
        let status = w.within("document-footer").find("footer-status").bounds();
        assert_eq!(tabs.center().y, status.center().y);
        assert!(footer.contains(&tabs.center()) && footer.contains(&status.center()));
        assert!(tabs.right() <= status.left() && status.right() < w.viewport_size().width);
        assert!(w.find(("sheet", last)).visible());
        w.click(("sheet", last), cx);
    });
    ui.run_until_parked();
    ui.update(|w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find(("sheet", last)).selected(), Some(true));
    });
    doc.read_with(&session.app, |doc, _| {
        assert_eq!(doc.displayed.sheet.as_deref(), Some("Last"))
    });
}

#[test]
fn worksheet_and_header_edits_compose_before_the_reader_finishes() {
    let session = Session::new();
    session.open(vec![fixture("multi-sheet.xlsx")]);
    let (doc, mut ui) = session.viewer(0);
    let (first, last) = doc.read_with(&session.app, |doc, _| {
        let index = |name| doc.sheets.iter().position(|s| s == name).unwrap();
        (index("First"), index("Last"))
    });

    ui.update(|w, cx| {
        w.click(("sheet", last), cx);
        w.render_frame(cx);
        assert!(w.try_find("header-mode").is_none());
        w.dispatch_action(Box::new(ImportOptions), cx);
    });
    ui.run_until_parked();
    session.app.simulate_prompt_answer("Read as Data");
    ui.run_until_parked();
    doc.read_with(&session.app, |doc, _| {
        assert_eq!(doc.displayed.sheet.as_deref(), Some("Last"));
        assert!(!doc.displayed.has_header);
    });

    ui.dispatch_action(ImportOptions);
    ui.run_until_parked();
    session.app.simulate_prompt_answer("Use as Column Names");
    ui.update(|w, cx| w.click(("sheet", first), cx));
    ui.run_until_parked();
    doc.read_with(&session.app, |doc, _| {
        assert_eq!(doc.displayed.sheet.as_deref(), Some("First"));
        assert!(doc.displayed.has_header);
    });
}

#[test]
fn document_title_search_and_inspector_share_the_top_row_in_narrow_windows() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("a-long-document-name-with-rows.csv");
    fs::write(&path, "name\none\ntwo\n").unwrap();
    let session = Session::new();
    session.open(vec![path]);
    let (doc, mut ui) = session.viewer(0);
    ui.simulate_resize(gpui_kit::size(px(380.), px(300.)));
    ui.update(|w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("worksheet-options").is_none());
        assert!(w.try_find("header-mode").is_none());
        let title = w.find("document-title").bounds();
        let field = w.find("table-search").bounds();
        let inspector = w.find("show-inspector").bounds();
        assert!(title.right() <= field.left());
        assert!(field.right() < inspector.left());
        assert!(title.contains(&point(title.center().x, inspector.center().y)));
        assert!(field.contains(&point(field.center().x, inspector.center().y)));
        let expanded = w.find("show-inspector").expanded();
        w.click("show-inspector", cx);
        assert_eq!(w.find("show-inspector").expanded(), expanded.map(|v| !v));
        w.click("table-search", cx);
    });
    ui.run_until_parked();
    ui.update(|w, cx| {
        w.input("two", cx);
    });
    ui.run_until_parked();
    ui.update(|w, cx| {
        w.render_frame(cx);
        let field = w.find("table-search").bounds();
        let table = doc.read(cx).table.as_ref().unwrap().read(cx);
        assert!(table.find.read(cx).input_focused(w, cx));
        assert_eq!(table.find.read(cx).text(cx).as_ref(), "two");
        assert!(field.size.width >= px(70.));
        let center = w.find("show-inspector").bounds().center().y;
        for id in ["table-search", "previous", "next", "close-find"] {
            assert!(
                (w.find(id).bounds().center().y - center).abs() <= px(1.),
                "{id} should be vertically centered in the title bar"
            );
        }
        assert!(w.find("close-find").bounds().right() < w.find("show-inspector").bounds().left());
        assert!(
            field.contains(&w.find("close-find").bounds().center()),
            "clear button should live inside the search field"
        );
        w.click("close-find", cx);
    });
    ui.run_until_parked();
    ui.update(|w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("close-find").is_none());
        assert!(
            doc.read(cx)
                .table
                .as_ref()
                .unwrap()
                .read(cx)
                .focus
                .is_focused(w)
        );
        w.click("table-search", cx);
    });
    ui.run_until_parked();
    ui.update(|w, cx| w.input("two", cx));
    ui.run_until_parked();
    ui.update(|w, cx| {
        w.press("escape", cx);
        assert!(
            doc.read(cx)
                .table
                .as_ref()
                .unwrap()
                .read(cx)
                .focus
                .is_focused(w)
        );
    });
}

#[test]
fn unreadable_first_sheet_leaves_the_workbook_catalog_available() {
    let session = Session::new();
    session.open(vec![fixture("unreadable-first.xlsx")]);
    let (doc, mut ui) = session.viewer(0);
    ui.update(|w, cx| {
        w.render_frame(cx);
        assert!(w.find("document-title").bounds().size.width > px(84.));
        assert!(w.try_find("table-search").is_none());
        assert!(w.is_action_available(&ImportOptions, cx));
    });
    let first = doc.read_with(&session.app, |doc, cx| {
        assert!(doc.table.is_none());
        assert!(
            matches!(&doc.load, Load::Failed(_, error) if error.to_string().contains("#DIV/0!"))
        );
        assert!(cx.global::<Documents>().recent.paths().is_empty());
        assert_eq!(doc.sheets[0], "Bad");
        doc.sheets.iter().position(|s| s == "First").unwrap()
    });
    ui.update(|w, cx| w.click(("sheet", first), cx));
    ui.run_until_parked();
    doc.read_with(&session.app, |doc, cx| {
        assert!(matches!(doc.load, Load::Ready));
        assert_eq!(doc.displayed.sheet.as_deref(), Some("First"));
        assert_eq!(
            cx.global::<Documents>().recent.paths(),
            std::slice::from_ref(&doc.path)
        );
    });
}

#[test]
fn chart_page_and_point_limit_survive_replacing_the_worksheet() {
    let session = Session::new();
    session.open(vec![fixture("multi-sheet.xlsx")]);
    let (doc, mut ui) = session.viewer(0);
    let last = doc.read_with(&session.app, |d, _| {
        d.sheets.iter().position(|s| s == "Last").unwrap()
    });
    ui.update(|w, cx| w.click("Chart", cx));
    ui.run_until_parked();
    ui.update(|w, cx| w.click("scatter-limits", cx));
    ui.run_until_parked();
    ui.update(|w, cx| w.click(("points", 512usize), cx));
    ui.run_until_parked();
    ui.update(|w, cx| w.click("Column", cx));
    ui.run_until_parked();
    ui.update(|w, cx| w.click("Chart", cx));
    ui.run_until_parked();
    ui.update(|w, cx| {
        assert_eq!(w.find("scatter-limits").value(), Some("512"));
        w.click(("sheet", last), cx);
    });
    ui.run_until_parked();
    doc.read_with(&session.app, |d, _| {
        assert_eq!(d.displayed.sheet.as_deref(), Some("Last"))
    });
    ui.update(|w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find("Chart").selected(), Some(true));
        assert_eq!(w.find("scatter-limits").value(), Some("512"));
    });
}

#[test]
fn failed_header_reload_retries_the_requested_options() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("values.csv");
    let csv = "first,second\n1,2\n";
    fs::write(&file, csv).unwrap();
    let session = Session::new();
    session.open(vec![file.clone()]);
    let (doc, mut ui) = session.viewer(0);
    let original = doc.read_with(&session.app, |doc, _| doc.table.clone().unwrap());
    let moved = dir.path().join("held.csv");
    fs::rename(&file, &moved).unwrap();
    ui.dispatch_action(ImportOptions);
    ui.run_until_parked();
    session.app.simulate_prompt_answer("Read as Data");
    ui.run_until_parked();
    doc.read_with(&session.app, |doc, _| {
        assert!(matches!(doc.load, Load::Failed(_, _)));
        assert!(doc.displayed.has_header);
        assert_eq!(doc.table.as_ref(), Some(&original));
    });
    fs::rename(moved, file).unwrap();
    ui.update(|w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("open-another-file").is_none());
        assert!(w.find("table-search").visible());
        w.click("retry-open", cx);
    });
    ui.run_until_parked();
    doc.read_with(&session.app, |doc, cx| {
        assert!(matches!(doc.load, Load::Ready));
        assert!(!doc.displayed.has_header);
        assert_eq!(doc.table.as_ref().unwrap().read(cx).data.row_count(), 2);
    });
}

#[test]
fn successive_unicode_sheet_requests_preserve_exact_names() {
    let session = Session::new();
    session.open(vec![fixture("unicode-sheets.xlsx")]);
    let (doc, mut ui) = session.viewer(0);
    let (composed, decomposed) = doc.read_with(&session.app, |doc, _| {
        let index = |sheet| doc.sheets.iter().position(|s| s == sheet).unwrap();
        (index("Caf\u{e9}"), index("Cafe\u{301}"))
    });
    ui.update(|w, cx| {
        w.click(("sheet", composed), cx);
        w.click(("sheet", decomposed), cx);
    });
    ui.run_until_parked();
    doc.read_with(&session.app, |doc, cx| {
        assert_eq!(doc.displayed.sheet.as_deref(), Some("Cafe\u{301}"));
        assert_eq!(doc.table.as_ref().unwrap().read(cx).data.columns.len(), 1);
    });
}

#[test]
fn document_menu_reads_file_info_and_changes_import_options() {
    let session = Session::new();
    session.open(vec![fixture("multi-sheet.xlsx")]);
    let (doc, mut ui) = session.viewer(0);
    ui.dispatch_action(GetInfo);
    session.app.run_until_parked();
    session.app.read(|cx| {
        assert_eq!(cx.windows().len(), 2);
        assert_eq!(cx.global::<Documents>().windows.len(), 1);
    });
    ui.dispatch_action(ImportOptions);
    session.app.run_until_parked();
    session.app.simulate_prompt_answer("Read as Data");
    session.app.run_until_parked();
    doc.read_with(&session.app, |doc, _| assert!(!doc.displayed.has_header));
    ui.dispatch_action(ClearRecent);
    session.app.run_until_parked();
    session
        .app
        .read(|cx| assert!(cx.global::<Documents>().recent.paths().is_empty()));
}

#[test]
fn export_keeps_the_original_worksheet_when_the_save_panel_outlives_a_switch() {
    let session = Session::new();
    session.open(vec![fixture("multi-sheet.xlsx")]);
    let (doc, mut ui) = session.viewer(0);
    ui.dispatch_action(crate::actions::ExportTable);
    for id in ["export-format", "CSV", "choose-export-path"] {
        ui.run_until_parked();
        ui.update(|w, cx| w.click(id, cx));
    }
    ui.run_until_parked();
    assert!(session.app.did_prompt_for_new_path());
    let last = doc.read_with(&session.app, |doc, _| {
        doc.sheets.iter().position(|s| s == "Last").unwrap()
    });
    ui.update(|w, cx| w.click(("sheet", last), cx));
    ui.run_until_parked();
    doc.read_with(&session.app, |doc, _| {
        assert_eq!(doc.displayed.sheet.as_deref(), Some("Last"))
    });
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("first.csv");
    session
        .app
        .simulate_new_path_selection(|_| Some(dest.clone()));
    ui.run_until_parked();
    assert_eq!(
        fs::read_to_string(dest)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["column_0", "0.0"]
    );
}

#[test]
fn view_and_filter_menus_operate_on_the_focused_document() {
    use gpui_kit::{App, OwnedMenuItem};

    let session = Session::new();
    session.open(vec![
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/workflow.csv"),
    ]);
    let (doc, mut ui) = session.viewer(0);
    let item = |cx: &App, menu: &str, title: &str| {
        let menu = cx
            .get_menus()
            .unwrap()
            .into_iter()
            .find(|m| m.name == menu)
            .unwrap();
        menu.items
            .into_iter()
            .find_map(|item| match item {
                OwnedMenuItem::Action {
                    name,
                    action,
                    checked,
                    ..
                } if name == title => Some((action, checked)),
                _ => None,
            })
            .unwrap()
    };
    let rows = || {
        doc.read_with(&session.app, |doc, cx| {
            doc.table.as_ref().unwrap().read(cx).data.row_count()
        })
    };

    ui.update(|w, cx| {
        w.render_frame(cx);
        let (chart, _) = item(cx, "View", "Show Chart Inspector");
        w.dispatch_action(chart, cx);
    });
    ui.run_until_parked();
    ui.update(|w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find("Chart").selected(), Some(true));
        w.press("home", cx);
        w.press("right", cx);
        w.render_frame(cx);
        w.press("cmd-alt-i", cx);
    });
    ui.run_until_parked();
    ui.update(|w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find("Column").selected(), Some(true));
        assert_eq!(w.find("Inspect column").value(), Some("score"));
        assert!(item(cx, "View", "Show Row Numbers").1);
        w.press("cmd-alt-r", cx);
    });
    ui.run_until_parked();
    ui.update(|w, cx| {
        assert!(!item(cx, "View", "Show Row Numbers").1);
        w.press("cmd-alt-f", cx);
    });
    ui.run_until_parked();
    ui.update(|w, cx| {
        w.render_frame(cx);
        assert!(w.find("apply-filter").visible());
        w.press("escape", cx);
        w.render_frame(cx);
        w.press("left", cx);
        w.render_frame(cx);
        let (filter, _) = item(cx, "Filter", "Filter by Selected Value");
        w.dispatch_action(filter, cx);
    });
    ui.run_until_parked();
    assert_eq!(rows(), 3);
    ui.update(|w, cx| {
        w.render_frame(cx);
        let (clear, _) = item(cx, "Filter", "Clear All Filters");
        w.dispatch_action(clear, cx);
    });
    ui.run_until_parked();
    assert_eq!(rows(), 5);
}

#[test]
fn settings_menu_updates_the_open_documents_display_preferences() {
    use crate::preferences::Preferences;
    let session = Session::new();
    session.open(vec![fixture("multi-sheet.xlsx")]);
    let (_, mut document) = session.viewer(0);
    document.dispatch_action(crate::shell::Preferences);
    session.app.run_until_parked();
    let (handle, before) = session.app.read(|cx| {
        let viewer = cx.global::<Documents>().windows[0].1;
        let handle = cx.windows().into_iter().find(|w| *w != viewer).unwrap();
        (handle, Preferences::get(cx))
    });
    let mut settings = VisualTestContext::from_window(handle, &session.app);
    settings.update(|w, cx| {
        w.render_frame(cx);
        let bottom = w.find(("pref", 4usize)).bounds().bottom();
        assert!(bottom <= w.viewport_size().height);
        assert!(w.viewport_size().height - bottom < px(60.));
        w.click(("row-height", 32usize), cx);
        w.click(("pref", 0usize), cx);
    });
    session.app.run_until_parked();
    session.app.read(|cx| {
        let after = Preferences::get(cx);
        assert_eq!(after.row_height, 32.);
        assert_ne!(after.show_row_numbers, before.show_row_numbers);
    });
}
