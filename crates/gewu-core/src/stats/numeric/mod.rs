//! Statistics borrow the original Arrow arrays. Type dispatch happens
//! once per operation; only individual values needed for floating-point results
//! are converted. No widened column or numeric cache is built.

use arrow::{
    array::{downcast_integer_array, Array, ArrayRef, ArrowNativeTypeOp, AsArray, PrimitiveArray},
    compute::single_decimal_to_float_lossy,
    datatypes::*,
};
use num_traits::ToPrimitive;

use crate::{
    col_view::Selection,
    stats::{scan::for_each_valid, Result},
};

#[derive(Debug, Clone)]
pub struct NumericStatistics {
    pub min_label: Option<String>,
    pub max_label: Option<String>,
    /// Exact native median text. Nulls and NaNs are excluded. Opposite infinities
    /// as the two central values have no median. Even temporal medians retain
    /// fractional native units rather than rounding back to the column's unit.
    pub median_label: Option<String>,
    pub mean: Option<f64>,
    /// Approximate temporal mean rounded to the physical unit and formatted by Core.
    /// None for non-temporal values or means outside the representable i64 domain.
    pub mean_label: Option<String>,
    pub std_dev: Option<f64>,
    pub nan_count: u64,
    pub infinite_count: u64,
}

pub(crate) fn supports(data_type: &DataType) -> bool {
    use DataType::*;
    matches!(
        data_type,
        Int8 | Int16
            | Int32
            | Int64
            | UInt8
            | UInt16
            | UInt32
            | UInt64
            | Float16
            | Float32
            | Float64
            | Decimal32(..)
            | Decimal64(..)
            | Decimal128(..)
            | Decimal256(..)
            | Date32
            | Date64
            | Time32(_)
            | Time64(_)
            | Timestamp(..)
            | Duration(_)
    )
}

/// A validated, borrowed numeric column.
#[derive(Clone, Copy)]
pub(crate) struct Numeric<'a>(&'a ArrayRef);

// Keep the supported physical types in one dispatch table. Each branch calls
// a monomorphized operation over the native array, not a per-row type switch.
macro_rules! dispatch {
    ($array:expr, $method:ident($($arg:expr),* $(,)?)) => {{
        let a = $array;
        use DataType::*;
        match a.data_type() {
            Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64 => {
                downcast_integer_array!(a => IntegerColumn(a).$method($($arg),*),
                    _ => unreachable!("integer type checked above"))
            }
            Date32 => IntegerColumn(a.as_primitive::<Date32Type>()).$method($($arg),*),
            Date64 => IntegerColumn(a.as_primitive::<Date64Type>()).$method($($arg),*),
            Time32(TimeUnit::Second) => IntegerColumn(a.as_primitive::<Time32SecondType>()).$method($($arg),*),
            Time32(TimeUnit::Millisecond) => IntegerColumn(a.as_primitive::<Time32MillisecondType>()).$method($($arg),*),
            Time64(TimeUnit::Microsecond) => IntegerColumn(a.as_primitive::<Time64MicrosecondType>()).$method($($arg),*),
            Time64(TimeUnit::Nanosecond) => IntegerColumn(a.as_primitive::<Time64NanosecondType>()).$method($($arg),*),
            Timestamp(TimeUnit::Second, _) => IntegerColumn(a.as_primitive::<TimestampSecondType>()).$method($($arg),*),
            Duration(TimeUnit::Second) => IntegerColumn(a.as_primitive::<DurationSecondType>()).$method($($arg),*),
            Timestamp(TimeUnit::Millisecond, _) => IntegerColumn(a.as_primitive::<TimestampMillisecondType>()).$method($($arg),*),
            Duration(TimeUnit::Millisecond) => IntegerColumn(a.as_primitive::<DurationMillisecondType>()).$method($($arg),*),
            Timestamp(TimeUnit::Microsecond, _) => IntegerColumn(a.as_primitive::<TimestampMicrosecondType>()).$method($($arg),*),
            Duration(TimeUnit::Microsecond) => IntegerColumn(a.as_primitive::<DurationMicrosecondType>()).$method($($arg),*),
            Timestamp(TimeUnit::Nanosecond, _) => IntegerColumn(a.as_primitive::<TimestampNanosecondType>()).$method($($arg),*),
            Duration(TimeUnit::Nanosecond) => IntegerColumn(a.as_primitive::<DurationNanosecondType>()).$method($($arg),*),
            Float16 => RealColumn::new(a.as_primitive::<Float16Type>()).$method($($arg),*),
            Float32 => RealColumn::new(a.as_primitive::<Float32Type>()).$method($($arg),*),
            Float64 => RealColumn::new(a.as_primitive::<Float64Type>()).$method($($arg),*),
            Decimal32(..) => RealColumn::new(a.as_primitive::<Decimal32Type>()).$method($($arg),*),
            Decimal64(..) => RealColumn::new(a.as_primitive::<Decimal64Type>()).$method($($arg),*),
            Decimal128(..) => RealColumn::new(a.as_primitive::<Decimal128Type>()).$method($($arg),*),
            Decimal256(..) => RealColumn::new(a.as_primitive::<Decimal256Type>()).$method($($arg),*),
            _ => unreachable!("Numeric validates supported types"),
        }
    }};
}

mod summary;

impl<'a> Numeric<'a> {
    pub(crate) fn new(array: &'a ArrayRef) -> Option<Self> {
        supports(array.data_type()).then_some(Self(array))
    }

    /// Compute moments and retain source rows for exact extrema.
    fn summarize(self, selection: &Selection<'_>) -> summary::NumericSummary {
        dispatch!(self.0, summarize(selection))
    }

    /// Convert the selected rows to `f64` (`out.len() == selection.len()`),
    /// writing NaN for nulls. Dispatch happens once per call; the conversion
    /// loop is native.
    pub(crate) fn write_f64(self, selection: &Selection<'_>, out: &mut [f64]) {
        dispatch!(self.0, write_f64(selection, out))
    }

    /// The central value (odd count) or the two central values (even count)
    /// of the selected rows in native total order, excluding nulls and NaNs,
    /// as a 1- or 2-element array of the column's type. `None` when nothing
    /// is ordered.
    pub(crate) fn median_values(self, selection: &Selection<'_>) -> Option<ArrayRef> {
        dispatch!(self.0, median_values(selection))
    }
}

/// Write native values converted to `f64`, then mark nulls with NaN.
fn write_f64<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    selection: &Selection<'_>,
    out: &mut [f64],
    as_float: impl Fn(T::Native) -> f64,
) {
    let values = array.values();
    let nulls = array.nulls().filter(|nulls| nulls.null_count() > 0);
    match selection {
        Selection::Range(range) => {
            for (out, &value) in out.iter_mut().zip(&values[range.clone()]) {
                *out = as_float(value);
            }
            if let Some(nulls) = nulls {
                let nulls = nulls.inner().slice(range.start, range.len());
                for index in (!&nulls).set_indices() {
                    out[index] = f64::NAN;
                }
            }
        }
        Selection::Rows(rows) => write_rows(
            values,
            nulls,
            rows.iter().map(|&row| row as usize),
            out,
            as_float,
        ),
        Selection::Mask(mask) => write_rows(values, nulls, mask.set_indices(), out, as_float),
    }
}

/// Gather `rows` into `out` as `f64`, writing NaN for nulls.
#[inline]
fn write_rows<N: Copy>(
    values: &[N],
    nulls: Option<&arrow::buffer::NullBuffer>,
    rows: impl Iterator<Item = usize>,
    out: &mut [f64],
    as_float: impl Fn(N) -> f64,
) {
    for (out, row) in out.iter_mut().zip(rows) {
        *out = match nulls {
            Some(nulls) if nulls.is_null(row) => f64::NAN,
            _ => as_float(values[row]),
        };
    }
}

/// Select the central native values; NaN-like values are excluded by `is_nan`.
fn median_values<T: ArrowPrimitiveType>(
    array: &PrimitiveArray<T>,
    selection: &Selection<'_>,
    is_nan: impl Fn(T::Native) -> bool,
) -> Option<ArrayRef> {
    let mut values = Vec::with_capacity(selection.len());
    for_each_valid(array, selection, |_, value| {
        if !is_nan(value) {
            values.push(value);
        }
    });
    if values.is_empty() {
        return None;
    }
    let even = values.len() % 2 == 0;
    let middle = values.len() / 2;
    // Arrow's comparator order: total order for floats, numeric order otherwise.
    let compare = |a: &T::Native, b: &T::Native| a.compare(*b);
    let (prefix, &mut high, _) = values.select_nth_unstable_by(middle, compare);
    let central = if even {
        let low = *prefix.iter().max_by(|a, b| compare(a, b))?;
        vec![low, high]
    } else {
        vec![high]
    };
    Some(std::sync::Arc::new(
        PrimitiveArray::<T>::from_iter_values(central).with_data_type(array.data_type().clone()),
    ))
}

pub(crate) fn analyze(view: crate::col_view::ColView<'_>) -> Result<NumericStatistics> {
    if !supports(view.data_type()) {
        return Err(crate::AnalysisError::UnsupportedOperation {
            operation: crate::AnalysisOperation::Statistics,
            data_type: view.data_type().clone(),
        });
    }
    let selection = view.selection();
    let numeric = Numeric::new(view.array()).expect("supported numeric type");
    let summary = numeric.summarize(&selection);
    let formatter = crate::format::formatter(view.array().as_ref())?;
    let label = |row: Option<usize>| {
        row.and_then(|row| crate::format::cell(view.array().as_ref(), &formatter, row))
    };
    Ok(NumericStatistics {
        min_label: label(summary.min_row),
        max_label: label(summary.max_row),
        median_label: super::median::label(numeric, &selection)?,
        mean: summary.mean.filter(|v| v.is_finite()),
        mean_label: temporal_mean_label(view.data_type(), summary.mean)?,
        std_dev: summary
            .std_dev
            .filter(|v| v.is_finite() && summary.infinite_count == 0),
        nan_count: summary.nan_count,
        infinite_count: summary.infinite_count,
    })
}

fn temporal_mean_label(data_type: &DataType, mean: Option<f64>) -> Result<Option<String>> {
    if !data_type.is_temporal() {
        return Ok(None);
    }
    let Some(value) = mean.and_then(|mean| ToPrimitive::to_i64(&mean.round())) else {
        return Ok(None);
    };
    let raw: ArrayRef = std::sync::Arc::new(arrow::array::Int64Array::from(vec![value]));
    let typed = match data_type {
        DataType::Date32 | DataType::Time32(_) => {
            arrow::compute::cast(&arrow::compute::cast(&raw, &DataType::Int32)?, data_type)?
        }
        _ => arrow::compute::cast(&raw, data_type)?,
    };
    let formatter = crate::format::formatter(typed.as_ref())?;
    Ok(crate::format::cell(typed.as_ref(), &formatter, 0))
}

struct IntegerColumn<'a, T: ArrowPrimitiveType>(&'a PrimitiveArray<T>);

impl<'a, T: ArrowPrimitiveType> IntegerColumn<'a, T>
where
    T::Native: Into<i128>,
{
    fn write_f64(self, selection: &Selection<'_>, out: &mut [f64]) {
        write_f64(self.0, selection, out, |value| value.into() as f64)
    }

    fn median_values(self, selection: &Selection<'_>) -> Option<ArrayRef> {
        median_values(self.0, selection, |_| false)
    }
}

/// Operations that differ between native floating values and decimal coefficients.
trait RealType: ArrowPrimitiveType {
    fn as_float(value: Self::Native, scale: i32) -> f64;
}

macro_rules! float_type {
    ($($ty:ty),* $(,)?) => {$(
        impl RealType for $ty {
            fn as_float(value: Self::Native, _: i32) -> f64 { f64::from(value) }
        }
    )*};
}
float_type!(Float16Type, Float32Type, Float64Type);

macro_rules! decimal_type {
    ($($ty:ty),* $(,)?) => {$(
        impl RealType for $ty {
            fn as_float(value: Self::Native, scale: i32) -> f64 {
                single_decimal_to_float_lossy::<Self, _>(
                    &|v| v.to_f64().expect("decimal coefficients fit in f64's exponent range"),
                    value, scale,
                )
            }
        }
    )*};
}
decimal_type!(Decimal32Type, Decimal64Type, Decimal128Type, Decimal256Type);

/// Borrowed native values plus the decimal scale, with no owned array buffers.
struct RealColumn<'a, T: RealType> {
    array: &'a PrimitiveArray<T>,
    scale: i32,
}

impl<'a, T: RealType> RealColumn<'a, T> {
    fn new(array: &'a PrimitiveArray<T>) -> Self {
        let scale = match array.data_type() {
            DataType::Decimal32(_, scale)
            | DataType::Decimal64(_, scale)
            | DataType::Decimal128(_, scale)
            | DataType::Decimal256(_, scale) => i32::from(*scale),
            _ => 0,
        };
        Self { array, scale }
    }

    fn write_f64(self, selection: &Selection<'_>, out: &mut [f64]) {
        write_f64(self.array, selection, out, |value| {
            T::as_float(value, self.scale)
        })
    }

    fn median_values(self, selection: &Selection<'_>) -> Option<ArrayRef> {
        median_values(self.array, selection, |value| {
            T::as_float(value, self.scale).is_nan()
        })
    }
}

#[cfg(test)]
mod tests;
