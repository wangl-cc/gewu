//! Streaming distinct counts for columns without a dictionary. Values are told
//! apart by hashing; floats use the shared normalized bit keys.

use std::{
    cmp::Reverse,
    collections::{
        hash_map::{DefaultHasher, HashMap},
        BinaryHeap,
    },
    hash::{Hash, Hasher},
};

use arrow::{
    array::{Array, ArrayRef, AsArray, PrimitiveArray, UInt32Array},
    compute::take,
    datatypes::{ArrowPrimitiveType, DataType, Float16Type, Float32Type, Float64Type},
    row::{RowConverter, SortField},
};

use super::{ArrayView, Cancellation};
use crate::{column::DistinctValue, format, scope::Selection, Result};

const CHUNK: usize = 64 * 1024;

/// Distinct values counted once, without keeping an index.
pub(crate) struct ValueCounts {
    /// Distinct values among the rows, null included.
    pub distinct: u64,
    /// The `keep` most common values (ties: earliest first), in value
    /// order. Their keys only number them (0, 1, ...): there is no index to
    /// name them, so they cannot be used in a filter.
    pub top: Vec<DistinctValue>,
}

pub(crate) fn count_values(
    view: ArrayView<'_>,
    keep: usize,
    context: &Cancellation,
) -> Result<ValueCounts> {
    context.check()?;
    let array = view.array;
    let rows = view.rows;
    let converter = RowConverter::new(vec![SortField::new(array.data_type().clone())])?;
    let counts = match array.data_type() {
        DataType::Float16 => count_floats(array.as_primitive::<Float16Type>(), rows, context)?,
        DataType::Float32 => count_floats(array.as_primitive::<Float32Type>(), rows, context)?,
        DataType::Float64 => count_floats(array.as_primitive::<Float64Type>(), rows, context)?,
        _ => {
            // hash -> (count, first row)
            let mut counts: HashMap<u64, (u64, u32)> = HashMap::new();
            let mut tally =
                |chunk: &ArrayRef, source: &mut dyn Iterator<Item = usize>| -> Result<()> {
                    context.check()?;
                    let converted = converter.convert_columns(std::slice::from_ref(chunk))?;
                    for (i, (row, r)) in converted.iter().zip(source).enumerate() {
                        context.check_at(i)?;
                        let mut h = DefaultHasher::new();
                        row.as_ref().hash(&mut h);
                        counts.entry(h.finish()).or_insert((0, r as u32)).0 += 1;
                    }
                    Ok(())
                };
            match rows {
                Selection::All(n) => {
                    for start in (0..*n).step_by(CHUNK) {
                        let len = CHUNK.min(n - start);
                        tally(&array.slice(start, len), &mut (start..start + len))?;
                    }
                }
                Selection::Rows(rows) => {
                    for chunk in rows.chunks(CHUNK) {
                        context.check()?;
                        let taken = take(array.as_ref(), &UInt32Array::from(chunk.to_vec()), None)?;
                        tally(&taken, &mut chunk.iter().map(|&r| r as usize))?;
                    }
                }
            }
            counts
        }
    };

    let distinct = counts.len() as u64;
    // Keep only the requested candidates, not a second collection of every
    // distinct value. Higher counts and earlier source rows win ties.
    let mut candidates = BinaryHeap::with_capacity(keep.min(counts.len()));
    for (i, (count, row)) in counts.into_values().enumerate() {
        context.check_at(i)?;
        let candidate = Reverse((count, Reverse(row)));
        if candidates.len() < keep {
            candidates.push(candidate);
        } else if let Some(mut smallest) = candidates.peek_mut() {
            if candidate.0 > smallest.0 {
                *smallest = candidate;
            }
        }
    }
    let top: Vec<(u64, u32)> = candidates
        .into_iter()
        .map(|Reverse((count, Reverse(row)))| (count, row))
        .collect();

    // Back into value order.
    let first_rows = UInt32Array::from(top.iter().map(|&(_, r)| r).collect::<Vec<_>>());
    let taken = take(array.as_ref(), &first_rows, None)?;
    let converted = converter.convert_columns(&[taken])?;
    let mut order: Vec<usize> = (0..top.len()).collect();
    order.sort_by(|&a, &b| converted.row(a).cmp(&converted.row(b)));

    let f = format::formatter(array.as_ref())?;
    let nulls = array.logical_nulls();
    let top = order
        .into_iter()
        .enumerate()
        .map(|(key, i)| DistinctValue {
            key: key as u64,
            label: if nulls
                .as_ref()
                .is_some_and(|nulls| nulls.is_null(top[i].1 as usize))
            {
                None
            } else {
                format::cell(array.as_ref(), &f, top[i].1 as usize)
            },
            count: top[i].0,
        })
        .collect();
    context.check()?;
    Ok(ValueCounts { distinct, top })
}

/// Float equality for analysis groups signed zero and NaN payloads. Native
/// scalar reads avoid a row-format allocation or widened copy of the column.
fn count_floats<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    rows: &Selection,
    context: &Cancellation,
) -> Result<HashMap<u64, (u64, u32)>>
where
    T::Native: Into<f64>,
{
    const NAN_KEY: u64 = f64::NAN.to_bits();
    // All source NaNs become NAN_KEY, leaving this payload exclusively for null.
    const NULL_KEY: u64 = NAN_KEY | 1;
    let mut counts = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        context.check_at(index)?;
        let key = if array.is_null(row) {
            NULL_KEY
        } else {
            let value: f64 = array.value(row).into();
            normalized_bits(value)
        };
        counts.entry(key).or_insert((0, row as u32)).0 += 1;
    }
    context.check()?;
    Ok(counts)
}

fn normalized_bits(value: f64) -> u64 {
    if value == 0.0 {
        0
    } else if value.is_nan() {
        f64::NAN.to_bits()
    } else {
        value.to_bits()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Float64Array, StringArray};

    use super::*;

    #[test]
    fn float_counts_group_signed_zero_and_nan_payloads_separately_from_null() {
        let source = Float64Array::from(vec![
            Some(-0.0),
            Some(0.0),
            Some(f64::NAN),
            Some(f64::from_bits(0x7ff8_1000_0000_0000)),
            Some(f64::from_bits(0xfff8_2000_0000_0000)),
            None,
            None,
            Some(1.0),
        ]);
        for data_type in [DataType::Float16, DataType::Float32, DataType::Float64] {
            let array = arrow::compute::cast(&source, &data_type).unwrap();
            for rows in [
                crate::scope::Selection::All(array.len()),
                crate::scope::Selection::Rows(Arc::new(vec![7, 6, 5, 4, 3, 2, 1, 0])),
            ] {
                let result =
                    count_values(ArrayView::new(&array, &rows), 8, &Cancellation::default())
                        .unwrap();
                assert_eq!(result.distinct, 4);
                assert_eq!(result.top.len(), 4);
                for value in result.top {
                    let expected = match value.label {
                        None => 2,
                        Some(label) => match label.parse::<f64>().unwrap() {
                            value if value.is_nan() => 3,
                            0.0 => 2,
                            1.0 => 1,
                            value => panic!("unexpected value {value}"),
                        },
                    };
                    assert_eq!(value.count, expected);
                }
            }
        }
    }

    #[test]
    fn counting_without_an_index() {
        let a: ArrayRef = Arc::new(StringArray::from(vec![
            Some("b"),
            Some("a"),
            None,
            Some("b"),
            Some("c"),
            Some("b"),
            Some("a"),
        ]));
        let all = crate::scope::Selection::All(a.len());
        let all = count_values(ArrayView::new(&a, &all), 2, &Cancellation::default()).unwrap();
        assert_eq!(all.distinct, 4);
        let top: Vec<_> = all
            .top
            .iter()
            .map(|v| (v.label.clone().unwrap(), v.count))
            .collect();
        assert_eq!(top, [("a".to_string(), 2), ("b".to_string(), 3)]);

        let some_sel = crate::scope::Selection::Rows(Arc::new(vec![2, 4]));
        let some =
            count_values(ArrayView::new(&a, &some_sel), 10, &Cancellation::default()).unwrap();
        assert_eq!(some.distinct, 2);
        assert_eq!(some.top[0].label, None);
    }
}
