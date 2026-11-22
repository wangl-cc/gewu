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
