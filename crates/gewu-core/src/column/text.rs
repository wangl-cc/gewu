//! Text predicates prepare their pattern once and scan only execution batches.

use std::fmt::Write;

use arrow::{
    array::{Array, ArrayAccessor, ArrayRef, AsArray},
    datatypes::DataType,
};
use regex::RegexBuilder;

use crate::{format, predicate::Predicate, Error, Result};

pub(super) fn contains<'a>(
    array: &'a ArrayRef,
    text: &str,
    case_sensitive: bool,
) -> Result<Predicate<'a>> {
    if matches!(
        array.data_type(),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
    ) {
        if case_sensitive {
            let text = text.to_owned();
            return Ok(strings(array, move |value| value.contains(&text)));
        }
        // Arrow's public ilike kernel compiles a new regex on every call.
        // Prepare the same Unicode literal matching once, without scanning
        // the column for ASCII or building a mask for every batch.
        let matcher = RegexBuilder::new(&regex::escape(text))
            .case_insensitive(true)
            .build()
            .map_err(|e| Error::InvalidArgument(format!("cannot prepare text search: {e}")))?;
        return Ok(strings(array, move |value| matcher.is_match(value)));
    }

    // A formatted fallback retains existing Contains semantics for non-ordered
    // values. One buffer per batch, reused across cells, never a column of strings.
    let text = text.to_owned();
    let lower = (!case_sensitive && !text.is_ascii()).then(|| text.to_lowercase());
    Ok(Predicate::batch(Box::new(move |rows, hits, _| {
        let f = format::formatter(array.as_ref())?;
        let mut buf = String::new();
        for (r, hit) in rows.zip(hits) {
            if array.is_null(r) {
                continue;
            }
            buf.clear();
            write!(buf, "{}", f.value(r)).expect("writing to String cannot fail");
            *hit = if case_sensitive {
                buf.contains(&text)
            } else if let Some(lower) = &lower {
                buf.to_lowercase().contains(lower)
            } else {
                text.is_empty()
                    || buf
                        .as_bytes()
                        .windows(text.len())
                        .any(|w| w.eq_ignore_ascii_case(text.as_bytes()))
            };
        }
        Ok(())
    })))
}

pub(super) fn equals<'a>(array: &'a ArrayRef, text: &str) -> Predicate<'a> {
    let text = text.to_owned();
    strings(array, move |value| value == text)
}

fn strings<'a>(
    array: &'a ArrayRef,
    test: impl Fn(&str) -> bool + Send + Sync + 'a,
) -> Predicate<'a> {
    match array.data_type() {
        DataType::Utf8 => bind(array.as_string::<i32>(), test),
        DataType::LargeUtf8 => bind(array.as_string::<i64>(), test),
        DataType::Utf8View => bind(array.as_string_view(), test),
        _ => unreachable!("plain string dispatch"),
    }
}

fn bind<'a, A>(array: A, test: impl Fn(&str) -> bool + Send + Sync + 'a) -> Predicate<'a>
where
    A: ArrayAccessor<Item = &'a str> + Copy + Send + Sync + 'a,
{
    Predicate::rows(Box::new(move |row| {
        array.is_valid(row) && test(array.value(row))
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{Scalar, StringArray},
        compute::{cast, kernels::comparison::ilike},
    };

    use super::*;
    use crate::engine::CancelToken;

    #[test]
    fn prepared_matching_preserves_arrows_unicode_and_literal_semantics() {
        let input: ArrayRef = Arc::new(StringArray::from(vec![
            Some("A%_\\z"),
            Some("Kilo"),
            Some("ſ"),
            Some("Σσς"),
            Some("straße"),
            Some("ﬀ"),
            Some("line\nBreak"),
            Some(""),
            None,
        ]));
        for data_type in [DataType::Utf8, DataType::LargeUtf8, DataType::Utf8View] {
            let array = cast(&input, &data_type).unwrap();
            for query in ["%_\\z", "k", "S", "σ", "SS", "ff", "\n", "", "[.*]"] {
                let escaped = query
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_");
                let pattern: ArrayRef = Arc::new(StringArray::from(vec![format!("%{escaped}%")]));
                let pattern = cast(&pattern, &data_type).unwrap();
                let expected = ilike(&array, &Scalar::new(pattern)).unwrap();
                let test = contains(&array, query, false).unwrap();
                let mut actual = vec![false; array.len()];
                test.evaluate(0..array.len(), &mut actual, &CancelToken::never())
                    .unwrap();
                assert_eq!(
                    actual,
                    expected.iter().map(|v| v == Some(true)).collect::<Vec<_>>(),
                    "{data_type}: {query:?}"
                );
            }
        }
    }
}
