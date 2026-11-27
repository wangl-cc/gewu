use std::sync::Arc;

use arrow::array::{ArrayRef, Int32Array, ListBuilder, RecordBatch, StringArray, StringBuilder};

use super::*;
use crate::{ColumnSort, Dataset};

#[test]
fn dictionary_empty_text_and_logical_nulls_stay_distinct() {
    use arrow::{
        array::{DictionaryArray, Int8Array},
        datatypes::Int8Type,
    };

    use crate::Filter;

    let values = Arc::new(StringArray::from(vec![Some(""), None, Some("x")]));
    let keys = Int8Array::from(vec![Some(2), None, Some(0), Some(1)]);
    let dictionary = DictionaryArray::<Int8Type>::try_new(keys, values).unwrap();
    let batch = RecordBatch::try_from_iter([("value", Arc::new(dictionary) as ArrayRef)]).unwrap();
    let dataset = Dataset::from_test_batch(batch);
    let view = dataset.unfiltered().unsorted();
    let id = ColumnId::from_raw(0);
    let segment = view.format_column(id, 0, 4).unwrap();
    for (row, expected) in [Some("x"), None, Some(""), None].into_iter().enumerate() {
        assert_eq!(segment.cell(row as u32), expected);
        let expected_filter = expected.map_or(Filter::OnlyNulls, |value| {
            Filter::exact(vec![value.to_owned()], false, false)
        });
        assert_eq!(
            view.exact_filter_for_cell(id, row as u64)
                .unwrap()
                .unwrap()
                .filter,
            expected_filter
        );
    }
}

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
    let dataset = Dataset::from_test_batch(batch);
    let text = ColumnId::from_raw(0);
    let number = ColumnId::from_raw(1);
    let in_file_order = dataset.unfiltered().unsorted();
    assert_eq!(in_file_order.source_rows_in(1..3).collect::<Vec<_>>(), [
        SourceRow(1),
        SourceRow(2)
    ]);
    assert_eq!(in_file_order.source_rows_in(u64::MAX..u64::MAX).len(), 0);
    let ordered = dataset
        .unfiltered()
        .sort(&[ColumnSort {
            column: number,
            descending: true,
        }])
        .unwrap();
    assert_eq!(ordered.source_rows(1, 64), [8, 7, 6, 5, 4, 3, 2, 1, 0]);
    assert_eq!(ordered.source_rows_in(8..u64::MAX).collect::<Vec<_>>(), [
        SourceRow(1),
        SourceRow(0)
    ]);
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
    let dataset = Dataset::from_test_batch(batch);
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
