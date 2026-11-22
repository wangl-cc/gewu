use arrow::{
    array::{
        DictionaryArray, FixedSizeListArray, Int32Array, Int8Array, ListArray, StringArray,
        StructArray,
    },
    buffer::NullBuffer,
    datatypes::{DataType, Field, Int32Type, Int8Type},
};

use super::*;
use crate::{execution::predicate::CHUNK, TextMatch};

fn col(array: ArrayRef) -> Column {
    Column::new("test", array).unwrap()
}

#[test]
fn unsupported_dictionary_sort_is_cached_as_negative_result() {
    let values: ArrayRef = Arc::new(Int32Array::from(vec![1, 2]));
    let field = Arc::new(arrow::datatypes::Field::new(
        "child",
        DataType::Int32,
        false,
    ));
    let column = col(Arc::new(StructArray::from(vec![(field, values)])));
    assert!(column.dictionary().unwrap().is_none());
    assert!(matches!(column.dictionary.get(), Some(None)));
}

fn matches(array: &ArrayRef, query: &str, range: std::ops::Range<usize>) -> Vec<bool> {
    let mut hits = vec![false; range.len()];
    if let Some(test) = predicate::compile_find(array, query, None).unwrap() {
        test.evaluate(range, &mut hits);
    }
    hits
}

#[test]
fn capabilities_distinguish_value_semantics_from_storage_support() {
    let dictionary = |value| DataType::Dictionary(Box::new(DataType::Int32), Box::new(value));
    for (data_type, contains, equals, ordered, logarithmic) in [
        (DataType::Int32, false, true, true, true),
        (DataType::Date32, false, true, true, false),
        (DataType::Utf8, true, true, false, false),
        (DataType::Boolean, false, true, false, false),
        (DataType::Binary, false, false, false, false),
        (dictionary(DataType::Int64), false, true, true, false),
        (dictionary(DataType::Utf8), true, true, false, false),
    ] {
        assert_eq!(
            col(arrow::array::new_null_array(&data_type, 1))
                .info(ColumnId::new(0))
                .capabilities,
            ColumnCapabilities {
                text_match: contains,
                range: ordered,
                exact: equals,
                scatter: ordered && !matches!(data_type, DataType::Dictionary(..)),
                logarithmic,
            },
            "{data_type}"
        );
    }
}

#[test]
fn unsupported_children_are_rejected_at_loading() {
    use arrow::datatypes::{Fields, UnionFields, UnionMode};
    let unsupported = DataType::Union(UnionFields::empty(), UnionMode::Sparse);
    let child = Arc::new(Field::new("child", unsupported.clone(), true));
    for data_type in [
        unsupported.clone(),
        DataType::List(child.clone()),
        DataType::Struct(Fields::from(vec![child.clone()])),
        DataType::Dictionary(Box::new(DataType::Int8), Box::new(unsupported)),
        DataType::RunEndEncoded(Arc::new(Field::new("runs", DataType::Int32, false)), child),
    ] {
        let array = arrow::array::new_empty_array(&data_type);
        assert!(
            matches!(
                Column::new("unsupported", array),
                Err(crate::FileError::UnsupportedColumnType { .. })
            ),
            "{data_type}"
        );
    }
}

#[test]
fn typed_membership_validates_every_input_and_empty_lists_match_nothing() {
    let array: ArrayRef = Arc::new(Int32Array::from(vec![Some(1), None, Some(2)]));
    let column = col(array);
    let test = column
        .compile_filter(&Filter::exact(Vec::new(), false, false))
        .unwrap();
    let mut hits = [true; 3];
    test.evaluate(0..3, &mut hits);
    assert_eq!(hits, [false; 3]);
    assert!(matches!(
        column.compile_filter(&Filter::exact(
            vec!["1".into(), "invalid".into()],
            false,
            false
        )),
        Err(crate::FilterError::Bound(
            crate::BoundError::InvalidValue { .. }
        ))
    ));

    let array: ArrayRef = Arc::new(
        DictionaryArray::<Int8Type>::try_new(
            Int8Array::from(vec![Some(0), None, Some(1), Some(2)]),
            Arc::new(StringArray::from(vec![Some("01"), None, Some("2")])),
        )
        .unwrap(),
    );
    let column = col(array);
    let test = column
        .compile_filter(&Filter::exact(vec!["01".into(), "2".into()], false, false))
        .unwrap();
    let mut hits = [false; 4];
    test.evaluate(0..4, &mut hits);
    assert_eq!(hits, [true, false, false, true]);
}

#[test]
fn text_patterns_are_literal_and_batches_keep_source_offsets() {
    let array: ArrayRef = Arc::new(StringArray::from(vec![
        Some("prefix"),
        Some("A%_\\z"),
        None,
        Some("a%_\\Z trailing"),
    ]));
    assert_eq!(matches(&array, "%_\\z", 1..4), [true, false, true]);
    let column = col(array);
    let test = column
        .compile_filter(&Filter::exact(vec!["A%_\\z".into()], false, false))
        .unwrap();
    let mut hits = vec![false; 3];
    test.evaluate(1..4, &mut hits);
    assert_eq!(hits, [true, false, false]);
}

#[test]
fn contains_null_policy_covers_dictionary_keys_and_values() {
    let array: ArrayRef = Arc::new(
        DictionaryArray::<Int8Type>::try_new(
            Int8Array::from(vec![Some(0), None, Some(1), Some(2), Some(0)]),
            Arc::new(StringArray::from(vec![Some("keep"), None, Some("skip")])),
        )
        .unwrap(),
    );
    let column = col(array);
    for (include_nulls, expected) in [
        (false, [false, false, false, true]),
        (true, [true, true, false, true]),
    ] {
        let test = column
            .compile_filter(&Filter::contains("keep".into(), true, include_nulls))
            .unwrap();
        let mut hits = [false; 4];
        test.evaluate(1..5, &mut hits);
        assert_eq!(hits, expected);
    }
}

#[test]
fn dictionary_text_match_matches_row_by_row_text_match() {
    use arrow::{
        array::{LargeStringArray, StringViewArray},
        datatypes::Int16Type,
    };

    let rows = [
        Some("Élan"),
        Some("élan"),
        None,
        Some(""),
        Some("ÉLAN vital"),
        Some("x"),
        Some("Élan"),
        None,
        Some("abc"),
    ];
    // Extra distinct values exceed the dictionary limit, forcing per-row matching.
    let filler: Vec<String> = (0..300).map(|i| format!("filler-{i}")).collect();
    let build = |extended: bool, kind: usize| -> ArrayRef {
        let values = rows
            .iter()
            .copied()
            .chain(filler.iter().map(|v| Some(v.as_str())).filter(|_| extended));
        match kind {
            0 => Arc::new(values.collect::<StringArray>()),
            1 => Arc::new(values.collect::<LargeStringArray>()),
            2 => Arc::new(values.collect::<StringViewArray>()),
            _ => Arc::new(values.collect::<DictionaryArray<Int16Type>>()),
        }
    };
    for kind in 0..4 {
        let encoded = col(build(false, kind));
        let plain = col(build(true, kind));
        assert!(encoded.dictionary().unwrap().is_some());
        assert!(plain.dictionary().unwrap().is_none());
        for (text, mode) in ["", "élan", "ÉLAN", "a", "zz", "Élan", "abc"]
            .into_iter()
            .flat_map(|text| [(text, TextMatch::Contains), (text, TextMatch::Equals)])
        {
            for case_sensitive in [false, true] {
                for include_nulls in [false, true] {
                    let filter = Filter::text(text.into(), mode, case_sensitive, include_nulls);
                    let hits = |column: &Column| {
                        let mut hits = vec![false; rows.len()];
                        column
                            .compile_filter(&filter)
                            .unwrap()
                            .evaluate(0..rows.len(), &mut hits);
                        hits
                    };
                    assert_eq!(
                        hits(&encoded),
                        hits(&plain),
                        "kind {kind}: {mode:?} {text:?} case_sensitive={case_sensitive} include_nulls={include_nulls}"
                    );
                }
            }
        }
    }
}

#[test]
fn text_equals_matches_whole_values_with_unicode_case_folding() {
    // Kelvin sign and long s fold to ASCII k and s; final sigma folds to sigma.
    let values = [
        Some("A1"),
        Some("A10"),
        Some("a1"),
        Some("k"),
        Some("\u{212A}"),
        Some("ſ"),
        Some("s"),
        Some("ΣΑΣ"),
        Some("σας"),
        Some(""),
        None,
    ];
    let column = col(Arc::new(StringArray::from(values.to_vec())));
    let matching = |text: &str, case_sensitive: bool| -> Vec<usize> {
        let mut hits = vec![false; values.len()];
        column
            .compile_filter(&Filter::text(
                text.into(),
                TextMatch::Equals,
                case_sensitive,
                false,
            ))
            .unwrap()
            .evaluate(0..values.len(), &mut hits);
        (0..values.len()).filter(|&row| hits[row]).collect()
    };
    // Whole-value matching: A1 never matches A10.
    assert_eq!(matching("A1", true), [0]);
    assert_eq!(matching("A1", false), [0, 2]);
    // ASCII input still matches non-ASCII values that fold to it, and back.
    assert_eq!(matching("K", false), [3, 4]);
    assert_eq!(matching("\u{212A}", false), [3, 4]);
    assert_eq!(matching("S", false), [5, 6]);
    assert_eq!(matching("σας", false), [7, 8]);
    assert_eq!(matching("σας", true), [8]);
    // The empty string is a value distinct from null.
    assert_eq!(matching("", false), [9]);
    // Every case-insensitive equality is also a case-insensitive Contains match.
    for text in ["A1", "K", "S", "σας", ""] {
        let mut contains = vec![false; values.len()];
        column
            .compile_filter(&Filter::contains(text.into(), false, false))
            .unwrap()
            .evaluate(0..values.len(), &mut contains);
        assert!(
            matching(text, false).iter().all(|&row| contains[row]),
            "{text:?}"
        );
    }
}

#[test]
fn non_text_text_match_is_rejected_even_for_empty_text_and_null_arrays() {
    use arrow::datatypes::{Field, Fields, IntervalUnit};

    let item = Arc::new(Field::new("item", DataType::Utf8, true));
    let entries = Arc::new(Field::new(
        "entries",
        DataType::Struct(Fields::from(vec![
            Field::new("key", DataType::Utf8, false),
            Field::new("value", DataType::Utf8, true),
        ])),
        false,
    ));
    for data_type in [
        DataType::Null,
        DataType::Boolean,
        DataType::Int32,
        DataType::Date32,
        DataType::Binary,
        DataType::LargeBinary,
        DataType::BinaryView,
        DataType::FixedSizeBinary(2),
        DataType::Interval(IntervalUnit::MonthDayNano),
        DataType::List(item.clone()),
        DataType::LargeList(item.clone()),
        DataType::ListView(item.clone()),
        DataType::LargeListView(item.clone()),
        DataType::FixedSizeList(item.clone(), 2),
        DataType::Struct(Fields::from(vec![item])),
        DataType::Map(entries, false),
    ] {
        for data_type in [
            data_type.clone(),
            DataType::Dictionary(Box::new(DataType::Int8), Box::new(data_type)),
        ] {
            let column = col(arrow::array::new_null_array(&data_type, 2));
            assert!(
                !column.info(ColumnId::new(0)).capabilities.text_match,
                "{data_type}"
            );
            for text in ["", "needle"] {
                for mode in [TextMatch::Contains, TextMatch::Equals] {
                    assert!(
                        matches!(
                            column.compile_filter(&Filter::text(text.into(), mode, false, true)),
                            Err(crate::FilterError::UnsupportedType { .. })
                        ),
                        "{data_type}: {mode:?} {text:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn nested_null_filters_distinguish_parent_nulls_from_empty_and_null_children() {
    use arrow::{buffer::NullBuffer, datatypes::Field};

    let list: ArrayRef = Arc::new(ListArray::from_iter_primitive::<Int32Type, _, _>([
        Some(vec![Some(12)]),
        None,
        Some(vec![]),
        Some(vec![None]),
    ]));
    let structure: ArrayRef = Arc::new(StructArray::new(
        vec![Field::new("value", DataType::Int32, true)].into(),
        vec![Arc::new(Int32Array::from(vec![
            Some(12),
            None,
            None,
            Some(34),
        ]))],
        Some(NullBuffer::from(vec![true, false, true, true])),
    ));
    for array in [list, structure] {
        let column = col(array);
        for (filter, expected) in [
            (Filter::OnlyNulls, [false, true, false, false]),
            (Filter::NotNull, [true, false, true, true]),
            (Filter::exact(vec![], true, true), [true; 4]),
            (Filter::exact(vec![], false, false), [false; 4]),
        ] {
            let test = column.compile_filter(&filter).unwrap();
            let mut hits = [false; 4];
            test.evaluate(0..4, &mut hits);
            assert_eq!(hits, expected);
        }
    }
}

#[test]
fn list_search_uses_child_type_with_nulls_slices_and_large_children() {
    let array: ArrayRef = Arc::new(ListArray::from_iter_primitive::<Int32Type, _, _>([
        Some(vec![Some(25)]),
        Some(vec![Some(2)]),
        None,
        Some(vec![]),
        Some(vec![None]),
    ]));
    assert_eq!(matches(&array, "2", 1..5), [true, false, false, false]);
    assert_eq!(matches(&array.slice(1, 3), "2", 0..3), [true, false, false]);
    let mut big = vec![Some(1); CHUNK + 7];
    big[CHUNK + 6] = Some(2);
    let array: ArrayRef = Arc::new(ListArray::from_iter_primitive::<Int32Type, _, _>([
        Some(big),
        Some(vec![Some(25)]),
        Some(vec![Some(2)]),
    ]));
    assert_eq!(matches(&array, "2", 0..3), [true, false, true]);
}

#[test]
fn fixed_lists_search_typed_values_and_respect_slices() {
    let array: ArrayRef = Arc::new(FixedSizeListArray::from_iter_primitive::<Int32Type, _, _>(
        [Some(vec![Some(25)]), Some(vec![Some(2)]), None],
        1,
    ));
    assert_eq!(matches(&array, "2", 0..3), [false, true, false]);
    assert_eq!(matches(&array.slice(1, 2), "2", 0..2), [true, false]);
}

#[test]
fn struct_parent_null_masks_matching_children() {
    let fields = vec![
        Arc::new(Field::new("number", DataType::Int32, false)),
        Arc::new(Field::new("text", DataType::Utf8, false)),
    ];
    let array: ArrayRef = Arc::new(StructArray::new(
        fields.into(),
        vec![
            Arc::new(Int32Array::from(vec![2, 25, 2])),
            Arc::new(StringArray::from(vec!["no", "2", "2"])),
        ],
        Some(NullBuffer::from(vec![true, true, false])),
    ));
    assert_eq!(matches(&array, "2", 0..3), [true, true, false]);
}

#[test]
fn dictionary_search_and_filter_keep_typed_values_and_key_offsets() {
    let array: ArrayRef = Arc::new(
        DictionaryArray::<Int8Type>::try_new(
            Int8Array::from(vec![Some(0), Some(1), None, Some(1)]),
            Arc::new(Int32Array::from(vec![25, 2])),
        )
        .unwrap(),
    );
    assert_eq!(matches(&array, "2", 1..4), [true, false, true]);
    let column = col(array);
    let test = column
        .compile_filter(&Filter::exact(vec!["2".into()], false, false))
        .unwrap();
    let mut first = [false; 2];
    let mut second = [false; 2];
    test.evaluate(0..2, &mut first);
    test.evaluate(2..4, &mut second);
    assert_eq!(first, [false, true]);
    assert_eq!(second, [false, true]);
}

#[test]
fn list_views_allow_overlapping_and_out_of_order_children() {
    use arrow::{array::ListViewArray, buffer::ScalarBuffer};
    let array: ArrayRef = Arc::new(ListViewArray::new(
        Arc::new(Field::new("item", DataType::Int32, true)),
        ScalarBuffer::from(vec![2_i32, 0, 1, 0]),
        ScalarBuffer::from(vec![1_i32, 2, 2, 0]),
        Arc::new(Int32Array::from(vec![25, 2, 30])),
        Some(NullBuffer::from(vec![true, true, false, true])),
    ));
    assert_eq!(matches(&array, "2", 0..4), [false, true, false, false]);
    assert_eq!(matches(&array.slice(1, 3), "2", 0..3), [true, false, false]);
}

#[test]
fn dictionary_filter_capabilities_follow_the_value_type() {
    let array: ArrayRef = Arc::new(
        DictionaryArray::<Int8Type>::try_new(
            Int8Array::from(vec![0]),
            Arc::new(Int32Array::from(vec![2])),
        )
        .unwrap(),
    );
    assert_eq!(
        col(array).info(ColumnId::new(0)).capabilities,
        ColumnCapabilities {
            text_match: false,
            range: true,
            exact: true,
            scatter: false,
            logarithmic: false,
        }
    );
}

#[test]
fn dictionary_is_built_once_and_cached() {
    let column = col(Arc::new(StringArray::from(vec![
        Some("a"),
        None,
        Some("b"),
    ])));
    let first = column.dictionary().unwrap().cloned().expect("dictionary");
    let second = column.dictionary().unwrap().cloned().expect("dictionary");
    assert!(Arc::ptr_eq(&first, &second));

    let high_cardinality: ArrayRef = Arc::new(Int32Array::from_iter_values(0..1000));
    assert!(col(high_cardinality).dictionary().unwrap().is_none());
}

#[test]
fn column_len_and_empty() {
    let empty_col = col(Arc::new(Int32Array::from(Vec::<i32>::new())));
    assert_eq!(empty_col.array().len(), 0);
    assert!(empty_col.array().is_empty());

    let array: ArrayRef = Arc::new(StringArray::from(vec![
        Some("apple"),
        Some("banana"),
        Some("apple"),
        None,
    ]));
    let column = col(array);
    assert_eq!(column.array().len(), 4);
    assert!(!column.array().is_empty());
}
