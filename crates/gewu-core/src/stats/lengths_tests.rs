use std::sync::Arc;

use arrow::{
    array::{ArrayRef, ListArray, StringArray},
    datatypes::Int32Type,
};

use super::*;

#[test]
fn lengths_measure_native_values() {
    let item =
        |data_type| std::sync::Arc::new(arrow::datatypes::Field::new("item", data_type, true));
    let dictionary = |value| DataType::Dictionary(Box::new(DataType::Int32), Box::new(value));
    assert_eq!(unit(&DataType::Utf8View), Some(LengthUnit::Characters));
    assert_eq!(unit(&DataType::Binary), Some(LengthUnit::Bytes));
    assert_eq!(
        unit(&DataType::List(item(DataType::Int32))),
        Some(LengthUnit::Elements)
    );
    assert_eq!(unit(&DataType::Int32), None);
    assert_eq!(unit(&dictionary(DataType::Utf8)), None);
}

#[test]
fn unicode_lengths_distinguish_empty_and_null() {
    let array: ArrayRef = Arc::new(StringArray::from(vec![
        Some("é🙂"),
        Some("e\u{301}"),
        Some(""),
        None,
    ]));
    let all = crate::col_view::test::Fixture::all(&array);
    let result = analyze(all.view(), NonZeroU32::new(40).unwrap()).unwrap();
    assert_eq!(result.unit, LengthUnit::Characters);
    assert_eq!(
        (result.min, result.max, result.empty_count),
        (Some(0), Some(2), 1)
    );
    assert_eq!(result.mean, Some(4.0 / 3.0));
    assert_eq!(result.histogram.counts, [1, 0, 2]);
    let grouped = analyze(all.view(), NonZeroU32::new(2).unwrap()).unwrap();
    assert_eq!(grouped.histogram.counts, [1, 2]);
    assert_eq!(grouped.bin_range(0), Some(0..=1));
    assert_eq!(grouped.bin_range(1), Some(2..=2));
    assert_eq!(grouped.bin_range(2), None);
    assert_eq!(crate::stats::count_nulls(all.view()), 1);
    for rows in [vec![], vec![3]] {
        let selected = crate::col_view::test::Fixture::new(&array, rows.clone());
        let result = analyze(selected.view(), NonZeroU32::new(40).unwrap()).unwrap();
        assert_eq!((result.min, result.max, result.mean), (None, None, None));
        assert!(result.histogram.counts.is_empty());
        assert_eq!(
            crate::stats::count_nulls(selected.view()),
            rows.len() as u64
        );
    }
    let selected = crate::col_view::test::Fixture::new(&array, vec![2]);
    let result = analyze(selected.view(), NonZeroU32::new(40).unwrap()).unwrap();
    assert_eq!(result.histogram.counts, [1]);
    assert_eq!(
        (result.histogram.lo, result.histogram.hi),
        (Some(0.0), Some(1.0))
    );
}

#[test]
fn list_lengths_respect_sliced_offsets_and_selected_rows() {
    let array = ListArray::from_iter_primitive::<Int32Type, _, _>([
        Some(vec![Some(1), Some(2), Some(3)]),
        Some(vec![]),
        Some(vec![None, Some(4)]),
        None,
        Some(vec![Some(5)]),
    ]);
    let array: ArrayRef = Arc::new(array.slice(1, 4));
    let selected = crate::col_view::test::Fixture::new(&array, vec![0, 1, 2]);
    let result = analyze(selected.view(), NonZeroU32::new(40).unwrap()).unwrap();
    assert_eq!(result.unit, LengthUnit::Elements);
    assert_eq!(
        (result.min, result.max, result.mean),
        (Some(0), Some(2), Some(1.0))
    );
    assert_eq!(result.histogram.counts, [1, 0, 1]);
    assert_eq!(
        (
            result.empty_count,
            crate::stats::count_nulls(selected.view()),
            selected.rows().len()
        ),
        (1, 1, 3)
    );
}
