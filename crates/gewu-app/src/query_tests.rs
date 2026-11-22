use std::io::Write;

use gewu_core::{ColumnId, ColumnSort, Dataset};

use super::*;

fn setup() -> (QuerySession, ColumnId) {
    let mut file = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    write!(file, "score\n6\n2\n9\n").unwrap();
    let dataset = Arc::new(Dataset::open(file.path()).unwrap());
    let score = dataset.column_id("score").unwrap();
    (QuerySession::new(AppliedView::new(dataset)), score)
}

fn sort(column: ColumnId, descending: bool) -> impl FnOnce(&mut TableQuery) {
    move |query| query.sorts = vec![ColumnSort { column, descending }]
}

#[test]
fn obsolete_failure_runs_only_the_latest_edit() {
    let (mut session, score) = setup();
    let original = session.displayed().view().clone();
    let invalid = session.edit(sort(ColumnId::from_raw(99), false)).unwrap();
    assert!(session.edit(sort(score, false)).is_none());
    assert!(session.edit(sort(score, true)).is_none());
    let QueryCompletion::Run(latest) = session.finish(invalid.run()) else {
        panic!("the latest sort must run after the obsolete failure");
    };
    assert!(Arc::ptr_eq(session.displayed().view(), &original));
    assert!(session.is_running());
    assert!(session.error().is_none());
    assert!(matches!(
        session.finish(latest.run()),
        QueryCompletion::Installed
    ));
    assert_eq!(session.displayed().view().source_rows(0, 3), [2, 0, 1]);
    assert!(!session.is_running());
}

#[test]
fn reverting_an_active_sort_discards_its_result_without_another_job() {
    let (mut session, score) = setup();
    let original = session.displayed().view().clone();
    let first = session.edit(sort(score, false)).unwrap();
    assert!(session.edit(|q| q.sorts.clear()).is_none());
    assert!(matches!(
        session.finish(first.run()),
        QueryCompletion::Unchanged
    ));
    assert!(Arc::ptr_eq(session.displayed().view(), &original));
    assert!(session.requested().sorts.is_empty());
    assert!(!session.is_running());
}

#[test]
fn a_failed_edit_restores_the_last_successful_query_and_allows_retry() {
    let (mut session, score) = setup();
    let first = session.edit(sort(score, false)).unwrap();
    assert!(matches!(
        session.finish(first.run()),
        QueryCompletion::Installed
    ));
    let original = session.displayed().view().clone();
    let invalid = session.edit(sort(ColumnId::from_raw(99), false)).unwrap();
    assert!(matches!(
        session.finish(invalid.run()),
        QueryCompletion::Unchanged
    ));
    assert!(session.error().is_some());
    assert!(Arc::ptr_eq(session.displayed().view(), &original));
    assert!(session.requested() == session.displayed().query().as_ref());
    let retry = session.edit(sort(score, true)).unwrap();
    assert!(session.error().is_none());
    assert!(matches!(
        session.finish(retry.run()),
        QueryCompletion::Installed
    ));
    assert_eq!(session.displayed().view().source_rows(0, 3), [2, 0, 1]);
}
