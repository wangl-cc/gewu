use std::sync::Arc;

use arrow::array::{ArrayRef, Int32Array, ListBuilder, RecordBatch, StringArray, StringBuilder};

use super::*;
use crate::{column::Column, ColumnSort, Dataset, FileMetadata, FileType};

#[test]
fn segments_keep_byte_offsets_nulls_and_display_order() {
    let strings = StringArray::from(vec![
        Some("é"),
        None,
        Some(""),
        Some("界🦀"),
        None,
        Some("five"),
        None,
        Some(""),
        None,
        Some("nine"),
    ]);
    let numbers = Int32Array::from_iter_values(0..10);
    let batch = RecordBatch::try_from_iter([
        ("text", Arc::new(strings) as ArrayRef),
        ("number", Arc::new(numbers) as ArrayRef),
    ])
    .unwrap();
    let dataset = Arc::new(Dataset {
        source_path: Default::default(),
        schema: batch.schema(),
        columns: vec![
            Column::new("text", batch.column(0).clone()).unwrap(),
            Column::new("number", batch.column(1).clone()).unwrap(),
        ],
        n_rows: batch.num_rows(),
        file_size: 0,
        format: FileType::Csv,
        metadata: FileMetadata::Delimited,
    });
    let text = ColumnId::from_raw(0);
    let number = ColumnId::from_raw(1);
    let ordered = dataset
        .unfiltered()
        .sort(&[ColumnSort {
            column: number,
            descending: true,
        }])
        .unwrap();
    assert_eq!(ordered.source_rows(1, 64), [8, 7, 6, 5, 4, 3, 2, 1, 0]);
    assert_eq!(
        ordered.format_column(number, 1, 1).unwrap().cell(0),
        Some("8")
    );
    let segment = ordered.format_column(text, 1, 64).unwrap();
    assert_eq!(segment.row_count(), 9, "clipped to the available rows");
    assert_eq!(segment.cell(9), None);
    let actual: Vec<_> = (0..segment.row_count())
        .map(|row| segment.cell(row))
        .collect();
    assert_eq!(actual, [
        None,
        Some(""),
        None,
        Some("five"),
        None,
        Some("界🦀"),
        Some(""),
        None,
        Some("é")
    ]);
}

#[test]
fn nested_values_format_as_their_display_text() {
    let mut tags = ListBuilder::new(StringBuilder::new());
    tags.values().append_value("雪");
    tags.values().append_null();
    tags.append(true);
    tags.append(false);
    tags.append(true);
    let tags: ArrayRef = Arc::new(tags.finish());
    let batch = RecordBatch::try_from_iter([("tags", tags)]).unwrap();
    let dataset = Arc::new(Dataset {
        source_path: Default::default(),
        schema: batch.schema(),
        columns: vec![Column::new("tags", batch.column(0).clone()).unwrap()],
        n_rows: batch.num_rows(),
        file_size: 0,
        format: FileType::Parquet,
        metadata: FileMetadata::Delimited,
    });
    let segment = dataset
        .unfiltered()
        .unsorted()
        .format_column(ColumnId::from_raw(0), 0, 64)
        .unwrap();
    let actual: Vec<_> = (0..segment.row_count())
        .map(|row| segment.cell(row))
        .collect();
    assert_eq!(actual, [Some("[雪, ]"), None, Some("[]")]);
}
