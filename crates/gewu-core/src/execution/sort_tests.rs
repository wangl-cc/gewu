use std::sync::Arc;

use arrow::{
    array::{
        BooleanArray, DictionaryArray, Float64Array, Int64Array, Int8Array, ListArray, StringArray,
        StringViewArray, UInt32Array,
    },
    compute::{lexsort_to_indices, take, SortColumn},
    datatypes::{Int32Type, Int8Type},
};

use super::*;

/// Sort with every applicable kernel and compare against Arrow lexsort
/// with the source row appended as the final key.
fn agrees_with_arrow(rows: RowSlice<'_>, columns: &[(&ArrayRef, bool)]) {
    let row_vec: Vec<u32> = rows.iter_u32().collect();
    let selected = UInt32Array::from(row_vec.clone());
    let mut sort_columns: Vec<_> = columns
        .iter()
        .map(|&(array, descending)| SortColumn {
            values: take(array.as_ref(), &selected, None).unwrap(),
            options: Some(SortOptions {
                descending,
                nulls_first: false,
            }),
        })
        .collect();
    sort_columns.push(SortColumn {
        values: Arc::new(selected),
        options: None,
    });
    let order = lexsort_to_indices(&sort_columns, None).unwrap();
    let expected: Vec<_> = order
        .values()
        .iter()
        .map(|&index| row_vec[index as usize])
        .collect();

    let dictionaries: Vec<_> = columns
        .iter()
        .map(|(array, _)| {
            codes_match_arrow_order(array.data_type())
                .then(|| Dictionary::build(array).unwrap())
                .flatten()
        })
        .collect();
    let keys = |use_codes: bool| -> Vec<SortKey<'_>> {
        columns
            .iter()
            .zip(&dictionaries)
            .map(|(&(array, descending), dictionary)| SortKey {
                array,
                descending,
                codes: dictionary.as_ref().filter(|_| use_codes),
            })
            .collect()
    };
    assert_eq!(sort_rows(rows, &keys(true)).unwrap(), expected, "codes");
    assert_eq!(sort_rows(rows, &keys(false)).unwrap(), expected, "native");
    let mut fallback: Vec<u32> = row_vec.clone();
    fallback.sort_unstable();
    sort_row_format(&mut fallback, &keys(false)).unwrap();
    assert_eq!(fallback, expected, "row format");
    let mut fallback: Vec<u32> = row_vec;
    fallback.sort_unstable();
    sort_comparators(&mut fallback, &keys(false)).unwrap();
    assert_eq!(fallback, expected, "comparators");
}

#[test]
fn kernels_preserve_arrow_native_and_multikey_order() {
    let len = 12_411;
    let numbers: ArrayRef = Arc::new(Int64Array::from_iter((0..len).map(|row| {
        [
            Some(i64::MAX),
            Some(9_007_199_254_740_993),
            None,
            Some(-1),
            Some(i64::MIN),
            Some(0),
        ][row % 6]
    })));
    let floats: ArrayRef = Arc::new(Float64Array::from_iter((0..len).map(|row| {
        [
            Some(f64::NAN),
            Some(-f64::NAN),
            Some(f64::INFINITY),
            Some(f64::NEG_INFINITY),
            Some(-0.0),
            Some(0.0),
            None,
            Some(1.5),
        ][row % 8]
    })));
    let strings: ArrayRef = Arc::new(StringArray::from_iter((0..len).map(|row| {
        [
            Some("z"),
            Some(""),
            None,
            Some("é"),
            Some("e\u{301}"),
            Some("A"),
            Some("abcdefgh"),
            Some("abcdefgh\0"),
            Some("0123456789abcdef"),
            Some("0123456789abcdef\0"),
            Some("0123456789abcdefg"),
            Some("0123456789abcde"),
        ][row % 12]
    })));
    // Distinct values beyond the dictionary limit exercise prefix ties.
    let wide: ArrayRef = Arc::new(StringViewArray::from_iter((0..len).map(|row| {
        (row % 13 != 0).then(|| format!("shared-prefix-{:05}", (row * 7919) % 3_001))
    })));
    let dictionary: ArrayRef = Arc::new(
        DictionaryArray::<Int8Type>::try_new(
            Int8Array::from_iter(
                (0..len).map(|row| [Some(0), Some(1), Some(2), None, Some(3)][row % 5]),
            ),
            Arc::new(StringArray::from(vec![
                Some("z"),
                None,
                Some("a"),
                Some("a"),
            ])),
        )
        .unwrap(),
    );
    let booleans: ArrayRef = Arc::new(BooleanArray::from_iter(
        (0..len).map(|row| [Some(true), None, Some(false)][row % 3]),
    ));
    let lists: ArrayRef = Arc::new(ListArray::from_iter_primitive::<Int32Type, _, _>(
        (0..len).map(|row| (row % 4 != 0).then(|| vec![Some((row % 7) as i32), None])),
    ));
    // A sparse, reversed selection verifies comparisons use source IDs,
    // rather than positions in the selected view.
    let rows: Vec<_> = (0..len as u32).rev().filter(|row| row % 3 != 0).collect();
    for array in [
        &numbers,
        &floats,
        &strings,
        &wide,
        &dictionary,
        &booleans,
        &lists,
    ] {
        for descending in [false, true] {
            agrees_with_arrow(RowSlice::Selected(&rows), &[(array, descending)]);
            agrees_with_arrow(RowSlice::All(len), &[(array, descending)]);
        }
    }
    for rows in [RowSlice::Selected(&rows), RowSlice::All(len)] {
        agrees_with_arrow(rows, &[
            (&dictionary, true),
            (&floats, false),
            (&numbers, true),
        ]);
        agrees_with_arrow(rows, &[(&floats, true), (&strings, false)]);
        agrees_with_arrow(rows, &[(&wide, false), (&booleans, true)]);
        agrees_with_arrow(rows, &[
            (&strings, true),
            (&lists, false),
            (&numbers, false),
        ]);
        agrees_with_arrow(rows, &[
            (&booleans, false),
            (&dictionary, false),
            (&wide, true),
        ]);
    }
}

#[test]
fn primitive_sort_keeps_random_repeated_sorted_and_sparse_selections_exact() {
    let len = 70_000usize;
    for pattern in 0..4 {
        let array: ArrayRef = Arc::new(Int64Array::from_iter((0..len).map(|row| {
            if pattern == 1 && row % 97 == 0 {
                None
            } else {
                Some(match pattern {
                    1 => (row % 17) as i64,
                    2 => row as i64,
                    _ => ((row * 104729) % len) as i64,
                })
            }
        })));
        if pattern == 3 {
            let rows: Vec<u32> = (0..len as u32).rev().filter(|row| row % 3 != 0).collect();
            agrees_with_arrow(RowSlice::Selected(&rows), &[(&array, false)]);
        } else {
            agrees_with_arrow(RowSlice::All(len), &[(&array, false)]);
        }
    }
}

#[test]
fn empty_single_and_repeated_selections_preserve_every_row_once() {
    let array: ArrayRef = Arc::new(Int64Array::from_iter_values(
        (0..5_000).map(|row| (row % 17) as i64),
    ));
    for len in [0, 1, 2, 17, 5_000] {
        let rev: Vec<u32> = (0..len as u32).rev().collect();
        agrees_with_arrow(RowSlice::Selected(&rev), &[(&array, false)]);
        agrees_with_arrow(RowSlice::All(len), &[(&array, false)]);
    }
}
