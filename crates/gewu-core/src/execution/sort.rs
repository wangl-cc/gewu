//! Sort source row IDs with native Arrow comparisons and deterministic source-row tie-breaking.

use std::cmp::Ordering;

use arrow::{
    array::{ArrayRef, DynComparator, UInt32Array},
    compute::{sort_to_indices, take, SortOptions},
    error::ArrowError,
};

use crate::row_ids::RowSlice;

pub(crate) fn sort_rows(
    rows: RowSlice<'_>,
    comparators: &[DynComparator],
    primitive: Option<(&ArrayRef, SortOptions)>,
) -> Result<Vec<u32>, ArrowError> {
    if rows.len() <= 1 {
        return Ok(rows.iter_u32().collect());
    }
    if let Some((array, options)) = primitive {
        return sort_primitive(rows, array, options, &comparators[0]);
    }
    let mut rows: Vec<u32> = rows.iter_u32().collect();
    rows.sort_unstable_by(|left, right| {
        for comparator in comparators {
            let order = comparator(*left as usize, *right as usize);
            if order != Ordering::Equal {
                return order;
            }
        }
        // Arrow's lexsort is unstable; source order keeps equal keys deterministic.
        left.cmp(right)
    });
    Ok(rows)
}

/// Arrow's primitive kernel sorts native `(row, value)` pairs without repeated
/// indirect source-array reads. Restrict it to one primitive key so multi-key
/// or nested types use the generic source-comparator sort directly.
fn sort_primitive(
    rows: RowSlice<'_>,
    array: &ArrayRef,
    options: SortOptions,
    compare: &DynComparator,
) -> Result<Vec<u32>, ArrowError> {
    let mut sorted = match rows {
        RowSlice::All(n) => {
            let values = if n == array.len() {
                array.clone()
            } else {
                array.slice(0, n)
            };
            let order = sort_to_indices(values.as_ref(), Some(options), None)?;
            order.values().to_vec()
        }
        RowSlice::Selected(selected) => {
            let values = if selected
                .windows(2)
                .all(|pair| pair[0].checked_add(1) == Some(pair[1]))
            {
                array.slice(selected[0] as usize, selected.len())
            } else {
                take(array.as_ref(), &UInt32Array::from(selected.to_vec()), None)?
            };
            let order = sort_to_indices(values.as_ref(), Some(options), None)?;
            order
                .values()
                .iter()
                .map(|&index| selected[index as usize])
                .collect()
        }
    };
    // Arrow's primitive kernel is unstable; normalize equal keys to source-ID
    // order, including nulls and NaNs.
    let mut start = 0;
    while start < sorted.len() {
        let mut end = start + 1;
        while end < sorted.len()
            && compare(sorted[start] as usize, sorted[end] as usize) == Ordering::Equal
        {
            end += 1;
        }
        if end - start > 1 {
            sorted[start..end].sort_unstable();
        }
        start = end;
    }
    Ok(sorted)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{
            make_comparator, ArrayRef, DictionaryArray, Float64Array, Int64Array, Int8Array,
            StringArray, UInt32Array,
        },
        compute::{lexsort_to_indices, take, SortColumn, SortOptions},
        datatypes::Int8Type,
    };

    use super::*;

    fn agrees_with_arrow(rows: RowSlice<'_>, columns: &[(&ArrayRef, bool)]) {
        let row_vec: Vec<u32> = rows.iter_u32().collect();
        let selected = UInt32Array::from(row_vec.clone());
        let mut old_columns = Vec::new();
        let mut comparators = Vec::new();
        for &(array, descending) in columns {
            let options = SortOptions {
                descending,
                nulls_first: false,
            };
            old_columns.push(SortColumn {
                values: take(array.as_ref(), &selected, None).unwrap(),
                options: Some(options),
            });
            comparators.push(make_comparator(array.as_ref(), array.as_ref(), options).unwrap());
        }
        let old_order = lexsort_to_indices(&old_columns, None).unwrap();
        let old_rows: Vec<_> = old_order
            .values()
            .iter()
            .map(|&index| row_vec[index as usize])
            .collect();
        // The old ordering is unstable for equal keys. Add the same explicit
        // source-row tie-break so equality here checks every output position.
        old_columns.push(SortColumn {
            values: Arc::new(selected),
            options: None,
        });
        let order = lexsort_to_indices(&old_columns, None).unwrap();
        let expected: Vec<_> = order
            .values()
            .iter()
            .map(|&index| row_vec[index as usize])
            .collect();
        if let [(array, descending)] = columns {
            if array.data_type().is_primitive() {
                let actual = sort_rows(
                    rows,
                    &comparators,
                    Some((array, SortOptions {
                        descending: *descending,
                        nulls_first: false,
                    })),
                )
                .unwrap();
                assert_eq!(actual, expected);
            }
        }
        let actual = sort_rows(rows, &comparators, None).unwrap();
        assert_eq!(actual, expected);
        for (&new, &old) in actual.iter().zip(&old_rows) {
            assert!(comparators
                .iter()
                .all(|compare| compare(new as usize, old as usize) == Ordering::Equal));
        }
    }

    #[test]
    fn source_comparators_preserve_arrow_native_and_multikey_order() {
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
            ][row % 6]
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
        // A sparse, reversed selection verifies comparisons use source IDs,
        // rather than positions in the selected view.
        let rows: Vec<_> = (0..len as u32).rev().filter(|row| row % 3 != 0).collect();
        for array in [&numbers, &floats, &strings, &dictionary] {
            for descending in [false, true] {
                agrees_with_arrow(RowSlice::Selected(&rows), &[(array, descending)]);
                agrees_with_arrow(RowSlice::All(len), &[(array, descending)]);
            }
        }
        agrees_with_arrow(RowSlice::Selected(&rows), &[
            (&dictionary, true),
            (&floats, false),
            (&numbers, true),
        ]);
        agrees_with_arrow(RowSlice::All(len), &[
            (&dictionary, true),
            (&floats, false),
            (&numbers, true),
        ]);
        agrees_with_arrow(RowSlice::Selected(&rows), &[
            (&floats, true),
            (&strings, false),
        ]);
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
}
