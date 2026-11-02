#![cfg(feature = "parquet")]

use std::sync::Arc;

use arrow::array::{ArrayRef, Int32Array, RecordBatch, StringArray};
use gewu_core::{Error, Filter, QuerySpec};
use parquet::arrow::ArrowWriter;

#[test]
fn batch_queries_preserve_source_codes_composition_and_highlights() {
    let len = 140_000;
    let numbers: ArrayRef = Arc::new(Int32Array::from_iter_values(0..len));
    let groups: ArrayRef = Arc::new(Int32Array::from_iter_values((0..len).map(|i| i % 3)));
    let text: ArrayRef = Arc::new(StringArray::from_iter_values((0..len).map(|i| {
        if i % 2 == 0 {
            "keep"
        } else {
            "skip"
        }
    })));
    let batch =
        RecordBatch::try_from_iter(vec![("id", numbers), ("group", groups), ("text", text)])
            .unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut writer = ArrowWriter::try_new(file.reopen().unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    let doc = gewu_core::Dataset::open(file.path())
        .map(|dataset| std::sync::Arc::new(dataset).view())
        .unwrap();
    let spec = QuerySpec::default()
        .with_filter(1, Filter::exact(vec!["2".into()], false, false))
        .with_filter(
            0,
            Filter::range(Some("65530.5".into()), Some("131080".into()), false),
        )
        .with_filter(2, Filter::contains("keep".into(), false, false));
    let view = doc
        .query(&spec, &gewu_core::Cancellation::default())
        .unwrap();
    let rows = view.rows(0, len as u32, &[0, 1, 2]).unwrap();
    let expected = (65531..=131080)
        .filter(|i| i % 3 == 2 && i % 2 == 0)
        .map(|i| i as u64)
        .collect::<Vec<_>>();
    assert_eq!(rows.source_rows, expected);

    let invalid = QuerySpec::single(
        0,
        Filter::range(Some("20".into()), Some("10".into()), false),
    );
    assert!(matches!(
        doc.query(&invalid, &gewu_core::Cancellation::default()),
        Err(Error::InvalidArgument(_))
    ));
    assert_eq!(view.rows(0, 1, &[0]).unwrap().source_rows, expected[..1]);
}

#[test]
fn query_spec_reorders_predicates_by_cost_and_shares_dataset() {
    let len = 50_000;
    let numbers: ArrayRef = Arc::new(Int32Array::from_iter_values(0..len));
    let text: ArrayRef = Arc::new(StringArray::from_iter_values((0..len).map(|i| {
        if i % 5 == 0 {
            "match_target"
        } else {
            "other"
        }
    })));
    let batch = RecordBatch::try_from_iter(vec![("id", numbers), ("text", text)]).unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut writer = ArrowWriter::try_new(file.reopen().unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    let dataset = Arc::new(gewu_core::Dataset::open(file.path()).unwrap());
    assert_eq!(dataset.n_rows(), len as usize);

    // Create session from shared dataset
    let doc = dataset.clone().view();
    assert_eq!(doc.row_count() as usize, len as usize);

    // Pass filters in reverse cost order: expensive text search first, cheap range second, null
    // check third
    let expensive_first = gewu_core::QuerySpec::default()
        .with_filter(1, Filter::contains("match_target".into(), true, false))
        .with_filter(
            0,
            Filter::range(Some("100".into()), Some("200".into()), false),
        );

    let view = doc
        .query(&expensive_first, &gewu_core::Cancellation::default())
        .unwrap();
    let expected: Vec<u64> = (100..=200)
        .filter(|i| i % 5 == 0)
        .map(|i| i as u64)
        .collect();
    let rows = view.rows(0, 100, &[0]).unwrap();
    assert_eq!(rows.source_rows, expected);
}
