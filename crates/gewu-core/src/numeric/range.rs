//! Parse scalar inputs and prepare native range predicates before scanning rows.

use std::{hash::Hash, sync::Arc};

use arrow::{
    array::{Array, ArrayRef, ArrowNativeTypeOp, StringArray},
    compute::cast,
    datatypes::ArrowPrimitiveType,
};

use super::{IntegerColumn, Numeric, RealColumn, RealType};
use crate::{Bound, Error, Result};

pub(crate) type RowTest<'a> = Box<dyn Fn(usize) -> bool + Send + Sync + 'a>;

impl Numeric<'_> {
    pub(crate) fn parse_bound(self, text: &str) -> Result<Bound> {
        let data_type = self.0.data_type();
        let text = text.trim();
        let bound = if data_type.is_integer() {
            let signed = text.parse::<i64>().ok().map(Bound::Int);
            let unsigned = text.parse::<u64>().ok().map(Bound::UInt);
            let integer = if data_type.is_unsigned_integer() {
                unsigned.or(signed)
            } else {
                signed.or(unsigned)
            };
            integer.or_else(|| text.parse::<f64>().ok().map(Bound::Float))
        } else if data_type.is_floating() {
            text.parse::<f64>().ok().map(Bound::Float)
        } else {
            // Temporal parsing must respect the column's unit and time zone.
            // Only this one scalar is cast, never the stored column.
            let input: ArrayRef = Arc::new(StringArray::from(vec![text]));
            let typed = cast(&input, data_type)?;
            Numeric::new(&typed).and_then(|n| n.bound(0))
        };
        bound.ok_or_else(|| Error::InvalidArgument(format!("cannot read {text:?} as {data_type}")))
    }
}

impl<'a, T: ArrowPrimitiveType> IntegerColumn<'a, T>
where
    T::Native: Into<i128> + TryFrom<i128> + Eq + Hash,
{
    pub(super) fn range_test(self, min: Option<Bound>, max: Option<Bound>) -> RowTest<'a> {
        // i128 contains the entire domain of every supported integer and
        // temporal array. Even saturated float -> i128 conversions remain
        // outside those domains, so impossible bounds cannot become matches.
        let endpoint = |bound: Option<Bound>, lower: bool| -> Option<i128> {
            Some(match bound {
                None if lower => i128::MIN,
                None => i128::MAX,
                Some(Bound::Int(v)) => v.into(),
                Some(Bound::UInt(v)) => v.into(),
                Some(Bound::Float(v)) if v.is_nan() => return None,
                Some(Bound::Float(v)) if lower => v.ceil() as i128,
                Some(Bound::Float(v)) => v.floor() as i128,
            })
        };
        let (Some(lo), Some(hi)) = (endpoint(min, true), endpoint(max, false)) else {
            return Box::new(|_| false);
        };
        let lo = lo.max(T::Native::MIN_TOTAL_ORDER.into());
        let hi = hi.min(T::Native::MAX_TOTAL_ORDER.into());
        if lo > hi {
            return Box::new(|_| false);
        }
        let (Ok(lo), Ok(hi)) = (T::Native::try_from(lo), T::Native::try_from(hi)) else {
            unreachable!("bounds intersected with the native type domain");
        };
        let array = self.0;
        Box::new(move |r| array.is_valid(r) && (lo..=hi).contains(&array.value(r)))
    }
}

impl<'a, T: RealType> RealColumn<'a, T> {
    pub(super) fn range_test(self, min: Option<Bound>, max: Option<Bound>) -> RowTest<'a> {
        // Decimal bounds still use the public f64 approximation. Inward
        // rounding applies only to truly floating-point columns: applying it
        // to rounded decimal coefficients could exclude an equal bound.
        let endpoint = if self.array.data_type().is_floating() {
            float_endpoint
        } else {
            rounded_endpoint
        };
        let lo = endpoint(min, true);
        let hi = endpoint(max, false);
        let array = self.array;
        let scale = self.scale;
        Box::new(move |r| {
            if array.is_null(r) {
                return false;
            }
            let value = T::as_float(array.value(r), scale);
            value >= lo && value <= hi
        })
    }
}

fn rounded_endpoint(bound: Option<Bound>, lower: bool) -> f64 {
    match bound {
        None if lower => f64::NEG_INFINITY,
        None => f64::INFINITY,
        Some(Bound::Int(v)) => v as f64,
        Some(Bound::UInt(v)) => v as f64,
        Some(Bound::Float(v)) => v,
    }
}

// Integer bounds on floating arrays round inward, so an integer not exactly
// representable as f64 cannot accidentally admit the adjacent floating value.
fn float_endpoint(bound: Option<Bound>, lower: bool) -> f64 {
    let value = match bound {
        None => {
            return if lower {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }
        }
        Some(Bound::Float(v)) => return v,
        Some(Bound::Int(v)) => i128::from(v),
        Some(Bound::UInt(v)) => i128::from(v),
    };
    let float = value as f64;
    if lower && (float as i128) < value {
        float.next_up()
    } else if !lower && (float as i128) > value {
        float.next_down()
    } else {
        float
    }
}
