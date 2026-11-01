#![cfg(feature = "parquet")]

use std::sync::Arc;

use arrow::array::{ArrayRef, Int32Array, RecordBatch, StringArray};
use gewu_core::{Error, Filter, QuerySpec, Session};
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
    let doc = Session::open(file.path()).unwrap();
    let keys = doc.keys_for_labels(1, &[Some("2".into())]).unwrap();
    let spec = QuerySpec::default()
        .with_filter(1, Filter::InValues { keys })
        .with_filter(0, Filter::Range {
            include_nulls: false,
            min: Some(doc.parse_bound(0, "65530.5").unwrap()),
            max: Some(doc.parse_bound(0, "131080").unwrap()),
        })
        .with_filter(2, Filter::Contains {
            include_nulls: false,
            text: "keep".into(),
            case_sensitive: false,
        });
    let view = doc.apply_view(&spec).unwrap();
    let rows = doc.rows(view.version, 0, len as u32, &[0, 1, 2]).unwrap();
    let expected = (65531..=131080)
        .filter(|i| i % 3 == 2 && i % 2 == 0)
        .map(|i| i as u64)
        .collect::<Vec<_>>();
    assert_eq!(rows.source_rows, expected);

    let invalid = QuerySpec::single(0, Filter::Range {
        include_nulls: false,
        min: Some(doc.parse_bound(0, "20").unwrap()),
        max: Some(doc.parse_bound(0, "10").unwrap()),
    });
    assert!(matches!(
        doc.apply_view(&invalid),
        Err(Error::InvalidArgument(_))
    ));
    assert_eq!(
        doc.rows(view.version, 0, 1, &[0]).unwrap().source_rows,
        expected[..1]
    );
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
    let doc = Session::from_dataset(dataset.clone());
    assert_eq!(doc.n_rows(), len as usize);

    // Pass filters in reverse cost order: expensive text search first, cheap range second, null
    // check third
    let expensive_first = gewu_core::QuerySpec::default()
        .with_filter(1, Filter::Contains {
            text: "match_target".into(),
            case_sensitive: true,
            include_nulls: false,
        })
        .with_filter(0, Filter::Range {
            min: Some(doc.parse_bound(0, "100").unwrap()),
            max: Some(doc.parse_bound(0, "200").unwrap()),
            include_nulls: false,
        })
        .with_filter(0, Filter::IsNull { negate: true });

    let view = doc.apply_view(&expensive_first).unwrap();
    let expected: Vec<u64> = (100..=200)
        .filter(|i| i % 5 == 0)
        .map(|i| i as u64)
        .collect();
    let rows = doc.rows(view.version, 0, 100, &[0]).unwrap();
    assert_eq!(rows.source_rows, expected);
}
