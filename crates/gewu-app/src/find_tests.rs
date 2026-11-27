use std::{io::Write, sync::Arc};

use gewu_core::{CellMatch, ColumnHighlight, ColumnId, Dataset, FindHighlight, ViewRow};

use super::{step, FindSession, FindStatus, Highlights};

/// An open session over `name` rows of a single text column.
fn session(values: &[&str]) -> (FindSession, ColumnId) {
    let mut file = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    writeln!(file, "name").unwrap();
    for value in values {
        writeln!(file, "{value}").unwrap();
    }
    let source = Arc::new(Dataset::open(file.path()).unwrap());
    let column = source.column_id("name").unwrap();
    let mut session = FindSession::new(source.unfiltered().unsorted(), vec![column]);
    session.open();
    (session, column)
}

/// Type `text` and run the step it starts.
fn type_text(find: &mut FindSession, text: &str) -> super::StepResult {
    find.set_text(text).step.unwrap().run()
}

fn at(row: u64, column: ColumnId) -> CellMatch {
    CellMatch {
        display_row: ViewRow(row),
        column,
    }
}

#[test]
fn navigation_crosses_page_boundaries_wraps_and_cancels_opposite_steps() {
    let mut file = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    writeln!(file, "name").unwrap();
    for _ in 0..300 {
        writeln!(file, "hit").unwrap();
    }
    let source = Arc::new(Dataset::open(file.path()).unwrap());
    let col = source.column_id("name").unwrap();
    let view = source.unfiltered().unsorted();
    let find = view.find("hit", &[col]).unwrap();
    let (last, wrapped) = step(&find, None, 300).unwrap();
    assert_eq!(last.unwrap().display_row.0, 299);
    assert!(!wrapped);
    let (first, wrapped) = step(&find, last, 601).unwrap();
    assert_eq!(first.unwrap().display_row.0, 0);
    assert!(wrapped);
    let (back, wrapped) = step(&find, first, -301).unwrap();
    assert_eq!(back, last);
    assert!(wrapped);
    assert_eq!(step(&find, back, 0).unwrap(), (back, false));
    assert_eq!(
        step(&view.find("absent", &[col]).unwrap(), None, i64::MIN).unwrap(),
        (None, false)
    );
}

#[test]
fn steps_requested_while_one_runs_add_up_and_run_next() {
    let (mut find, column) = session(&["hit", "miss", "hit", "hit"]);
    let typed = find.set_text("hit").step.unwrap();
    assert!(typed.debounce);
    assert!(find.step(1).step.is_none());
    assert!(find.step(1).step.is_none());
    assert!(matches!(find.status(), FindStatus::Searching {
        from: None
    }));
    let work = find.step_finished(typed.run());
    assert_eq!(work.reveal, Some(at(0, column)));
    assert!(work.count.is_some());
    let queued = work.step.unwrap();
    assert!(!queued.debounce);
    let work = find.step_finished(queued.run());
    assert_eq!(work.reveal, Some(at(3, column)));
    assert!(work.step.is_none());
    assert_eq!(find.current(), Some(at(3, column)));
}

#[test]
fn results_of_a_replaced_search_are_ignored() {
    let (mut find, column) = session(&["alpha", "beta"]);
    let stale = find.set_text("alpha").step.unwrap();
    let current = find.set_text("beta").step.unwrap();
    let work = find.step_finished(stale.run());
    assert!(work.reveal.is_none() && work.step.is_none());
    assert!(matches!(find.status(), FindStatus::Searching { .. }));
    assert_eq!(
        find.step_finished(current.run()).reveal,
        Some(at(1, column))
    );
}

#[test]
fn the_count_follows_navigation_from_its_starting_match_and_wraps() {
    let (mut find, column) = session(&["hit", "hit", "hit"]);
    let typed = type_text(&mut find, "hit");
    let work = find.step_finished(typed);
    let count = work.count.unwrap();
    assert_eq!(find.count(), Some(None));
    let work = find.step(1);
    find.step_finished(work.step.unwrap().run());
    let work = find.step(1);
    assert_eq!(
        find.step_finished(work.step.unwrap().run()).reveal,
        Some(at(2, column))
    );
    find.count_finished(count.run());
    assert_eq!(find.count(), Some(Some((3, 3))));
    let work = find.step(1);
    find.step_finished(work.step.unwrap().run());
    assert!(matches!(find.status(), FindStatus::At {
        wrapped: true,
        ..
    }));
    assert_eq!(find.count(), Some(Some((1, 3))));
}

#[test]
fn closing_or_an_empty_scope_starts_no_work() {
    let (mut find, _) = session(&["hit"]);
    find.close();
    let work = find.set_text("hit");
    assert!(work.step.is_none() && work.highlight.is_none());
    find.open();
    assert!(find.set_columns(Vec::new(), false).step.is_none());
    assert!(!find.can_navigate());
}

#[test]
fn reordering_keeps_the_current_match_and_recounts_it() {
    let (mut find, column) = session(&["hit", "hit"]);
    let typed = type_text(&mut find, "hit");
    let work = find.step_finished(typed);
    assert_eq!(work.reveal, Some(at(0, column)));
    let work = find.set_columns(vec![column], true);
    assert!(work.step.is_none() && work.count.is_some());
    assert_eq!(find.current(), Some(at(0, column)));
    let work = find.set_columns(vec![column], false);
    assert!(work.step.is_some());
    assert_eq!(find.current(), None);
}

#[test]
fn cell_spans_mark_preview_hits_and_the_ellipsis_of_hidden_ones() {
    let (mut find, column) = session(&["a needle", "prefix", "plain"]);
    let mut spans = Vec::new();
    find.cell_spans(
        column,
        ViewRow(0),
        true,
        || Some(("a needle", "a needle")),
        8,
        &mut spans,
    );
    assert!(spans.is_empty(), "no search yet");
    let work = find.set_text("needle");
    find.step_finished(work.step.unwrap().run());
    find.cell_spans(
        column,
        ViewRow(0),
        true,
        || Some(("a needle", "a needle")),
        8,
        &mut spans,
    );
    assert_eq!(spans, vec![2..8]);
    // The current match proves a hit past the shown prefix; its ellipsis is marked.
    spans.clear();
    let hidden = "prefix\nneedle";
    find.cell_spans(
        column,
        ViewRow(0),
        true,
        || Some((hidden, "prefix")),
        9,
        &mut spans,
    );
    assert_eq!(spans, vec![6..9]);
    // A typed column can match a differently formatted value: the whole cell is marked.
    spans.clear();
    find.cell_spans(
        column,
        ViewRow(0),
        false,
        || Some(("1.50", "1.50")),
        4,
        &mut spans,
    );
    assert_eq!(spans, vec![0..4]);
    // An unclassified typed cell waits for the engine.
    spans.clear();
    find.cell_spans(column, ViewRow(2), false, || unreachable!(), 5, &mut spans);
    assert!(spans.is_empty());
}

#[test]
fn batched_highlight_uses_relative_rows_across_cache_blocks() {
    let column = ColumnId::from_raw(3);
    let mut highlights = Highlights::default();
    // A 67-row result starting at display row 64, with matches straddling
    // the cache boundary and a final match in the partial block.
    highlights.store(FindHighlight {
        start: 64,
        n_rows: 67,
        columns: vec![ColumnHighlight {
            column,
            matched: vec![0, 0, 0, 0, 0, 0, 0, 0b1000_0000, 0b0000_0101],
        }],
    });
    let hits: Vec<_> = (0..192)
        .filter(|&row| highlights.cached_match(column, row) == Some(true))
        .collect();
    assert_eq!(hits, [127, 128, 130]);
}

#[test]
fn highlights_read_uncached_visible_blocks_in_one_batch() {
    let values = vec!["hit"; 200];
    let (mut find, column) = session(&values);
    let typed = type_text(&mut find, "hit");
    find.step_finished(typed);
    let job = find.set_viewport(0..130, vec![column]).highlight.unwrap();
    assert_eq!((job.start, job.rows), (0, 192));
    assert!(find.set_viewport(0..130, vec![column]).highlight.is_none());
    assert!(find.highlight_finished(job.run()).highlight.is_none());
    assert_eq!(find.cached_match(column, 129), Some(true));
    assert!(find
        .set_viewport(150..190, vec![column])
        .highlight
        .is_none());
    let next = find.set_viewport(150..200, vec![column]).highlight.unwrap();
    assert_eq!((next.start, next.rows), (192, 64));
}
