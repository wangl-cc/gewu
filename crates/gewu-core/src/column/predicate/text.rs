//! Text predicates compile their pattern once and scan only execution batches.

use std::{collections::HashSet, sync::Arc};

use arrow::{
    array::{downcast_dictionary_array, Array, ArrayAccessor, ArrayRef, AsArray},
    datatypes::{ArrowNativeType, DataType},
};
use regex::RegexBuilder;

use super::Result;
use crate::execution::{predicate::Predicate, selection::RowSelection};

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
        _ => Err(crate::FilterError::UnsupportedType {
            condition: crate::FilterKind::Text,
            data_type: array.data_type().clone(),
        }),
    }
}

fn borrow_strings<'a, A>(array: A) -> StringAccessor<'a>
where
    A: ArrayAccessor<Item = &'a str> + Copy + Send + Sync + 'a,
{
    Box::new(move |row| array.is_valid(row).then(|| array.value(row)))
}

/// Non-null membership. The column compiler owns exclusion and null policy.
pub(super) fn selected_values<'a>(
    array: &'a ArrayRef,
    values: &[String],
    cancel: &crate::Cancellation,
) -> Result<Predicate<'a>> {
    let read = accessor(array)?;
    let mut selected = HashSet::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        cancel.check_at(index)?;
        selected.insert(value.clone());
    }
    Ok(Predicate::new(Box::new(
        move |rows, incoming, output, _cancel| {
            match (rows, incoming) {
                (RowSelection::Range(range), None) => {
                    for (i, row) in range.enumerate() {
                        let included = read(row).is_some_and(|value| selected.contains(value));
                        output[i] = included;
                    }
                }
                (RowSelection::Range(range), Some(mask)) => {
                    for (i, row) in range.enumerate() {
                        if mask[i] {
                            let included = read(row).is_some_and(|value| selected.contains(value));
                            output[i] = included;
                        } else {
                            output[i] = false;
                        }
                    }
                }
                (RowSelection::Indices(indices), None) => {
                    for (i, &row) in indices.iter().enumerate() {
                        let included = read(row).is_some_and(|value| selected.contains(value));
                        output[i] = included;
                    }
                }
                (RowSelection::Indices(indices), Some(mask)) => {
                    for (i, &row) in indices.iter().enumerate() {
                        if mask[i] {
                            let included = read(row).is_some_and(|value| selected.contains(value));
                            output[i] = included;
                        } else {
                            output[i] = false;
                        }
                    }
                }
            }
            Ok(())
        },
    )))
}

pub(super) fn contains<'a>(
    array: &'a ArrayRef,
    text: &str,
    case_sensitive: bool,
    matcher: Option<&regex::Regex>,
) -> Result<Predicate<'a>> {
    if !matches!(
        array.data_type(),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
    ) {
        return Err(crate::FilterError::UnsupportedType {
            condition: crate::FilterKind::Text,
            data_type: array.data_type().clone(),
        });
    }
    if case_sensitive {
        let text = text.to_owned();
        return Ok(strings(array, move |value| value.contains(&text)));
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
    Ok(strings(array, move |value| matcher.is_match(value)))
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
            if !matches!(
                array.data_type(),
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
            ) {
                return Err(crate::FilterError::UnsupportedType {
                    condition: crate::FilterKind::Text,
                    data_type: array.data_type().clone(),
                });
            }
            let text = text.to_owned();
            if case_sensitive {
                return Ok(strings(array, move |value| value == text));
            }
            let folded = RegexBuilder::new(&format!(r"\A(?:{})\z", regex::escape(&text)))
                .case_insensitive(true)
                .build()
                .map_err(|source| crate::FilterError::PatternCompilationFailed { source })?;
            let ascii = text.is_ascii();
            // ASCII folding only applies when both sides are ASCII: under Unicode
            // folding an ASCII input still matches characters such as the Kelvin sign.
            Ok(strings(array, move |value| {
                if ascii && value.is_ascii() {
                    value.eq_ignore_ascii_case(&text)
                } else {
                    folded.is_match(value)
                }
            }))
        }
    }
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

fn bind<'a, A>(array: A, is_match: impl Fn(&str) -> bool + Send + Sync + 'a) -> Predicate<'a>
where
    A: ArrayAccessor<Item = &'a str> + Copy + Send + Sync + 'a,
{
    Predicate::new(Box::new(move |rows, incoming, output, _cancel| {
        match incoming {
            Some(mask) => {
                for (i, (&m, row)) in mask.iter().zip(rows.iter()).enumerate() {
                    output[i] = m && array.is_valid(row) && is_match(array.value(row));
                }
            }
            None => {
                for (i, row) in rows.iter().enumerate() {
                    output[i] = array.is_valid(row) && is_match(array.value(row));
                }
            }
        }
        Ok(())
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
    use crate::{column::Column, Cancellation, Filter};

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
            let test = column
                .compile_filter(&filter, &Cancellation::default())
                .unwrap();
            let mut actual = vec![false; array.len()];
            test.evaluate(0..array.len(), &mut actual, &Cancellation::default())
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
                let column = Column::new("value", array.clone()).unwrap();
                let filter = Filter::exact(vec!["a".into()], exclude, !exclude);
                let test = column
                    .compile_filter(&filter, &Cancellation::default())
                    .unwrap();
                let mut actual = vec![false; array.len()];
                test.evaluate(0..array.len(), &mut actual, &Cancellation::default())
                    .unwrap();
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
                test.evaluate(0..array.len(), &mut actual, &Cancellation::default())
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
