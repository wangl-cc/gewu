//! Text predicates compile their pattern once and scan only execution batches.

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayAccessor, ArrayRef, AsArray},
    datatypes::DataType,
};
use rapidhash::RapidHashSet;
use regex::RegexBuilder;

use super::Result;
use crate::{
    execution::predicate::{collect_bits, Predicate},
    FilterKind,
};

/// Non-null membership over plain strings. Dictionary columns are matched per
/// distinct value by the column compiler, which also owns exclusion and null policy.
pub(super) fn selected_values<'a>(array: &'a ArrayRef, values: &[String]) -> Result<Predicate<'a>> {
    let selected: RapidHashSet<String> = values.iter().cloned().collect();
    strings(array, FilterKind::Exact, move |value| {
        selected.contains(value)
    })
}

pub(super) fn contains<'a>(
    array: &'a ArrayRef,
    text: &str,
    case_sensitive: bool,
    matcher: Option<&regex::Regex>,
) -> Result<Predicate<'a>> {
    if case_sensitive {
        let text = text.to_owned();
        return strings(array, FilterKind::Text, move |value| value.contains(&text));
    }
    // If a session-wide matcher was provided, reuse it; otherwise compile one.
    let matcher = match matcher {
        Some(m) => Arc::new(m.clone()),
        None => Arc::new(
            RegexBuilder::new(&regex::escape(text))
                .case_insensitive(true)
                .build()
                .map_err(|source| crate::FilterError::PatternCompilationFailed { source })?,
        ),
    };
    if !text.is_ascii() {
        return strings(array, FilterKind::Text, move |value| {
            matcher.is_match(value)
        });
    }
    // Unicode simple case folding only maps non-ASCII characters (such as the
    // Kelvin sign) onto ASCII letters, so ASCII values match an ASCII needle
    // under ASCII case folding exactly. Those characters take at least two
    // bytes, so no value shorter than the needle can match.
    let needle = text.to_ascii_lowercase().into_bytes();
    strings(array, FilterKind::Text, move |value| {
        if value.len() < needle.len() {
            false
        } else if value.is_ascii() {
            ascii_contains_ignore_case(value.as_bytes(), &needle)
        } else {
            matcher.is_match(value)
        }
    })
}

/// Whether `haystack` contains `needle` (already ASCII-lowercased), ignoring ASCII case.
fn ascii_contains_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    let Some((&first, rest)) = needle.split_first() else {
        return true;
    };
    haystack.windows(needle.len()).any(|window| {
        window[0].to_ascii_lowercase() == first && window[1..].eq_ignore_ascii_case(rest)
    })
}

/// Explicit text filter. Case-insensitive equality agrees with case-insensitive
/// Contains: both use Unicode simple case folding.
pub(super) fn text_match<'a>(
    array: &'a ArrayRef,
    text: &str,
    mode: crate::TextMatch,
    case_sensitive: bool,
) -> Result<Predicate<'a>> {
    use crate::TextMatch;
    match mode {
        TextMatch::Contains => contains(array, text, case_sensitive, None),
        TextMatch::Equals => {
            let text = text.to_owned();
            if case_sensitive {
                return strings(array, FilterKind::Text, move |value| value == text);
            }
            let folded = RegexBuilder::new(&format!(r"\A(?:{})\z", regex::escape(&text)))
                .case_insensitive(true)
                .build()
                .map_err(|source| crate::FilterError::PatternCompilationFailed { source })?;
            let ascii = text.is_ascii();
            // ASCII folding only applies when both sides are ASCII: under Unicode
            // folding an ASCII input still matches characters such as the Kelvin sign.
            strings(array, FilterKind::Text, move |value| {
                if ascii && value.is_ascii() {
                    value.eq_ignore_ascii_case(&text)
                } else {
                    folded.is_match(value)
                }
            })
        }
    }
}

fn strings<'a>(
    array: &'a ArrayRef,
    condition: FilterKind,
    test: impl Fn(&str) -> bool + Send + Sync + 'a,
) -> Result<Predicate<'a>> {
    Ok(match array.data_type() {
        DataType::Utf8 => bind(array.as_string::<i32>(), test),
        DataType::LargeUtf8 => bind(array.as_string::<i64>(), test),
        DataType::Utf8View => bind(array.as_string_view(), test),
        data_type => {
            return Err(crate::FilterError::UnsupportedType {
                condition,
                data_type: data_type.clone(),
            })
        }
    })
}

fn bind<'a, A>(array: A, is_match: impl Fn(&str) -> bool + Send + Sync + 'a) -> Predicate<'a>
where
    A: ArrayAccessor<Item = &'a str> + Copy + Send + Sync + 'a,
{
    let is_match = Arc::new(is_match);
    let test = is_match.clone();
    Predicate::new(Box::new(move |rows, output| {
        for (i, row) in rows.iter().enumerate() {
            output[i] = array.is_valid(row) && is_match(array.value(row));
        }
    }))
    .with_bitmap(Box::new(move |rows| {
        if array.null_count() == 0 {
            return collect_bits(rows, |row| test(array.value(row)));
        }
        collect_bits(rows, |row| array.is_valid(row) && test(array.value(row)))
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{DictionaryArray, Int8Array, Scalar, StringArray},
        compute::{cast, kernels::comparison::ilike},
        datatypes::Int8Type,
    };

    use super::*;
    use crate::{column::Column, Filter};

    #[test]
    fn dictionary_membership_combines_nulls_and_duplicate_values() {
        let values: ArrayRef = Arc::new(StringArray::from(vec![
            Some("a"),
            None,
            Some("a"),
            Some("A"),
        ]));
        let keys = Int8Array::from(vec![Some(0), Some(1), None, Some(2), Some(3), Some(0)]);
        let array: ArrayRef = Arc::new(DictionaryArray::<Int8Type>::try_new(keys, values).unwrap());
        let column = Column::new("test", array.clone()).unwrap();
        for exclude in [false, true] {
            let filter = Filter::exact(vec!["a".into(), "a".into()], exclude, !exclude);
            let test = column.compile_filter(&filter).unwrap();
            let mut actual = vec![false; array.len()];
            test.evaluate(0..array.len(), &mut actual);
            assert_eq!(
                actual,
                [true, true, true, true, false, true].map(|v| v != exclude)
            );
        }
    }

    #[test]
    fn selected_values_borrow_each_native_string_representation() {
        let input: ArrayRef = Arc::new(StringArray::from(vec![
            Some("a"),
            Some("A"),
            None,
            Some(""),
            Some("(null)"),
        ]));
        for data_type in [DataType::Utf8, DataType::LargeUtf8, DataType::Utf8View] {
            let array = cast(&input, &data_type).unwrap();
            for exclude in [false, true] {
                let column = Column::new("value", array.clone()).unwrap();
                let filter = Filter::exact(vec!["a".into()], exclude, !exclude);
                let test = column.compile_filter(&filter).unwrap();
                let mut actual = vec![false; array.len()];
                test.evaluate(0..array.len(), &mut actual);
                let expected = [true, false, true, false, false].map(|value| value != exclude);
                assert_eq!(actual, expected, "{data_type}: exclude={exclude}");
            }
        }
    }

    #[test]
    fn compiled_matching_preserves_arrows_unicode_and_literal_semantics() {
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
                let test = contains(&array, query, false, None).unwrap();
                let mut actual = vec![false; array.len()];
                test.evaluate(0..array.len(), &mut actual);
                assert_eq!(
                    actual,
                    expected.iter().map(|v| v == Some(true)).collect::<Vec<_>>(),
                    "{data_type}: {query:?}"
                );
            }
        }
    }
}
