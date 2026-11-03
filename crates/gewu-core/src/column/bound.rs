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

use super::filter::scalar::{
    decimal::{Endpoint, Literal},
    float, temporal,
};
use crate::{
    format,
    types::{direct_domain, Domain},
    Error, Result,
};

/// A parsed endpoint for one column's logical type. Construct through [`Self::parse`]
/// for text inputs or a native constructor for numerical controls. Bounds are
/// not interchangeable between different units, scales, widths or time zones.
/// Cloning shares immutable parsed state; it does not copy labels or digits.
#[derive(Clone, Debug, PartialEq)]
pub struct Bound {
    inner: Arc<BoundData>,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::column) struct BoundData {
    pub(in crate::column) data_type: DataType,
    pub(in crate::column) value: Value,
    pub(in crate::column) display: String,
    pub(in crate::column) axis: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::column) enum Value {
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
    pub(in crate::column) fn new(inner: BoundData) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Parse an endpoint according to the complete column type. Integer and
    /// Decimal inputs retain exact fractional bounds and arbitrary digit lengths.
    /// Invalid inputs, NaN and unsupported types return an error.
    pub fn parse(data_type: &DataType, text: &str) -> Result<Self> {
        let text = text.trim();
        if let Some(ticks) = text.strip_prefix('@') {
            if matches!(
                direct_domain(data_type),
                Some(Domain::Date | Domain::Time | Domain::Timestamp)
            ) {
                let value = ticks
                    .parse::<i64>()
                    .map_err(|_| invalid_text(data_type, text))?;
                return Ok(Self::new(BoundData {
                    data_type: data_type.clone(),
                    value: Value::Temporal(value),
                    display: text.into(),
                    axis: value as f64,
                }));
            }
        }
        match direct_domain(data_type) {
            Some(Domain::Integer | Domain::Decimal | Domain::Duration) => {
                let literal = Literal::parse(text).ok_or_else(|| invalid_text(data_type, text))?;
                let axis = text
                    .parse::<f64>()
                    .map_err(|_| invalid_text(data_type, text))?;
                Self::exact(data_type, literal, text.to_owned(), axis)
            }
            Some(Domain::Float) => {
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
            Some(Domain::Date | Domain::Time | Domain::Timestamp) => {
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
        match direct_domain(data_type) {
            Some(Domain::Integer | Domain::Decimal | Domain::Duration) => {
                Self::native_exact(data_type, value.into(), 0, value.to_string(), value as f64)
            }
            Some(Domain::Float) => Self::float_integer(data_type, i128::from(value)),
            Some(Domain::Date | Domain::Time | Domain::Timestamp) => Ok(Self::new(BoundData {
                data_type: data_type.clone(),
                value: Value::Temporal(value),
                display: format_temporal(data_type, value)?,
                axis: value as f64,
            })),
            _ => Err(invalid_type(data_type)),
        }
    }

    /// Construct an unsigned integer endpoint without passing through f64.
    /// Temporal physical values must still fit their signed representation.
    pub fn from_u64(data_type: &DataType, value: u64) -> Result<Self> {
        match direct_domain(data_type) {
            Some(Domain::Integer | Domain::Decimal | Domain::Duration) => Self::native_exact(
                data_type,
                i256::from_i128(i128::from(value)),
                0,
                value.to_string(),
                value as f64,
            ),
            Some(Domain::Float) => Self::float_integer(data_type, i128::from(value)),
            Some(Domain::Date | Domain::Time | Domain::Timestamp) => Self::from_i64(
                data_type,
                i64::try_from(value).map_err(|_| invalid_type(data_type))?,
            ),
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
        if !matches!(direct_domain(data_type), Some(Domain::Decimal)) {
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

    /// Lossless filter protocol text, independent of display formatting.
    pub fn filter_text(&self) -> String {
        match &self.inner.value {
            Value::Exact { literal, .. } => literal.filter_text(),
            Value::Float { .. } => self.inner.display.clone(),
            Value::Temporal(ticks) => format!("@{ticks}"),
        }
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

    /// Complete logical type, including width, scale, unit and time zone.
    pub fn data_type(&self) -> &DataType {
        &self.inner.data_type
    }

    pub(in crate::column) fn compare(&self, other: &Self) -> Ordering {
        compare(&self.inner.value, &other.inner.value)
    }

    pub(in crate::column) fn float_lower(&self) -> f64 {
        match self.inner.value {
            Value::Float { lower, .. } => lower,
            _ => unreachable!("bound constructors validate type"),
        }
    }

    pub(in crate::column) fn float_upper(&self) -> f64 {
        match self.inner.value {
            Value::Float { upper, .. } => upper,
            _ => unreachable!("bound constructors validate type"),
        }
    }

    pub(in crate::column) fn exact_lower(&self) -> Endpoint {
        match self.inner.value {
            Value::Exact { lower, .. } => lower,
            Value::Temporal(v) => Endpoint::Finite(v.into()),
            _ => unreachable!("bound constructors validate type"),
        }
    }

    pub(in crate::column) fn exact_upper(&self) -> Endpoint {
        match self.inner.value {
            Value::Exact { upper, .. } => upper,
            Value::Temporal(v) => Endpoint::Finite(v.into()),
            _ => unreachable!("bound constructors validate type"),
        }
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

pub(in crate::column) fn coefficient_scale(data_type: &DataType) -> Result<i8> {
    match data_type {
        DataType::Decimal32(_, s)
        | DataType::Decimal64(_, s)
        | DataType::Decimal128(_, s)
        | DataType::Decimal256(_, s) => Ok(*s),
        _ if matches!(
            direct_domain(data_type),
            Some(Domain::Integer | Domain::Duration)
        ) =>
        {
            Ok(0)
        }
        _ => Err(invalid_type(data_type)),
    }
}

/// Inward endpoints for an exact native coefficient. Native controls never
/// route numerical values through the text parser, even when scales differ.
pub(in crate::column) fn rescale_coefficient(
    coefficient: i256,
    power: i16,
) -> (Endpoint, Endpoint) {
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

pub(in crate::column) fn compare(left: &Value, right: &Value) -> Ordering {
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

pub(in crate::column) fn invalid_type(data_type: &DataType) -> Error {
    Error::InvalidArgument(format!(
        "value is not a valid range endpoint for {data_type}"
    ))
}

pub(in crate::column) fn invalid_text(data_type: &DataType, text: &str) -> Error {
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
