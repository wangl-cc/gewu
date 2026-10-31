//! Sort source row IDs with native Arrow comparisons and cooperative cancellation.
//!
//! Bounded runs and merge batches let an obsolete sort stop without panicking
//! inside its comparator. The merge scratch buffer contains row IDs only;
//! single primitive keys use a bounded native Arrow kernel for initial runs. Arrow
//! comparator preparation and an individual value comparison are not interruptible
//! (dictionary preparation can compute logical nulls), so this is a work bound
//! between checkpoints, not a wall-clock cancellation guarantee.

use std::cmp::Ordering;

use arrow::{
    array::{ArrayRef, DynComparator, UInt32Array},
    compute::{sort_to_indices, take, SortOptions},
};

use crate::Result;

const RUN: usize = 4096;
const NATIVE_RUN: usize = 65536;
const BATCH: usize = 4096;

pub(super) fn sort_rows(
    mut rows: Vec<u32>,
    comparators: &[DynComparator],
    primitive: Option<(&ArrayRef, SortOptions)>,
    mut check: impl FnMut() -> Result<()>,
) -> Result<Vec<u32>> {
    check()?;
    let compare = |left: &u32, right: &u32| {
        for comparator in comparators {
            let order = comparator(*left as usize, *right as usize);
            if order != Ordering::Equal {
                return order;
            }
        }
        // Arrow's old lexsort was unstable. Source order makes ties deterministic.
        left.cmp(right)
    };
    let run_size = if primitive.is_some() { NATIVE_RUN } else { RUN };
    for run in rows.chunks_mut(run_size) {
        check()?;
        if let Some((array, options)) = primitive {
            sort_primitive_run(run, array, options, &comparators[0])?;
        } else {
            run.sort_unstable_by(compare);
        }
    }
    check()?;
    if rows.len() <= run_size {
        return Ok(rows);
    }

    let mut scratch = Vec::with_capacity(rows.len());
    let mut width = run_size;
    while width < rows.len() {
        check()?;
        scratch.clear();
        let merged = width.saturating_mul(2);
        for start in (0..rows.len()).step_by(merged) {
            let middle = start.saturating_add(width).min(rows.len());
            let end = start.saturating_add(merged).min(rows.len());
            let mut left = start;
            let mut right = middle;
            while scratch.len() < end {
                check()?;
                let batch_end = scratch.len().saturating_add(BATCH).min(end);
                while scratch.len() < batch_end {
                    // The same bounded loop copies any remaining tail, so a
                    // large unmatched run cannot bypass cancellation checks.
                    let next = if right == end
                        || (left < middle
                            && compare(&rows[left], &rows[right]) != Ordering::Greater)
                    {
                        let row = rows[left];
                        left += 1;
                        row
                    } else {
                        let row = rows[right];
                        right += 1;
                        row
                    };
                    scratch.push(next);
                }
            }
        }
        std::mem::swap(&mut rows, &mut scratch);
        width = merged;
    }
    check()?;
    Ok(rows)
}

/// Arrow's primitive kernel sorts native (row, value) pairs, avoiding repeated
/// indirect source-array reads. Restrict it to one primitive key: dictionary
/// or nested kernels can rank storage beyond this bounded run.
fn sort_primitive_run(
    rows: &mut [u32],
    array: &ArrayRef,
    options: SortOptions,
    compare: &DynComparator,
) -> Result<()> {
    let values = if rows
        .windows(2)
        .all(|pair| pair[0].checked_add(1) == Some(pair[1]))
    {
        array.slice(rows[0] as usize, rows.len())
    } else {
        take(array.as_ref(), &UInt32Array::from(rows.to_vec()), None)?
    };
    let order = sort_to_indices(values.as_ref(), Some(options), None)?;
    let mut sorted: Vec<_> = order
        .values()
        .iter()
        .map(|&index| rows[index as usize])
        .collect();
    // Arrow's primitive kernel is unstable; normalize equal keys to the same
    // source-ID order as generic runs and merging, including nulls and NaNs.
    let mut start = 0;
    while start < sorted.len() {
        let mut end = start + 1;
        while end < sorted.len()
            && compare(sorted[start] as usize, sorted[end] as usize) == Ordering::Equal
        {
            end += 1;
        }
        sorted[start..end].sort_unstable();
        start = end;
    }
    rows.copy_from_slice(&sorted);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering as AtomicOrdering},
        Arc,
    };

    use arrow::{
        array::{
            make_comparator, ArrayRef, DictionaryArray, Float64Array, Int64Array, Int8Array,
            StringArray, UInt32Array,
        },
        compute::{lexsort_to_indices, take, SortColumn, SortOptions},
        datatypes::Int8Type,
    };

    use super::*;
    use crate::{engine::ViewEngine, Error};

    fn agrees_with_arrow(rows: Vec<u32>, columns: &[(&ArrayRef, bool)]) {
        let selected = UInt32Array::from(rows.clone());
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
            .map(|&index| rows[index as usize])
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
            .map(|&index| rows[index as usize])
            .collect();
        if let [(array, descending)] = columns {
            if array.data_type().is_primitive() {
                let actual = sort_rows(
                    rows.clone(),
                    &comparators,
                    Some((array, SortOptions {
                        descending: *descending,
                        nulls_first: false,
                    })),
                    || Ok(()),
                )
                .unwrap();
                assert_eq!(actual, expected);
            }
        }
        let actual = sort_rows(rows, &comparators, None, || Ok(())).unwrap();
        assert_eq!(actual, expected);
        for (&new, &old) in actual.iter().zip(&old_rows) {
            assert!(comparators
                .iter()
                .all(|compare| compare(new as usize, old as usize) == Ordering::Equal));
        }
    }

    #[test]
    fn source_comparators_preserve_arrow_native_and_multikey_order() {
        let len = RUN * 3 + 123;
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
        // rather than positions in the selected view. It spans three runs.
        let rows: Vec<_> = (0..len as u32).rev().filter(|row| row % 3 != 0).collect();
        for array in [&numbers, &floats, &strings, &dictionary] {
            for descending in [false, true] {
                agrees_with_arrow(rows.clone(), &[(array, descending)]);
            }
        }
        agrees_with_arrow(rows.clone(), &[
            (&dictionary, true),
            (&floats, false),
            (&numbers, true),
        ]);
        agrees_with_arrow(rows, &[(&floats, true), (&strings, false)]);
    }

    #[test]
    fn primitive_runs_keep_random_repeated_sorted_and_sparse_selections_exact() {
        let len = NATIVE_RUN * 2 + 17;
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
            let rows = if pattern == 3 {
                (0..len as u32).rev().filter(|row| row % 3 != 0).collect()
            } else {
                (0..len as u32).collect()
            };
            agrees_with_arrow(rows, &[(&array, false)]);
        }
    }

    #[test]
    fn native_runs_check_cancellation_before_and_after_the_bounded_kernel() {
        let len = NATIVE_RUN + 1;
        let array: ArrayRef = Arc::new(Int64Array::from_iter_values((0..len as i64).rev()));
        let options = SortOptions {
            descending: false,
            nulls_first: false,
        };
        let comparators = [make_comparator(array.as_ref(), array.as_ref(), options).unwrap()];
        // Check 1 starts sorting; check 2 precedes the first run; check 3
        // follows that run, before the second kernel can start.
        for cancel_at in [2, 3] {
            let engine = ViewEngine::new(len);
            let token = engine.begin();
            let mut checks = 0;
            let result = sort_rows(
                (0..len as u32).collect(),
                &comparators,
                Some((&array, options)),
                || {
                    checks += 1;
                    if checks == cancel_at {
                        engine.cancel_pending();
                    }
                    token.check()
                },
            );
            assert!(matches!(result, Err(Error::Cancelled)));
            assert_eq!(checks, cancel_at);
        }
    }

    #[test]
    fn empty_single_run_and_merge_boundaries_keep_every_row_once() {
        let array: ArrayRef = Arc::new(Int64Array::from_iter_values(
            (0..RUN * 3 + 1).map(|row| (row % 17) as i64),
        ));
        for len in [0, 1, RUN, RUN + 1, RUN * 2, RUN * 3 + 1] {
            agrees_with_arrow((0..len as u32).rev().collect(), &[(&array, false)]);
        }
    }

    #[test]
    fn cancellation_during_merge_stops_work_and_keeps_the_published_view() {
        let len = RUN * 3 + 17;
        let engine = ViewEngine::new(len);
        let token = engine.begin();
        let comparisons = Arc::new(AtomicUsize::new(0));
        let count = comparisons.clone();
        let comparators: Vec<DynComparator> = vec![Box::new(move |left, right| {
            count.fetch_add(1, AtomicOrdering::Relaxed);
            (left % 13).cmp(&(right % 13))
        })];
        let mut checkpoints = 0;
        let mut before_merge = 0;
        let first_merge_check = 1 + len.div_ceil(RUN) + 1 + 1 + 1;
        let result = sort_rows((0..len as u32).rev().collect(), &comparators, None, || {
            checkpoints += 1;
            if checkpoints == first_merge_check {
                before_merge = comparisons.load(AtomicOrdering::Relaxed);
            }
            if checkpoints == first_merge_check + 1 {
                assert!(comparisons.load(AtomicOrdering::Relaxed) > before_merge);
                engine.cancel_pending();
            }
            token.check()
        });
        assert!(matches!(result, Err(Error::Cancelled)));
        assert_eq!(checkpoints, first_merge_check + 1);
        assert_eq!(engine.current().version, 0);
        assert!(matches!(
            engine.commit(&token, Vec::new()),
            Err(Error::Cancelled)
        ));
        assert_eq!(engine.current().rows.len(), len);
    }
}
