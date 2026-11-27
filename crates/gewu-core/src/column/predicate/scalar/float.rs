//! Floating search precision and exact equality are separate operations.

use arrow::{
    array::{Array, ArrayRef, AsArray},
    datatypes::{DataType, Float16Type, Float32Type, Float64Type},
};

use crate::{
    column::predicate::Result,
    execution::{
        predicate::{collect_bits, with_nulls, Predicate},
        selection::RowSelection,
    },
};

pub(super) fn search<'a>(array: &'a ArrayRef, text: &str) -> Option<Predicate<'a>> {
    let value: f64 = text.parse().ok()?;
    if value.is_nan() {
        return Some(nan(array));
    }
    if value.is_infinite()
        && !matches!(
            text.to_ascii_lowercase().as_str(),
            "inf" | "+inf" | "-inf" | "infinity" | "+infinity" | "-infinity"
        )
    {
        return None;
    }
    if value.is_infinite() {
        return Some(bound_range(array, value, value));
    }
    let (mantissa, exponent) = match text.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.parse::<i64>().ok()?),
        None => (text, 0),
    };
    let decimals = mantissa.split_once('.').map_or(0, |(_, d)| d.len() as i64);
    // Saturate only the exponent calculation, never the target. Very tiny
    // precision produces the exact parsed value, including at zero.
    let power = exponent
        .saturating_sub(decimals)
        .clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    let half = 0.5 * 10f64.powi(power);
    Some(bound_range(
        array,
        (value - half).min(value),
        (value + half).next_down().max(value),
    ))
}

pub(in crate::column) fn bound_range(array: &ArrayRef, lo: f64, hi: f64) -> Predicate<'_> {
    macro_rules! native {
        ($ty:ty) => {{
            let array = array.as_primitive::<$ty>();
            let values = array.values();
            let has_nulls = array.null_count() > 0;
            Predicate::new(Box::new(move |rows, output| match rows {
                RowSelection::Range(range) => {
                    let slice = &values[range.clone()];
                    for (i, &v) in slice.iter().enumerate() {
                        let v = f64::from(v);
                        output[i] = v >= lo && v <= hi;
                    }
                    if has_nulls {
                        for (i, row) in range.enumerate() {
                            if array.is_null(row) {
                                output[i] = false;
                            }
                        }
                    }
                }
                RowSelection::Indices(indices) => {
                    for (i, &row) in indices.iter().enumerate() {
                        output[i] = (!has_nulls || array.is_valid(row)) && {
                            let v = f64::from(values[row]);
                            v >= lo && v <= hi
                        };
                    }
                }
            }))
            .with_bitmap(Box::new(move |rows| {
                let bits = collect_bits(rows.clone(), |row| {
                    let v = f64::from(values[row]);
                    v >= lo && v <= hi
                });
                with_nulls(bits, array.nulls(), &rows, false)
            }))
        }};
    }
    match array.data_type() {
        DataType::Float16 => native!(Float16Type),
        DataType::Float32 => native!(Float32Type),
        DataType::Float64 => native!(Float64Type),
        _ => unreachable!("float dispatch"),
    }
}

pub(super) fn nan(array: &ArrayRef) -> Predicate<'_> {
    macro_rules! native {
        ($ty:ty) => {{
            let a = array.as_primitive::<$ty>();
            Predicate::new(Box::new(move |rows, output| {
                for (i, row) in rows.iter().enumerate() {
                    output[i] = a.is_valid(row) && a.value(row).is_nan();
                }
            }))
        }};
    }
    match array.data_type() {
        DataType::Float16 => native!(Float16Type),
        DataType::Float32 => native!(Float32Type),
        DataType::Float64 => native!(Float64Type),
        _ => unreachable!("float dispatch"),
    }
}

pub(super) fn equals<'a>(array: &'a ArrayRef, text: &str) -> Result<Predicate<'a>> {
    let target = parse(array.data_type(), text)?;
    if target.is_infinite()
        && !matches!(
            text.to_ascii_lowercase().as_str(),
            "inf" | "+inf" | "-inf" | "infinity" | "+infinity" | "-infinity"
        )
    {
        return Ok(Predicate::empty());
    }
    if target.is_nan() {
        return Ok(nan(array));
    }
    // Equality parses to the native representation, unlike a search interval
    // or an inclusive range endpoint. No stored array is cast.
    let original = target;
    let target = match array.data_type() {
        DataType::Float16 => {
            use arrow::compute::kernels::cast_utils::Parser;
            let value =
                Float16Type::parse(text).ok_or_else(|| crate::FilterError::InvalidValue {
                    value: text.into(),
                    expected: DataType::Float16,
                    reason: crate::ValueParseError::InvalidSyntax,
                })?;
            f64::from(value)
        }
        DataType::Float32 => {
            f64::from(
                text.parse::<f32>()
                    .map_err(|_| crate::FilterError::InvalidValue {
                        value: text.into(),
                        expected: DataType::Float32,
                        reason: crate::ValueParseError::InvalidSyntax,
                    })?,
            )
        }
        DataType::Float64 => target,
        _ => unreachable!("float dispatch"),
    };
    if original.is_finite() && !target.is_finite() {
        return Ok(Predicate::empty());
    }
    Ok(bound_range(array, target, target))
}

fn parse(data_type: &DataType, text: &str) -> Result<f64> {
    text.parse().map_err(|_| crate::FilterError::InvalidValue {
        value: text.into(),
        expected: data_type.clone(),
        reason: crate::ValueParseError::InvalidSyntax,
    })
}

#[cfg(test)]
#[path = "float_tests.rs"]
mod tests;
