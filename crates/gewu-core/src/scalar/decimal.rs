//! Exact decimal values and native endpoints.

use std::cmp::Ordering;

use arrow::datatypes::i256;

/// Signed digits times 10^exponent. Trailing digits are retained because they
/// specify search precision; leading zeros do not affect that precision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Literal {
    negative: bool,
    digits: String,
    exponent: i64,
}

impl Literal {
    pub(crate) fn filter_text(&self) -> String {
        if self.digits.is_empty() {
            return "0".into();
        }
        format!(
            "{}{}e{}",
            if self.negative { "-" } else { "" },
            self.digits,
            self.exponent
        )
    }

    /// Integer and duration searches accept signed digits only, including
    /// out-of-domain values, without decimal points or exponents.
    pub(crate) fn parse_integer(text: &str) -> Option<Self> {
        let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Self::parse(text)
    }

    /// Construct a parsed decimal directly from a native coefficient. Formatting
    /// its digits is lossless; this does not round-trip through a text parser.
    pub(crate) fn from_coefficient(coefficient: i256, scale: i8) -> Self {
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

    pub(crate) fn parse(text: &str) -> Option<Self> {
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

    pub(crate) fn compare(&self, other: &Self) -> Ordering {
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
    pub(crate) fn search_interval(&self) -> Option<(Self, Self)> {
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
    pub(crate) fn coefficient(&self, scale: i8, ceil: bool) -> Endpoint {
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
pub(crate) enum Endpoint {
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

    pub(crate) fn preceding(self) -> Self {
        match self {
            Self::Finite(v) => v.checked_sub(i256::ONE).map_or(Self::Below, Self::Finite),
            other => other,
        }
    }
}
