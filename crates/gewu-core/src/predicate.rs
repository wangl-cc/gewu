//! Row tests on one column, evaluated into a mask of rows.
//!
//! Every filter and every column of a search becomes a [`ColumnTest`]. A test
//! marks the rows it accepts in a `&mut [bool]`; filters are combined with
//! AND, the columns of a search with OR. Nothing is cached after a call.

use std::{fmt::Write, sync::Arc};

use arrow::{
    array::{Array, ArrayRef, AsArray, BooleanArray, Datum, LargeStringArray, Scalar, StringArray},
    compute::{
        cast,
        kernels::comparison::{ilike, like},
    },
    datatypes::{DataType, Float64Type},
};

use crate::{
    engine::CancelToken, format, numeric::Numeric, value_index::ValueIndex, Bound, Error, Result,
};

/// How many rows to process between cancellation checks.
const CHUNK: usize = 64 * 1024;

pub(crate) enum ColumnTest {
    /// The value's code is allowed (checkbox filter). Codes index the whole
    /// column, so this only works on the source column, not on a slice.
    Codes {
        index: Arc<ValueIndex>,
        allowed: Vec<bool>,
    },
    /// Null (or not null with `negate`).
    Null { negate: bool },
    /// The displayed text contains `text`.
    Text { text: String, case_sensitive: bool },
    /// `min <= value <= max` on an ordered column; a missing end is open.
    Range {
        min: Option<Bound>,
        max: Option<Bound>,
    },
    /// A boolean equal to this.
    Bool(bool),
    /// A floating-point NaN.
    NaN,
}

impl ColumnTest {
    /// Mark rows of `array` this test accepts. Rows already marked stay
    /// marked, so calling several tests ORs them. Nulls only ever pass
    /// [`ColumnTest::Null`].
    pub fn mark(&self, array: &ArrayRef, hits: &mut [bool], token: &CancelToken) -> Result<()> {
        debug_assert_eq!(array.len(), hits.len());
        match self {
            Self::Codes { index, allowed } => {
                if index.codes.len() != array.len() {
                    return Err(Error::InvalidArgument(
                        "a value filter needs the whole column".into(),
                    ));
                }
                mark(hits, token, |r| allowed[index.codes[r] as usize])
            }
            Self::Null { negate } => mark(hits, token, |r| array.is_null(r) != *negate),
            Self::Text {
                text,
                case_sensitive,
            } => mark_text(array, text, *case_sensitive, hits, token),
            Self::Range { min, max } => {
                let test = Numeric::new(array)
                    .ok_or_else(|| {
                        Error::InvalidArgument(format!(
                            "a range needs numbers, dates or times, not {}",
                            array.data_type()
                        ))
                    })?
                    .range_test(*min, *max);
                mark(hits, token, test)
            }
            Self::Bool(target) => {
                let a = array.as_boolean_opt().ok_or_else(|| {
                    Error::InvalidArgument(format!("{} is not boolean", array.data_type()))
                })?;
                mark(hits, token, |r| a.is_valid(r) && a.value(r) == *target)
            }
            Self::NaN => {
                if !array.data_type().is_floating() {
                    return Ok(());
                }
                let values = cast(array, &DataType::Float64)?;
                let a = values.as_primitive::<Float64Type>();
                mark(hits, token, |r| a.is_valid(r) && a.value(r).is_nan())
            }
        }
    }
}

/// Mark rows passing `test`, checking for cancellation between chunks.
fn mark(hits: &mut [bool], token: &CancelToken, test: impl Fn(usize) -> bool) -> Result<()> {
    let len = hits.len();
    for start in (0..len).step_by(CHUNK) {
        token.check()?;
        let end = (start + CHUNK).min(len);
        for (r, hit) in (start..end).zip(&mut hits[start..end]) {
            if !*hit && test(r) {
                *hit = true;
            }
        }
    }
    Ok(())
}

/// Substring of the displayed text: Arrow's `like` kernels for plain
/// strings, otherwise each cell formatted into one reused buffer.
fn mark_text(
    array: &ArrayRef,
    text: &str,
    case_sensitive: bool,
    hits: &mut [bool],
    token: &CancelToken,
) -> Result<()> {
    if matches!(
        array.data_type(),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
    ) {
        let mask = string_kernel(array, text, case_sensitive)?;
        for (hit, m) in hits.iter_mut().zip(mask.iter()) {
            *hit |= m == Some(true);
        }
        return Ok(());
    }

    let lower = text.to_lowercase();
    let ascii = text.is_ascii();
    let f = format::formatter(array.as_ref())?;
    let mut buf = String::new();
    for start in (0..array.len()).step_by(CHUNK) {
        token.check()?;
        let end = (start + CHUNK).min(array.len());
        for (r, hit) in (start..end).zip(&mut hits[start..end]) {
            if *hit || array.is_null(r) {
                continue;
            }
            buf.clear();
            let _ = write!(buf, "{}", f.value(r));
            *hit = if case_sensitive {
                buf.contains(text)
            } else if ascii {
                contains_ignore_ascii_case(buf.as_bytes(), text.as_bytes())
            } else {
                buf.to_lowercase().contains(&lower)
            };
        }
    }
    Ok(())
}

/// `like`/`ilike` with the text as a literal substring.
fn string_kernel(array: &ArrayRef, text: &str, case_sensitive: bool) -> Result<BooleanArray> {
    let pattern = format!("%{}%", escape_like(text));
    let scalar: Box<dyn Datum> = match array.data_type() {
        DataType::LargeUtf8 => Box::new(Scalar::new(LargeStringArray::from(vec![pattern]))),
        DataType::Utf8View => Box::new(Scalar::new(arrow::array::StringViewArray::from(vec![
            pattern,
        ]))),
        _ => Box::new(Scalar::new(StringArray::from(vec![pattern]))),
    };
    Ok(if case_sensitive {
        like(array, scalar.as_ref())?
    } else {
        ilike(array, scalar.as_ref())?
    })
}

/// Escape `%`, `_` and `\` so they match literally.
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn contains_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len()
        && haystack
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle))
}
