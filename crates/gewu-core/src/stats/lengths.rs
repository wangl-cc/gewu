//! Length statistics scan native values twice and retain only the histogram.

use arrow::{
    array::{Array, AsArray},
    datatypes::DataType,
};

use super::ArrayView;
use crate::{execution::scope::Selection, Cancellation, Error, Histogram, Result};

#[derive(Debug, Clone)]
pub struct LengthSummary {
    pub unit: LengthUnit,
    pub empty_count: u64,
    pub min: Option<u64>,
    pub max: Option<u64>,
    pub mean: Option<f64>,
    pub histogram: Histogram,
}

/// What a length measurement counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthUnit {
    Characters,
    Bytes,
    Elements,
}

pub(super) fn unit(data_type: &DataType) -> Option<LengthUnit> {
    use DataType::*;
    match data_type {
        Utf8 | LargeUtf8 | Utf8View => Some(LengthUnit::Characters),
        Binary | LargeBinary | BinaryView | FixedSizeBinary(_) => Some(LengthUnit::Bytes),
        List(_) | LargeList(_) | ListView(_) | LargeListView(_) | FixedSizeList(..) | Map(..) => {
            Some(LengthUnit::Elements)
        }
        _ => None,
    }
}

pub(crate) fn analyze(
    view: ArrayView<'_>,
    bins: u32,
    context: &Cancellation,
) -> Result<LengthSummary> {
    context.check()?;
    let array = view.array;
    let rows = view.rows;
    let unit = unit(array.data_type()).ok_or_else(|| {
        Error::InvalidArgument(format!("{} has no length analysis", array.data_type()))
    })?;
    // Dispatch once per column. The closures borrow native storage, including
    // sliced offsets, and never construct child arrays or formatted strings.
    macro_rules! scan {
        ($native:expr, | $column:ident, $row:ident | $length:expr) => {{
            let $column = $native;
            summarize(array.as_ref(), rows, unit, bins, context, |$row| {
                $length as u64
            })
        }};
    }
    match array.data_type() {
        DataType::Utf8 => scan!(array.as_string::<i32>(), |a, row| a
            .value(row)
            .chars()
            .count()),
        DataType::LargeUtf8 => scan!(array.as_string::<i64>(), |a, row| a
            .value(row)
            .chars()
            .count()),
        DataType::Utf8View => scan!(array.as_string_view(), |a, row| a
            .value(row)
            .chars()
            .count()),
        DataType::Binary => scan!(array.as_binary::<i32>(), |a, row| a.value(row).len()),
        DataType::LargeBinary => scan!(array.as_binary::<i64>(), |a, row| a.value(row).len()),
        DataType::BinaryView => scan!(array.as_binary_view(), |a, row| a.value(row).len()),
        DataType::FixedSizeBinary(_) => {
            scan!(array.as_fixed_size_binary(), |a, _row| a.value_length())
        }
        DataType::List(_) => scan!(array.as_list::<i32>(), |a, row| a.value_length(row)),
        DataType::LargeList(_) => scan!(array.as_list::<i64>(), |a, row| a.value_length(row)),
        DataType::ListView(_) => scan!(array.as_list_view::<i32>(), |a, row| a.value_sizes()[row]),
        DataType::LargeListView(_) => {
            scan!(array.as_list_view::<i64>(), |a, row| a.value_sizes()[row])
        }
        DataType::FixedSizeList(..) => {
            scan!(array.as_fixed_size_list(), |a, _row| a.value_length())
        }
        DataType::Map(..) => scan!(array.as_map(), |a, row| a.value_length(row)),
        _ => unreachable!("validated length type"),
    }
}

fn summarize(
    array: &dyn Array,
    rows: &Selection,
    unit: LengthUnit,
    bins: u32,
    context: &Cancellation,
    length: impl Fn(usize) -> u64,
) -> Result<LengthSummary> {
    let mut min = u64::MAX;
    let mut max = 0;
    let mut sum = 0_u128;
    let mut empty_count = 0;
    let mut null_count = 0;
    for (index, row) in rows.iter().enumerate() {
        context.check_at(index)?;
        if array.is_null(row) {
            null_count += 1;
            continue;
        }
        let value = length(row);
        min = min.min(value);
        max = max.max(value);
        sum += u128::from(value);
        empty_count += u64::from(value == 0);
    }
    context.check()?;
    let total = rows.len() as u64;
    let non_null = total - null_count;
    let mut histogram = Histogram {
        scale: crate::AxisScale::Linear,
        lo: None,
        hi: None,
        min_label: None,
        max_label: None,
        integer: true,
        counts: Vec::new(),
        bin_edges: Vec::new(),
        non_positive_count: 0,
        infinite_count: 0,
    };
    if non_null == 0 {
        return Ok(LengthSummary {
            unit,
            empty_count,
            min: None,
            max: None,
            mean: None,
            histogram,
        });
    }
    let span = u128::from(max) - u128::from(min) + 1;
    let bins = span.min(u128::from(bins)) as usize;
    histogram.counts = vec![0; bins];
    let mut lo = min as f64;
    let mut hi = (u128::from(max) + 1) as f64;
    if lo as u128 > u128::from(min) {
        lo = lo.next_down();
    }
    if (hi as u128) < u128::from(max) + 1 {
        hi = hi.next_up();
    }
    histogram.lo = Some(lo);
    histogram.hi = Some(hi);
    histogram.min_label = Some(min.to_string());
    histogram.max_label = Some(max.to_string());
    for (index, row) in rows.iter().enumerate() {
        context.check_at(index)?;
        if !array.is_null(row) {
            // Equivalent to the shared integer histogram's ceil-based cuts.
            let bin = (u128::from(length(row) - min) * bins as u128 / span) as usize;
            histogram.counts[bin] += 1;
        }
    }
    context.check()?;
    Ok(LengthSummary {
        unit,
        empty_count,
        min: Some(min),
        max: Some(max),
        mean: Some(sum as f64 / non_null as f64),
        histogram,
    })
}

#[cfg(test)]
mod tests {
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
        let context = Cancellation::default();
        let all = crate::execution::scope::Selection::All(4);
        let result = analyze(ArrayView::new(&array, &all), 40, &context).unwrap();
        assert_eq!(result.unit, LengthUnit::Characters);
        assert_eq!(
            (result.min, result.max, result.empty_count),
            (Some(0), Some(2), 1)
        );
        assert_eq!(result.mean, Some(4.0 / 3.0));
        assert_eq!(result.histogram.counts, [1, 0, 2]);
        assert_eq!(
            ArrayView::new(&array, &all).count_nulls(&context).unwrap(),
            1
        );
        for rows in [vec![], vec![3]] {
            let sel = crate::execution::scope::Selection::Rows(Arc::from(rows.clone()));
            let result = analyze(ArrayView::new(&array, &sel), 40, &context).unwrap();
            assert_eq!((result.min, result.max, result.mean), (None, None, None));
            assert!(result.histogram.counts.is_empty());
            assert_eq!(
                ArrayView::new(&array, &sel).count_nulls(&context).unwrap(),
                rows.len() as u64
            );
        }
        let sel2 = crate::execution::scope::Selection::Rows(Arc::from(vec![2]));
        let result = analyze(ArrayView::new(&array, &sel2), 40, &context).unwrap();
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
        let sel = crate::execution::scope::Selection::Rows(Arc::from(vec![0, 1, 2]));
        let result = analyze(ArrayView::new(&array, &sel), 40, &Cancellation::default()).unwrap();
        assert_eq!(result.unit, LengthUnit::Elements);
        assert_eq!(
            (result.min, result.max, result.mean),
            (Some(0), Some(2), Some(1.0))
        );
        assert_eq!(result.histogram.counts, [1, 0, 1]);
        assert_eq!(
            (
                result.empty_count,
                ArrayView::new(&array, &sel)
                    .count_nulls(&Cancellation::default())
                    .unwrap(),
                sel.len()
            ),
            (1, 1, 3)
        );
    }
}
