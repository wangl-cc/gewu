//! Find predicate compilation. Type dispatch and text parsing stay inside
//! this operation; callers provide arrays and consume compiled predicates.

use arrow::{array::ArrayRef, datatypes::DataType};

use super::{nested, scalar, text};
use crate::{execution::predicate::Predicate, Result};

pub(crate) fn compile<'a>(
    array: &'a ArrayRef,
    query: &str,
    matcher: Option<&regex::Regex>,
) -> Result<Option<Predicate<'a>>> {
    if query.is_empty() {
        return Ok(None);
    }
    match array.data_type() {
        DataType::Dictionary(..) => {
            nested::dictionary_find(array, |values| compile(values, query, matcher))
        }
        DataType::List(_)
        | DataType::LargeList(_)
        | DataType::ListView(_)
        | DataType::LargeListView(_)
        | DataType::FixedSizeList(..)
        | DataType::Struct(_) => nested::search(array, query, matcher),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
            text::contains(array, query, false, matcher).map(Some)
        }
        DataType::Boolean
        | DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64
        | DataType::Float16
        | DataType::Float32
        | DataType::Float64
        | DataType::Decimal32(..)
        | DataType::Decimal64(..)
        | DataType::Decimal128(..)
        | DataType::Decimal256(..)
        | DataType::Date32
        | DataType::Date64
        | DataType::Time32(_)
        | DataType::Time64(_)
        | DataType::Timestamp(..)
        | DataType::Duration(_) => scalar::search(array, query),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{new_null_array, Int32Array, StringArray, StructArray},
        datatypes::{Field, Fields},
    };

    use super::*;
    use crate::Cancellation;

    #[test]
    fn unsearchable_children_produce_no_predicate() {
        let field = Arc::new(Field::new("item", DataType::Binary, true));
        for data_type in [
            DataType::List(field.clone()),
            DataType::Struct(Fields::from(vec![field])),
            DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::Binary)),
        ] {
            let array = new_null_array(&data_type, 2);
            assert!(
                compile(&array, "needle", None).unwrap().is_none(),
                "{data_type}"
            );
        }
    }

    #[test]
    fn nested_find_parses_each_child_and_skips_inapplicable_text() {
        let array: ArrayRef = Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new("number", DataType::Int32, false)),
                Arc::new(Int32Array::from(vec![42, 7, 8])) as ArrayRef,
            ),
            (
                Arc::new(Field::new("text", DataType::Utf8, false)),
                Arc::new(StringArray::from(vec!["other", "42", "needle"])) as ArrayRef,
            ),
            (
                Arc::new(Field::new("binary", DataType::Binary, true)),
                new_null_array(&DataType::Binary, 3),
            ),
        ]));
        for (query, expected) in [
            ("42", [true, true, false]),
            ("needle", [false, false, true]),
        ] {
            let predicate = compile(&array, query, None).unwrap().unwrap();
            let mut matches = [false; 3];
            predicate
                .evaluate(0..3, &mut matches, &Cancellation::default())
                .unwrap();
            assert_eq!(matches, expected);
        }
    }
}
