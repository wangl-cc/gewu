#![cfg(feature = "parquet")]

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use gewu_core::{Filter, QuerySpec, Session};
use parquet::arrow::ArrowWriter;

fn document() -> Session {
    let score: ArrayRef = Arc::new(Float64Array::from(vec![
        None,
        Some(1.0),
        Some(2.0),
        Some(3.0),
        None,
        Some(f64::NAN),
        Some(2.0),
    ]));
    let text: ArrayRef = Arc::new(StringArray::from(vec![
        None,
        Some("alpha"),
        Some("beta"),
        Some("alphabet"),
        Some("alpha"),
        Some("alphabet"),
        None,
    ]));
    let category: ArrayRef = Arc::new(StringArray::from(vec!["A", "A", "B", "A", "B", "A", "A"]));
    let marker: ArrayRef = Arc::new(StringArray::from(vec![
        "needle", "needle", "needle", "other", "needle", "needle", "other",
    ]));
    let batch = RecordBatch::try_from_iter(vec![
        ("score", score),
        ("text", text),
        ("category", category),
        ("marker", marker),
    ])
    .unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut writer = ArrowWriter::try_new(file.reopen().unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    Session::open(file.path()).unwrap()
}

fn range(include_nulls: bool) -> Filter {
    Filter::range(Some("1".into()), Some("2".into()), include_nulls)
}

fn contains(include_nulls: bool) -> Filter {
    Filter::contains("ALPHA".into(), false, include_nulls)
}

fn rows(doc: &Session, spec: QuerySpec) -> Vec<u64> {
    let view = doc.apply_view(&spec).unwrap();
    doc.rows(view.version, 0, 20, &[0, 1]).unwrap().source_rows
}

#[test]
fn range_null_policy_keeps_nulls_without_matching_nan_or_outside_values() {
    let doc = document();
    for (filter, expected) in [
        (range(false), vec![1, 2, 6]),
        (range(true), vec![0, 1, 2, 4, 6]),
        (Filter::exact(vec![], false, true), vec![0, 4]),
        (Filter::exact(vec![], true, false), vec![1, 2, 3, 5, 6]),
    ] {
        assert_eq!(rows(&doc, QuerySpec::single(0, filter)), expected);
    }
}

#[test]
fn contains_null_policy_preserves_case_matching_and_only_nulls() {
    let doc = document();
    for (filter, expected) in [
        (contains(false), vec![1, 3, 4, 5]),
        (contains(true), vec![0, 1, 3, 4, 5, 6]),
        (Filter::contains("ALPHA".into(), true, true), vec![0, 6]),
        (Filter::exact(vec![], false, true), vec![0, 6]),
    ] {
        assert_eq!(rows(&doc, QuerySpec::single(1, filter)), expected);
    }
}

#[test]
fn included_nulls_still_obey_other_column_filters() {
    let doc = document();
    for (filter, expected) in [(range(true), vec![0, 1]), (contains(true), vec![0, 1, 5])] {
        let col = if matches!(filter.condition, gewu_core::FilterCondition::Range { .. }) {
            0
        } else {
            1
        };
        assert_eq!(
            rows(
                &doc,
                QuerySpec::single(col, filter)
                    .with_filter(2, Filter::exact(vec!["A".into()], false, false))
                    .with_filter(3, Filter::contains("needle".into(), false, false),),
            ),
            expected
        );
    }
}

#[test]
fn exact_values_combine_with_an_independent_null_policy() {
    let doc = document();
    for (values, include_nulls, expected) in [
        (vec![], true, vec![0, 6]),
        (vec!["alpha".into()], false, vec![1, 4]),
        (vec!["beta".into()], true, vec![0, 2, 6]),
    ] {
        assert_eq!(
            rows(
                &doc,
                QuerySpec::single(1, Filter::exact(values, false, include_nulls))
            ),
            expected
        );
    }
}

#[test]
fn facets_count_included_nulls_and_still_ignore_only_their_own_filter() {
    let doc = document();
    let spec = QuerySpec::single(0, range(true))
        .with_filter(2, Filter::exact(vec!["A".into()], false, false))
        .with_filter(3, Filter::contains("needle".into(), false, false));
    let values = doc
        .prepare_text_choices(2, Some(&spec))
        .unwrap()
        .page("", None, 100)
        .unwrap()
        .values;
    assert_eq!(
        values
            .iter()
            .map(|value| (value.value.as_str(), value.count))
            .collect::<Vec<_>>(),
        [("A", 2), ("B", 2)]
    );

    // Removing only score's own filter leaves source rows 0, 1 and 5: null,
    // one finite value, and NaN. The UI must not conflate those last two classes.
    let choices = doc.filter_choices(0, Some(&spec)).unwrap();
    let values = choices.values.unwrap();
    assert_eq!(
        values.iter().map(|value| value.count).sum::<u64>() + choices.null_count,
        3
    );
    assert_eq!(choices.null_count, 1);
    assert_eq!(
        values
            .iter()
            .find(|value| value.label == "NaN")
            .unwrap()
            .count,
        1
    );
}
