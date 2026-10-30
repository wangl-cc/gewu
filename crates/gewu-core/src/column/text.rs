//! Text predicates prepare their pattern once and scan only execution batches.

use std::{collections::HashSet, fmt::Write};

use arrow::{
    array::{downcast_dictionary_array, Array, ArrayAccessor, ArrayRef, AsArray},
    datatypes::{ArrowNativeType, DataType},
};
use regex::RegexBuilder;

use crate::{format, predicate::Predicate, Error, Result};

/// Borrow a string cell without decoding dictionaries or formatting a column.
/// Both a null dictionary key and a key naming a null value yield `None`.
pub(crate) type StringAccessor<'a> = Box<dyn Fn(usize) -> Option<&'a str> + Send + Sync + 'a>;

pub(crate) fn accessor(array: &ArrayRef) -> Result<StringAccessor<'_>> {
    match array.data_type() {
        DataType::Utf8 => Ok(borrow_strings(array.as_string::<i32>())),
        DataType::LargeUtf8 => Ok(borrow_strings(array.as_string::<i64>())),
        DataType::Utf8View => Ok(borrow_strings(array.as_string_view())),
        DataType::Dictionary(..) => {
            downcast_dictionary_array!(array => {
                let values = accessor(array.values())?;
                Ok(Box::new(move |row| {
                    if array.is_null(row) { None } else { values(array.keys().value(row).as_usize()) }
                }) as StringAccessor<'_>)
            }, _ => unreachable!("dictionary type checked above"))
        }
        _ => Err(Error::InvalidArgument(format!(
            "{} is not a string column",
            array.data_type()
        ))),
    }
}

fn borrow_strings<'a, A>(array: A) -> StringAccessor<'a>
where
    A: ArrayAccessor<Item = &'a str> + Copy + Send + Sync + 'a,
{
    Box::new(move |row| array.is_valid(row).then(|| array.value(row)))
}

/// Exact membership retains only the explicitly included or excluded values.
pub(super) fn selected_values<'a>(
    array: &'a ArrayRef,
    values: &[Option<String>],
    exclude: bool,
) -> Result<Predicate<'a>> {
    let read = accessor(array)?;
    let null_selected = values.iter().any(Option::is_none);
    let selected: HashSet<String> = values.iter().filter_map(Clone::clone).collect();
    Ok(Predicate::rows(Box::new(move |row| {
        let included = read(row).map_or(null_selected, |value| selected.contains(value));
        included != exclude
    })))
}

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
        array::{DictionaryArray, Int8Array, Scalar, StringArray},
        compute::{cast, kernels::comparison::ilike},
        datatypes::Int8Type,
    };

    use super::*;
    use crate::{column::Column, engine::CancelToken, Filter};

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
        for exclude in [false, true] {
            let filter = Filter::TextValues {
                column: 0,
                values: vec![Some("a".into()), None, Some("a".into()), None],
                exclude,
            };
            let test = Column::new(&array).prepare_filter(&filter).unwrap();
            let mut actual = vec![false; array.len()];
            test.evaluate(0..array.len(), &mut actual, &CancelToken::never())
                .unwrap();
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
                let test = selected_values(&array, &[Some("a".into()), None], exclude).unwrap();
                let mut actual = vec![false; array.len()];
                test.evaluate(0..array.len(), &mut actual, &CancelToken::never())
                    .unwrap();
                let expected = [true, false, true, false, false].map(|value| value != exclude);
                assert_eq!(actual, expected, "{data_type}: exclude={exclude}");
            }
        }
    }

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
