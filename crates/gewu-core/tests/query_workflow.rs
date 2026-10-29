#![cfg(feature = "parquet")]

use std::sync::Arc;

use arrow::array::{ArrayRef, Int32Array, RecordBatch, StringArray};
use gewu_core::{Document, Error, Filter, ViewState};
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
    let doc = Document::open(file.path()).unwrap();
    let keys = doc.keys_for_labels(1, &[Some("2".into())]).unwrap();
    let state = ViewState {
        filters: vec![Filter::InValues { column: 1, keys }, Filter::Range {
            column: 0,
            min: Some(doc.parse_bound(0, "65530.5").unwrap()),
            max: Some(doc.parse_bound(0, "131080").unwrap()),
        }],
        search: Some("keep".into()),
        ..Default::default()
    };
    let view = doc.apply_view(&state).unwrap();
    let rows = doc.rows(view.version, 0, len as u32, &[0, 1, 2]).unwrap();
    let expected = (65531..=131080)
        .filter(|i| i % 3 == 2 && i % 2 == 0)
        .map(|i| i as u64)
        .collect::<Vec<_>>();
    assert_eq!(rows.source_rows, expected);
    assert!(rows
        .matched
        .as_chunks::<3>()
        .0
        .iter()
        .all(|cells| *cells == [false, false, true]));

    let invalid = ViewState {
        filters: vec![Filter::Range {
            column: 0,
            min: Some(doc.parse_bound(0, "20").unwrap()),
            max: Some(doc.parse_bound(0, "10").unwrap()),
        }],
        ..Default::default()
    };
    assert!(matches!(
        doc.apply_view(&invalid),
        Err(Error::InvalidArgument(_))
    ));
    assert_eq!(
        doc.rows(view.version, 0, 1, &[0]).unwrap().source_rows,
        expected[..1]
    );
}
