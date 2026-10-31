//! A validated scalar range endpoint, bound to a complete logical Arrow type.
//! The original label is for editing; execution consumes the parsed values and
//! precomputed native coefficient endpoints, never reparses that label.

use std::{cmp::Ordering, sync::Arc};

use arrow::{
    array::{ArrayRef, Int32Array, Int64Array},
    compute::cast,
    datatypes::{i256, DataType},
};
use num_traits::ToPrimitive;

use super::scalar::{
    decimal::{Endpoint, Literal},
    float, integer, temporal,
};
use crate::{format, predicate::Predicate, Error, LogicalType, Result};

/// A parsed endpoint for one column's logical type. Construct through [`Self::parse`]
/// for text inputs or a native constructor for numerical controls. Bounds are
/// not interchangeable between different units, scales, widths or time zones.
/// Cloning shares immutable parsed state; it does not copy labels or digits.
#[derive(Clone, Debug, PartialEq)]
pub struct Bound {
    inner: Arc<BoundData>,
}

#[derive(Clone, Debug, PartialEq)]
struct BoundData {
    data_type: DataType,
    value: Value,
    display: String,
    axis: f64,
}

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Exact {
        literal: Literal,
        lower: Endpoint,
        upper: Endpoint,
    },
    Float {
        value: f64,
        lower: f64,
        upper: f64,
        exact: Option<Literal>,
    },
    Temporal(i64),
}

impl Bound {
    fn new(inner: BoundData) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Parse an endpoint according to the complete column type. Integer and
    /// Decimal inputs retain exact fractional bounds and arbitrary digit lengths.
    /// Invalid inputs, NaN and unsupported types return an error.
    pub fn parse(data_type: &DataType, text: &str) -> Result<Self> {
        let text = text.trim();
        match LogicalType::from_arrow(data_type) {
            LogicalType::Integer { .. }
            | LogicalType::Decimal { .. }
            | LogicalType::Duration { .. } => {
                let literal = Literal::parse(text).ok_or_else(|| invalid_text(data_type, text))?;
                let axis = text
                    .parse::<f64>()
                    .map_err(|_| invalid_text(data_type, text))?;
                Self::exact(data_type, literal, text.to_owned(), axis)
            }
            LogicalType::Float { .. } => {
                let value = text
                    .parse::<f64>()
                    .map_err(|_| invalid_text(data_type, text))?;
                if value.is_nan() {
                    return Err(invalid_text(data_type, text));
                }
                Ok(Self::new(BoundData {
                    data_type: data_type.clone(),
                    value: Value::Float {
                        value,
                        lower: float::integer_endpoint(text, value, true),
                        upper: float::integer_endpoint(text, value, false),
                        exact: Literal::parse(text),
                    },
                    display: text.to_owned(),
                    axis: value,
                }))
            }
            LogicalType::Date | LogicalType::Time | LogicalType::Timestamp { .. } => {
                let value = temporal::parse_value(data_type, text)?;
                Ok(Self::new(BoundData {
                    data_type: data_type.clone(),
                    value: Value::Temporal(value),
                    display: text.to_owned(),
                    axis: value as f64,
                }))
            }
            _ => Err(invalid_type(data_type)),
        }
    }

    /// Construct an exact integer endpoint. For dates/times the integer is in
    /// the column's physical unit. Negative bounds on unsigned columns are valid.
    pub fn from_i64(data_type: &DataType, value: i64) -> Result<Self> {
        match LogicalType::from_arrow(data_type) {
            LogicalType::Integer { .. }
            | LogicalType::Decimal { .. }
            | LogicalType::Duration { .. } => {
                Self::native_exact(data_type, value.into(), 0, value.to_string(), value as f64)
            }
            LogicalType::Float { .. } => Self::float_integer(data_type, i128::from(value)),
            LogicalType::Date | LogicalType::Time | LogicalType::Timestamp { .. } => {
                Ok(Self::new(BoundData {
                    data_type: data_type.clone(),
                    value: Value::Temporal(value),
                    display: format_temporal(data_type, value)?,
                    axis: value as f64,
                }))
            }
            _ => Err(invalid_type(data_type)),
        }
    }

    /// Construct an unsigned integer endpoint without passing through f64.
    /// Temporal physical values must still fit their signed representation.
    pub fn from_u64(data_type: &DataType, value: u64) -> Result<Self> {
        match LogicalType::from_arrow(data_type) {
            LogicalType::Integer { .. }
            | LogicalType::Decimal { .. }
            | LogicalType::Duration { .. } => Self::native_exact(
                data_type,
                i256::from_i128(i128::from(value)),
                0,
                value.to_string(),
                value as f64,
            ),
            LogicalType::Float { .. } => Self::float_integer(data_type, i128::from(value)),
            LogicalType::Date | LogicalType::Time | LogicalType::Timestamp { .. } => {
                Self::from_i64(
                    data_type,
                    i64::try_from(value).map_err(|_| invalid_type(data_type))?,
                )
            }
            _ => Err(invalid_type(data_type)),
        }
    }

    /// Construct a floating range endpoint. Only floating columns accept this
    /// constructor. Float16/32 endpoints are not narrowed: bin boundaries can
    /// lie between native values. Infinity is valid; NaN is not ordered.
    pub fn from_f64(data_type: &DataType, value: f64) -> Result<Self> {
        if !data_type.is_floating() || value.is_nan() {
            return Err(invalid_type(data_type));
        }
        // Shortest float formatting can spell an integral value with rounded
        // decimal digits (e.g. 1000000000000000128 as 1000000000000000100).
        // Text range inputs treat integers exactly, so preserve those digits
        // when restoring or exporting this native endpoint as text.
        let display = if value.fract() == 0.0 {
            format!("{value:.0}")
        } else {
            value.to_string()
        };
        Ok(Self::new(BoundData {
            data_type: data_type.clone(),
            value: Value::Float {
                value,
                lower: value,
                upper: value,
                exact: None,
            },
            display,
            axis: value,
        }))
    }

    /// Construct an exact decimal endpoint as `coefficient * 10^-scale`.
    /// The input scale need not equal the column's scale; range preparation
    /// rounds inward to the column's native coefficient grid.
    pub fn from_decimal(data_type: &DataType, coefficient: i256, scale: i8) -> Result<Self> {
        if !matches!(
            LogicalType::from_arrow(data_type),
            LogicalType::Decimal { .. }
        ) {
            return Err(invalid_type(data_type));
        }
        let axis =
            coefficient.to_f64().expect("i256 exponent fits f64") * 10f64.powi(-i32::from(scale));
        Self::native_exact(
            data_type,
            coefficient,
            scale,
            format_decimal(coefficient, scale),
            axis,
        )
    }

    /// Label to restore in a text editor. It is not used to execute a filter.
    pub fn display(&self) -> &str {
        &self.inner.display
    }

    /// Approximate histogram coordinate. Adjacent large integer/decimal values
    /// can share this coordinate while remaining distinct filter endpoints.
    pub fn axis_value(&self) -> f64 {
        self.inner.axis
    }

    /// Whether two endpoints denote the same value of the same logical type.
    /// Display spelling and approximate chart coordinates do not participate.
    pub fn same_value(&self, other: &Self) -> bool {
        self.inner.data_type == other.inner.data_type
            && compare(&self.inner.value, &other.inner.value).is_eq()
    }

    /// Complete logical type, including width, scale, unit and time zone.
    pub fn data_type(&self) -> &DataType {
        &self.inner.data_type
    }

    fn exact(data_type: &DataType, literal: Literal, display: String, axis: f64) -> Result<Self> {
        let scale = coefficient_scale(data_type)?;
        let lower = literal.coefficient(scale, true);
        let upper = literal.coefficient(scale, false);
        Ok(Self::new(BoundData {
            data_type: data_type.clone(),
            value: Value::Exact {
                literal,
                lower,
                upper,
            },
            display,
            axis,
        }))
    }

    fn native_exact(
        data_type: &DataType,
        coefficient: i256,
        input_scale: i8,
        display: String,
        axis: f64,
    ) -> Result<Self> {
        let scale = coefficient_scale(data_type)?;
        let (lower, upper) =
            rescale_coefficient(coefficient, i16::from(scale) - i16::from(input_scale));
        Ok(Self::new(BoundData {
            data_type: data_type.clone(),
            value: Value::Exact {
                literal: Literal::from_coefficient(coefficient, input_scale),
                lower,
                upper,
            },
            display,
            axis,
        }))
    }

    fn float_integer(data_type: &DataType, value: i128) -> Result<Self> {
        let rounded = value as f64;
        Ok(Self::new(BoundData {
            data_type: data_type.clone(),
            value: Value::Float {
                value: rounded,
                lower: float::integer_value_endpoint(value, rounded, true),
                upper: float::integer_value_endpoint(value, rounded, false),
                exact: Some(Literal::from_coefficient(i256::from_i128(value), 0)),
            },
            display: value.to_string(),
            axis: rounded,
        }))
    }
}

pub(crate) fn prepare_range<'a>(
    array: &'a ArrayRef,
    min: Option<&Bound>,
    max: Option<&Bound>,
) -> Result<Predicate<'a>> {
    let data_type = array.data_type();
    for bound in [min, max].into_iter().flatten() {
        if &bound.inner.data_type != data_type {
            return Err(Error::InvalidArgument(format!(
                "{} bound cannot filter {data_type}",
                bound.inner.data_type
            )));
        }
    }
    if min
        .zip(max)
        .is_some_and(|(lo, hi)| compare(&lo.inner.value, &hi.inner.value).is_gt())
    {
        return Err(Error::InvalidArgument(
            "minimum must not exceed maximum".into(),
        ));
    }
    if data_type.is_floating() {
        let lower = min.map_or(f64::NEG_INFINITY, |bound| match bound.inner.value {
            Value::Float { lower, .. } => lower,
            _ => unreachable!("bound constructors validate type"),
        });
        let upper = max.map_or(f64::INFINITY, |bound| match bound.inner.value {
            Value::Float { upper, .. } => upper,
            _ => unreachable!("bound constructors validate type"),
        });
        return Ok(float::bound_range(array, lower, upper));
    }
    let lower = min.map_or(Endpoint::Below, |bound| match bound.inner.value {
        Value::Exact { lower, .. } => lower,
        Value::Temporal(v) => Endpoint::Finite(v.into()),
        _ => unreachable!("bound constructors validate type"),
    });
    let upper = max.map_or(Endpoint::Above, |bound| match bound.inner.value {
        Value::Exact { upper, .. } => upper,
        Value::Temporal(v) => Endpoint::Finite(v.into()),
        _ => unreachable!("bound constructors validate type"),
    });
    Ok(match LogicalType::from_arrow(data_type) {
        LogicalType::Integer { .. } => integer::bind(array, lower, upper),
        LogicalType::Decimal { .. } => super::scalar::decimal::bind(array, lower, upper),
        LogicalType::Date
        | LogicalType::Time
        | LogicalType::Timestamp { .. }
        | LogicalType::Duration { .. } => temporal::bind(array, lower, upper),
        _ => return Err(invalid_type(data_type)),
    })
}

fn coefficient_scale(data_type: &DataType) -> Result<i8> {
    match data_type {
        DataType::Decimal32(_, s)
        | DataType::Decimal64(_, s)
        | DataType::Decimal128(_, s)
        | DataType::Decimal256(_, s) => Ok(*s),
        _ if matches!(
            LogicalType::from_arrow(data_type),
            LogicalType::Integer { .. } | LogicalType::Duration { .. }
        ) =>
        {
            Ok(0)
        }
        _ => Err(invalid_type(data_type)),
    }
}

/// Inward endpoints for an exact native coefficient. Native controls never
/// route numerical values through the text parser, even when scales differ.
fn rescale_coefficient(coefficient: i256, power: i16) -> (Endpoint, Endpoint) {
    if coefficient == i256::ZERO {
        return (Endpoint::Finite(i256::ZERO), Endpoint::Finite(i256::ZERO));
    }
    let ten = i256::from(10);
    if power >= 0 {
        let mut value = coefficient;
        for _ in 0..power {
            let Some(next) = value.checked_mul(ten) else {
                let outside = if coefficient < i256::ZERO {
                    Endpoint::Below
                } else {
                    Endpoint::Above
                };
                return (outside, outside);
            };
            value = next;
        }
        return (Endpoint::Finite(value), Endpoint::Finite(value));
    }
    let mut divisor = i256::ONE;
    for _ in power..0 {
        let Some(next) = divisor.checked_mul(ten) else {
            return if coefficient < i256::ZERO {
                (
                    Endpoint::Finite(i256::ZERO),
                    Endpoint::Finite(i256::MINUS_ONE),
                )
            } else {
                (Endpoint::Finite(i256::ONE), Endpoint::Finite(i256::ZERO))
            };
        };
        divisor = next;
    }
    let (whole, remainder) = coefficient
        .checked_div_rem(divisor)
        .expect("positive divisor");
    let lower = if remainder > i256::ZERO {
        whole + i256::ONE
    } else {
        whole
    };
    let upper = if remainder < i256::ZERO {
        whole - i256::ONE
    } else {
        whole
    };
    (Endpoint::Finite(lower), Endpoint::Finite(upper))
}

fn compare(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Exact { literal: left, .. }, Value::Exact { literal: right, .. }) => {
            left.compare(right)
        }
        (Value::Temporal(left), Value::Temporal(right)) => left.cmp(right),
        (
            Value::Float {
                value: left,
                exact: a,
                ..
            },
            Value::Float {
                value: right,
                exact: b,
                ..
            },
        ) => match (a, b) {
            (Some(a), Some(b)) => a.compare(b),
            (Some(_), None) if right.is_infinite() => {
                if right.is_sign_positive() {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            (None, Some(_)) if left.is_infinite() => {
                if left.is_sign_positive() {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            _ => left
                .partial_cmp(right)
                .expect("bound constructors reject NaN"),
        },
        _ => unreachable!("matching datatype implies matching bound representation"),
    }
}

fn invalid_type(data_type: &DataType) -> Error {
    Error::InvalidArgument(format!(
        "value is not a valid range endpoint for {data_type}"
    ))
}

fn invalid_text(data_type: &DataType, text: &str) -> Error {
    Error::InvalidArgument(format!(
        "cannot read {text:?} as a range endpoint for {data_type}"
    ))
}

fn format_temporal(data_type: &DataType, value: i64) -> Result<String> {
    let raw: ArrayRef = if matches!(data_type, DataType::Date32 | DataType::Time32(_)) {
        let value = i32::try_from(value).map_err(|_| invalid_type(data_type))?;
        Arc::new(Int32Array::from(vec![value]))
    } else {
        Arc::new(Int64Array::from(vec![value]))
    };
    let typed = cast(&raw, data_type)?;
    let formatter = format::formatter(typed.as_ref())?;
    Ok(formatter.value(0).to_string())
}

fn format_decimal(coefficient: i256, scale: i8) -> String {
    let text = coefficient.to_string();
    let (sign, digits) = text
        .strip_prefix('-')
        .map_or(("", text.as_str()), |digits| ("-", digits));
    if scale <= 0 {
        return format!("{text}{}", "0".repeat((-i16::from(scale)) as usize));
    }
    let scale = scale as usize;
    if scale < digits.len() {
        let split = digits.len() - scale;
        format!("{sign}{}.{}", &digits[..split], &digits[split..])
    } else {
        format!("{sign}0.{}{digits}", "0".repeat(scale - digits.len()))
    }
}

#[cfg(test)]
mod tests {
    use arrow::{
        array::{
            Decimal128Array, Decimal256Array, Float32Array, Float64Array, Int8Array,
            TimestampNanosecondArray, TimestampSecondArray, UInt8Array,
        },
        datatypes::TimeUnit,
    };

    use super::*;
    use crate::engine::CancelToken;

    #[test]
    fn native_float_labels_roundtrip_through_exact_integer_text_parsing() {
        for value in [
            1.0000000000000001e18,
            -1.0000000000000001e18,
            u64::MAX as f64,
            f64::MAX,
            -f64::MAX,
            0.1,
            f64::MIN_POSITIVE,
        ] {
            let array: ArrayRef = Arc::new(Float64Array::from(vec![
                value.next_down(),
                value,
                value.next_up(),
            ]));
            let native = Bound::from_f64(array.data_type(), value).unwrap();
            let parsed = Bound::parse(array.data_type(), native.display()).unwrap();
            assert_eq!(matches(&array, Some(&parsed), Some(&parsed)), [
                false, true, false
            ]);
        }
    }

    fn matches(array: &ArrayRef, min: Option<&Bound>, max: Option<&Bound>) -> Vec<bool> {
        let mut out = vec![false; array.len()];
        prepare_range(array, min, max)
            .unwrap()
            .evaluate(0..array.len(), &mut out, &CancelToken::never())
            .unwrap();
        out
    }

    #[test]
    fn native_constructors_validate_kind_without_narrowing_integer_bounds() {
        let array: ArrayRef = Arc::new(Int8Array::from(vec![-128, 0, 127]));
        let bound = Bound::from_i64(&DataType::Int8, 128).unwrap();
        assert_eq!(matches(&array, Some(&bound), None), [false; 3]);
        let array: ArrayRef = Arc::new(UInt8Array::from(vec![0, 255]));
        let below = Bound::from_i64(&DataType::UInt8, -1).unwrap();
        assert_eq!(matches(&array, Some(&below), None), [true; 2]);
        assert!(Bound::from_i64(&DataType::Boolean, 1).is_err());
        assert!(Bound::from_f64(&DataType::Int64, 1.0).is_err());
        assert!(Bound::from_f64(&DataType::Float64, f64::NAN).is_err());
        assert!(Bound::from_decimal(&DataType::Int64, i256::ONE, 0).is_err());
        assert!(Bound::parse(&DataType::Float64, "NaN").is_err());
    }

    #[test]
    fn decimal256_bound_roundtrips_and_execution_ignores_its_display_label() {
        let value: i256 = "123456789012345678901234567890123456789012345678901234567890"
            .parse()
            .unwrap();
        let data_type = DataType::Decimal256(76, 2);
        let array: ArrayRef = Arc::new(
            Decimal256Array::from(vec![value, value + i256::ONE])
                .with_precision_and_scale(76, 2)
                .unwrap(),
        );
        let mut bound = Bound::from_decimal(&data_type, value, 2).unwrap();
        let reparsed = Bound::parse(&data_type, bound.display()).unwrap();
        assert_eq!(matches(&array, Some(&bound), Some(&bound)), [true, false]);
        assert_eq!(matches(&array, Some(&reparsed), Some(&reparsed)), [
            true, false
        ]);
        Arc::make_mut(&mut bound.inner).display = "this is no longer a number".into();
        assert_eq!(matches(&array, Some(&bound), Some(&bound)), [true, false]);
        assert_eq!(bound.data_type(), &data_type);

        let data_type = DataType::Decimal128(20, -2);
        let array: ArrayRef = Arc::new(
            Decimal128Array::from(vec![12, 13])
                .with_precision_and_scale(20, -2)
                .unwrap(),
        );
        let bound = Bound::from_decimal(&data_type, i256::from(12), -2).unwrap();
        assert_eq!(bound.display(), "1200");
        assert_eq!(matches(&array, Some(&bound), Some(&bound)), [true, false]);
    }

    #[test]
    fn native_decimal_rescaling_matches_text_at_extreme_scales() {
        for coefficient in [i256::ZERO, i256::ONE, i256::MINUS_ONE, i256::MAX, i256::MIN] {
            for input_scale in [-128, -76, -2, 0, 2, 76, 127] {
                for column_scale in [-76, 0, 76] {
                    let data_type = DataType::Decimal256(76, column_scale);
                    let native = Bound::from_decimal(&data_type, coefficient, input_scale).unwrap();
                    let parsed = Bound::parse(&data_type, native.display()).unwrap();
                    let (
                        Value::Exact {
                            lower: a, upper: b, ..
                        },
                        Value::Exact {
                            lower: c, upper: d, ..
                        },
                    ) = (&native.inner.value, &parsed.inner.value)
                    else {
                        unreachable!()
                    };
                    assert_eq!(
                        (a, b),
                        (c, d),
                        "{coefficient} at scale {input_scale} to {column_scale}"
                    );
                }
            }
        }
        let original = Bound::from_i64(&DataType::Int64, 42).unwrap();
        let cloned = original.clone();
        assert!(Arc::ptr_eq(&original.inner, &cloned.inner));
    }

    #[test]
    fn bounds_are_tied_to_full_logical_type() {
        let array: ArrayRef = Arc::new(Int8Array::from(vec![1]));
        let wider = Bound::from_i64(&DataType::Int64, 1).unwrap();
        assert!(prepare_range(&array, Some(&wider), None).is_err());
        let array: ArrayRef = Arc::new(
            Decimal128Array::from(vec![100])
                .with_precision_and_scale(20, 2)
                .unwrap(),
        );
        let other_scale =
            Bound::from_decimal(&DataType::Decimal128(20, 3), i256::from(1), 0).unwrap();
        assert!(prepare_range(&array, Some(&other_scale), None).is_err());
        let array: ArrayRef = Arc::new(TimestampSecondArray::from(vec![0]).with_timezone("UTC"));
        for data_type in [
            DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            DataType::Timestamp(TimeUnit::Second, Some("Europe/London".into())),
        ] {
            let other = Bound::from_i64(&data_type, 0).unwrap();
            assert!(prepare_range(&array, Some(&other), None).is_err());
        }
    }

    #[test]
    fn native_float_bounds_preserve_integer_endpoints_and_between_value_cuts() {
        let top = u64::MAX as f64;
        let array: ArrayRef = Arc::new(Float64Array::from(vec![top.next_down(), top]));
        let bound = Bound::from_u64(&DataType::Float64, u64::MAX).unwrap();
        assert_eq!(matches(&array, None, Some(&bound)), [true, false]);
        assert_eq!(matches(&array, Some(&bound), Some(&bound)), [false; 2]);
        let midpoint = (f64::from(1.0_f32) + f64::from(1.0_f32.next_up())) / 2.0;
        let array: ArrayRef = Arc::new(Float32Array::from(vec![1.0, 1.0_f32.next_up()]));
        let cut = Bound::from_f64(&DataType::Float32, midpoint).unwrap();
        assert_eq!(cut.axis_value(), midpoint);
        assert_eq!(matches(&array, Some(&cut), None), [false, true]);
        assert_eq!(matches(&array, None, Some(&cut)), [true, false]);
    }

    #[test]
    fn temporal_native_bound_formats_and_roundtrips_without_losing_nanoseconds() {
        let data_type = DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into()));
        let value = 9_007_199_254_740_993;
        let array: ArrayRef =
            Arc::new(TimestampNanosecondArray::from(vec![value - 1, value]).with_timezone("UTC"));
        let native = Bound::from_i64(&data_type, value).unwrap();
        let parsed = Bound::parse(&data_type, native.display()).unwrap();
        assert_eq!(native.axis_value(), (value - 1) as f64);
        assert_eq!(matches(&array, Some(&native), Some(&native)), [false, true]);
        assert_eq!(matches(&array, Some(&parsed), Some(&parsed)), [false, true]);
        assert_eq!(
            Bound::from_i64(&DataType::Date32, 1).unwrap().display(),
            "1970-01-02"
        );
    }

    #[test]
    fn reversed_bounds_are_rejected_before_axis_rounding() {
        let data_type = DataType::Decimal128(38, 0);
        let array: ArrayRef = Arc::new(
            Decimal128Array::from(vec![0])
                .with_precision_and_scale(38, 0)
                .unwrap(),
        );
        let lo = Bound::parse(&data_type, "9007199254740993.2").unwrap();
        let hi = Bound::parse(&data_type, "9007199254740993.1").unwrap();
        assert_eq!(lo.axis_value(), hi.axis_value());
        assert!(prepare_range(&array, Some(&lo), Some(&hi)).is_err());
        let array: ArrayRef = Arc::new(Float64Array::from(vec![f64::INFINITY]));
        let lo = Bound::from_f64(&DataType::Float64, f64::INFINITY).unwrap();
        let hi = Bound::parse(&DataType::Float64, "1e999").unwrap();
        assert!(prepare_range(&array, Some(&lo), Some(&hi)).is_err());
    }
}
