//! Length statistics measure each selected value once, then bin the lengths.

use arrow::{
    array::{Array, AsArray},
    datatypes::DataType,
};

use crate::{
    col_view::{ColView, Selection},
    stats::{scan::for_each_valid_index, Result},
    Histogram,
};

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

pub(crate) fn analyze(view: ColView<'_>, bins: u32) -> Result<LengthSummary> {
    let unit =
        unit(view.data_type()).ok_or_else(|| crate::AnalysisError::UnsupportedOperation {
            operation: crate::AnalysisOperation::Lengths,
            data_type: view.data_type().clone(),
        })?;
    let array = view.array();
    let selection = view.selection();
    // Dispatch once per column. The closures borrow native storage, including
    // sliced offsets, and never construct child arrays or formatted strings.
    macro_rules! scan {
        ($native:expr, | $column:ident, $row:ident | $length:expr) => {{
            let $column = $native;
            Ok(summarize(array.as_ref(), &selection, unit, bins, |$row| {
                $length as u64
            }))
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
    selection: &Selection<'_>,
    unit: LengthUnit,
    bins: u32,
    length: impl Fn(usize) -> u64,
) -> LengthSummary {
    let mut min = u64::MAX;
    let mut max = 0;
    let mut sum = 0_u128;
    let mut empty_count = 0;
    // Measure each value once; the histogram pass reads these native lengths.
    let mut lengths = Vec::with_capacity(selection.len());
    for_each_valid_index(array, selection, |row| {
        let value = length(row);
        min = min.min(value);
        max = max.max(value);
        sum += u128::from(value);
        empty_count += u64::from(value == 0);
        lengths.push(value);
    });
    let non_null = lengths.len() as u64;
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
        return LengthSummary {
            unit,
            empty_count,
            min: None,
            max: None,
            mean: None,
            histogram,
        };
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
    for value in lengths {
        // Equivalent to the shared integer histogram's ceil-based cuts.
        let bin = (u128::from(value - min) * bins as u128 / span) as usize;
        histogram.counts[bin] += 1;
    }
    LengthSummary {
        unit,
        empty_count,
        min: Some(min),
        max: Some(max),
        mean: Some(sum as f64 / non_null as f64),
        histogram,
    }
}

#[cfg(test)]
#[path = "lengths_tests.rs"]
mod tests;
