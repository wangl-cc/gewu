use super::{Import, ImportSession, Load, ReadOutcome};

fn sheet(name: &str) -> Import {
    Import {
        sheet: Some(name.into()),
        has_header: true,
    }
}

fn failure() -> gewu_core::Error {
    gewu_core::Error::Cancelled
}

#[test]
fn one_reader_runs_and_reads_the_latest_request_when_it_starts() {
    let mut imports = ImportSession::new(sheet("first"));
    assert!(!imports.request(sheet("second")));
    assert!(!imports.request(sheet("third")));
    assert_eq!(imports.begin_read(), Some(sheet("third")));
    assert!(!imports.request(sheet("fourth")));
    // The finished read was superseded: a new reader picks up the latest request.
    assert!(matches!(
        imports.read_finished(sheet("third"), Ok(())),
        ReadOutcome::Next
    ));
    assert_eq!(imports.begin_read(), Some(sheet("fourth")));
    assert!(matches!(
        imports.read_finished(sheet("fourth"), Ok(7)),
        ReadOutcome::Install(7)
    ));
    imports.installed(sheet("fourth"));
    assert!(matches!(imports.load(), Load::Ready));
    assert!(imports.request(sheet("fifth")));
}

#[test]
fn requesting_the_shown_import_withdraws_a_pending_read() {
    let mut imports = ImportSession::new(sheet("first"));
    imports.begin_read();
    imports.read_finished(sheet("first"), Ok(()));
    imports.installed(sheet("first"));
    assert!(imports.request(sheet("second")));
    assert!(!imports.request(sheet("first")));
    assert!(matches!(imports.load(), Load::Ready));
    assert_eq!(imports.begin_read(), None);
    assert!(imports.request(sheet("third")));
}

#[test]
fn a_failed_request_keeps_the_shown_table_selected_but_not_before_one_is_shown() {
    let mut imports = ImportSession::new(sheet("bad"));
    imports.begin_read();
    assert!(matches!(
        imports.read_finished(sheet("bad"), Err::<(), _>(failure())),
        ReadOutcome::Failed
    ));
    assert_eq!(imports.selection(), &sheet("bad"));
    assert!(imports.request(sheet("good")));
    imports.begin_read();
    imports.read_finished(sheet("good"), Ok(()));
    imports.installed(sheet("good"));
    assert!(imports.request(sheet("bad")));
    imports.begin_read();
    imports.read_finished(sheet("bad"), Err::<(), _>(failure()));
    assert!(matches!(imports.load(), Load::Failed(..)));
    assert_eq!(imports.selection(), &sheet("good"));
}
