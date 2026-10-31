//! Exact decimal inputs. Only query-sized strings are allocated; stored
//! coefficients stay in their original native arrays.

use std::cmp::Ordering;

use arrow::{
    array::{Array, ArrayRef, AsArray, PrimitiveArray},
    datatypes::*,
};

use crate::predicate::{Predicate, RowSelection};

/// Signed digits times 10^exponent. Trailing digits are retained because they
/// specify search precision; leading zeros do not affect that precision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::column) struct Literal {
    negative: bool,
    digits: String,
    exponent: i64,
}

impl Literal {
    /// Integer and duration searches accept signed digits only, including
    /// out-of-domain values, without decimal points or exponents.
    pub(in crate::column) fn parse_integer(text: &str) -> Option<Self> {
        let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Self::parse(text)
    }

    /// Construct a parsed decimal directly from a native coefficient. Formatting
    /// its digits is lossless; this does not round-trip through a text parser.
    pub(in crate::column) fn from_coefficient(coefficient: i256, scale: i8) -> Self {
        let text = coefficient.to_string();
        let negative = text.starts_with('-');
        let digits = text
            .trim_start_matches('-')
            .trim_start_matches('0')
            .to_owned();
        Self {
            negative: negative && !digits.is_empty(),
            digits,
            exponent: -i64::from(scale),
        }
    }

    pub(in crate::column) fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (negative, text) = match text.as_bytes().first()? {
            b'-' => (true, &text[1..]),
            b'+' => (false, &text[1..]),
            _ => (false, text),
        };
        let (mantissa, exponent) = match text.split_once(['e', 'E']) {
            Some((m, e)) => (m, e.parse::<i64>().ok()?),
            None => (text, 0),
        };
        let mut digits = String::with_capacity(mantissa.len());
        let mut fraction = None::<i64>;
        for b in mantissa.bytes() {
            match b {
                b'0'..=b'9' => {
                    digits.push(char::from(b));
                    if let Some(n) = &mut fraction {
                        *n += 1;
                    }
                }
                b'.' if fraction.is_none() => fraction = Some(0),
                _ => return None,
            }
        }
        if digits.is_empty() {
            return None;
        }
        let exponent = exponent.checked_sub(fraction.unwrap_or(0))?;
        let digits = digits.trim_start_matches('0');
        Some(Self {
            negative: negative && !digits.is_empty(),
            digits: digits.to_owned(),
            exponent,
        })
    }

    pub(in crate::column) fn compare(&self, other: &Self) -> Ordering {
        if self.digits.is_empty() && other.digits.is_empty() {
            return Ordering::Equal;
        }
        if self.negative != other.negative {
            return other.negative.cmp(&self.negative);
        }
        let magnitude = if self.digits.is_empty() {
            Ordering::Less
        } else if other.digits.is_empty() {
            Ordering::Greater
        } else {
            let left = self.digits.len() as i128 + i128::from(self.exponent);
            let right = other.digits.len() as i128 + i128::from(other.exponent);
            left.cmp(&right).then_with(|| {
                let n = self.digits.len().max(other.digits.len());
                self.digits
                    .bytes()
                    .chain(std::iter::repeat(b'0'))
                    .take(n)
                    .cmp(other.digits.bytes().chain(std::iter::repeat(b'0')).take(n))
            })
        };
        if self.negative {
            magnitude.reverse()
        } else {
            magnitude
        }
    }

    /// The half-unit interval implied by the last typed digit.
    fn search_interval(&self) -> Option<(Self, Self)> {
        let exponent = self.exponent.checked_sub(1)?;
        if self.digits.is_empty() {
            return Some((
                Self {
                    negative: true,
                    digits: "5".into(),
                    exponent,
                },
                Self {
                    negative: false,
                    digits: "5".into(),
                    exponent,
                },
            ));
        }
        let outer = format!("{}5", self.digits);
        let mut inner = self.digits.as_bytes().to_vec();
        // Subtract one from the nonzero magnitude, then append five:
        // (10 * coefficient - 5) is exact even for arbitrarily long inputs.
        for digit in inner.iter_mut().rev() {
            if *digit != b'0' {
                *digit -= 1;
                break;
            }
            *digit = b'9';
        }
        inner.push(b'5');
        let inner = String::from_utf8(inner).expect("ASCII decimal digits");
        let inner = inner.trim_start_matches('0').to_owned();
        let make = |digits| Self {
            negative: self.negative,
            digits,
            exponent,
        };
        Some(if self.negative {
            (make(outer), make(inner))
        } else {
            (make(inner), make(outer))
        })
    }

    /// Round a decimal to a coefficient at `scale`. Overflow remains outside
    /// the domain, rather than saturating to a potentially matching endpoint.
    pub(in crate::column) fn coefficient(&self, scale: i8, ceil: bool) -> Endpoint {
        if self.digits.is_empty() {
            return Endpoint::Finite(i256::ZERO);
        }
        let integer_len = self.digits.len() as i128 + i128::from(self.exponent) + i128::from(scale);
        let mut integer = String::with_capacity(79);
        if self.negative {
            integer.push('-');
        }
        let fractional = if integer_len <= 0 {
            integer.push('0');
            true
        } else if integer_len > 77 {
            return Endpoint::infinity(self.negative);
        } else {
            let integer_len = integer_len as usize;
            let take = integer_len.min(self.digits.len());
            integer.push_str(&self.digits[..take]);
            integer.extend(std::iter::repeat_n('0', integer_len - take));
            self.digits.as_bytes()[take..].iter().any(|&b| b != b'0')
        };
        let Ok(mut value) = integer.parse::<i256>() else {
            return Endpoint::infinity(self.negative);
        };
        if fractional && ceil != self.negative {
            let next = if self.negative {
                value.checked_sub(i256::ONE)
            } else {
                value.checked_add(i256::ONE)
            };
            let Some(next) = next else {
                return Endpoint::infinity(self.negative);
            };
            value = next;
        }
        Endpoint::Finite(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::column) enum Endpoint {
    Below,
    Finite(i256),
    Above,
}

impl Endpoint {
    fn infinity(negative: bool) -> Self {
        if negative {
            Self::Below
        } else {
            Self::Above
        }
    }

    fn preceding(self) -> Self {
        match self {
            Self::Finite(v) => v.checked_sub(i256::ONE).map_or(Self::Below, Self::Finite),
            other => other,
        }
    }
}

pub(in crate::column) trait Coefficient:
    Copy + PartialOrd + Send + Sync + 'static
{
    const MIN: i256;
    const MAX: i256;
    fn from_wide(value: i256) -> Self;
}

macro_rules! coefficient {
    ($($t:ty),*) => {$ (
        impl Coefficient for $t {
            const MIN: i256 = i256::from_i128(<$t>::MIN as i128);
            const MAX: i256 = i256::from_i128(<$t>::MAX as i128);
            fn from_wide(value: i256) -> Self { value.to_i128().expect("clamped to native range") as Self }
        }
    )*};
}
coefficient!(i8, i16, i32, i64, i128, u8, u16, u32, u64);
impl Coefficient for i256 {
    const MAX: i256 = i256::MAX;
    const MIN: i256 = i256::MIN;

    fn from_wide(value: i256) -> Self {
        value
    }
}

pub(in crate::column) fn native_range<'a, T: ArrowPrimitiveType>(
    array: &'a PrimitiveArray<T>,
    lo: Endpoint,
    hi: Endpoint,
) -> Predicate<'a>
where
    T::Native: Coefficient,
{
    let lo = lo.max(Endpoint::Finite(<T::Native as Coefficient>::MIN));
    let hi = hi.min(Endpoint::Finite(<T::Native as Coefficient>::MAX));
    if lo > hi {
        return Predicate::empty();
    }
    let (Endpoint::Finite(lo), Endpoint::Finite(hi)) = (lo, hi) else {
        unreachable!("native domain intersection");
    };
    let (lo, hi) = (T::Native::from_wide(lo), T::Native::from_wide(hi));
    let values = array.values();
    let has_nulls = array.null_count() > 0;

    Predicate::new(Box::new(move |rows, incoming, output, _cancel| {
        match (rows, incoming) {
            (RowSelection::Range(range), None) => {
                let slice = &values[range.clone()];
                for (i, &v) in slice.iter().enumerate() {
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
            (RowSelection::Range(range), Some(mask)) => {
                let slice = &values[range.clone()];
                for (i, &v) in slice.iter().enumerate() {
                    output[i] = (v >= lo && v <= hi) & mask[i];
                }
                if has_nulls {
                    for (i, row) in range.enumerate() {
                        if array.is_null(row) {
                            output[i] = false;
                        }
                    }
                }
            }
            (RowSelection::Indices(indices), None) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = (!has_nulls || array.is_valid(row))
                        && values[row] >= lo
                        && values[row] <= hi;
                }
            }
            (RowSelection::Indices(indices), Some(mask)) => {
                for (i, &row) in indices.iter().enumerate() {
                    output[i] = mask[i]
                        && (!has_nulls || array.is_valid(row))
                        && values[row] >= lo
                        && values[row] <= hi;
                }
            }
        }
        Ok(())
    }))
}

fn scale(array: &ArrayRef) -> i8 {
    match array.data_type() {
        DataType::Decimal32(_, s)
        | DataType::Decimal64(_, s)
        | DataType::Decimal128(_, s)
        | DataType::Decimal256(_, s) => *s,
        _ => unreachable!("decimal dispatch"),
    }
}

pub(in crate::column) fn bind(array: &ArrayRef, lo: Endpoint, hi: Endpoint) -> Predicate<'_> {
    match array.data_type() {
        DataType::Decimal32(..) => native_range(array.as_primitive::<Decimal32Type>(), lo, hi),
        DataType::Decimal64(..) => native_range(array.as_primitive::<Decimal64Type>(), lo, hi),
        DataType::Decimal128(..) => native_range(array.as_primitive::<Decimal128Type>(), lo, hi),
        DataType::Decimal256(..) => native_range(array.as_primitive::<Decimal256Type>(), lo, hi),
        _ => unreachable!("decimal dispatch"),
    }
}

pub(in crate::column) fn search<'a>(array: &'a ArrayRef, text: &str) -> Option<Predicate<'a>> {
    let (lo, hi) = Literal::parse(text)?.search_interval()?;
    let scale = scale(array);
    Some(bind(
        array,
        lo.coefficient(scale, true),
        hi.coefficient(scale, true).preceding(),
    ))
}
