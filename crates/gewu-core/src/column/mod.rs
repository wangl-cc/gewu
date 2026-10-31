//! A borrowed column interprets user input using its complete Arrow type.
//!
//! Logical semantics live here; Arrow arrays retain physical storage. Preparing
//! a predicate never scans the column. Execution and cancellation live outside
//! the type parsers; value indexes remain owned by the document.

pub(crate) mod bound;
pub(crate) mod histogram;
mod nested;
mod scalar;
pub(crate) mod text;

use std::sync::{Arc, OnceLock};

use arrow::{
    array::{Array, ArrayRef},
    datatypes::DataType,
    util::display::ArrayFormatter,
};
use regex::RegexBuilder;

use crate::{
    format, predicate::Predicate, value_index::ValueIndex, Error, Filter, LogicalType, Result,
};

/// A Find query prepared once per captured session. The parsed literal matcher
/// is shared by every column, so a page or highlight never recompiles it.
pub(crate) struct SearchQuery {
    text: String,
    matcher: OnceLock<std::result::Result<Arc<regex::Regex>, String>>,
}

impl SearchQuery {
    pub(crate) fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            matcher: OnceLock::new(),
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// The case-insensitive literal matcher, compiled at most once.
    pub(crate) fn matcher(&self) -> Result<Arc<regex::Regex>> {
        let matcher = self.matcher.get_or_init(|| {
            RegexBuilder::new(&regex::escape(&self.text))
                .case_insensitive(true)
                .build()
                .map(Arc::new)
                .map_err(|error| error.to_string())
        });
        match matcher {
            Ok(matcher) => Ok(matcher.clone()),
            Err(error) => Err(Error::InvalidArgument(format!(
                "cannot prepare text search: {error}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub logical_type: LogicalType,
    pub display_type: String,
    pub technical_type: String,
    pub nullable: bool,
}

/// Operations supported by a column's logical type. Value-list availability
/// additionally depends on cardinality and is reported by `DistinctResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnCapabilities {
    pub contains: bool,
    pub range: bool,
    pub equals: bool,
    /// Histograms currently require a directly stored numeric array.
    pub histogram: bool,
    /// Direct numeric and temporal arrays support approximate plot coordinates.
    pub scatter: bool,
    /// Direct numeric values support positive-only logarithmic charts.
    pub logarithmic: bool,
}

/// Maximum source cardinality offered as a checkbox list.
pub(crate) const CHOICE_LIMIT: usize = 500;

#[derive(Clone, Copy)]
pub(crate) enum FilterEditorKind {
    Range,
    Choices,
    Contains,
    NullOnly,
}

#[derive(Clone, Copy)]
pub(crate) struct Column<'a> {
    array: &'a ArrayRef,
}

impl<'a> Column<'a> {
    pub(crate) fn new(array: &'a ArrayRef) -> Self {
        Self { array }
    }

    pub(crate) fn capabilities(self) -> ColumnCapabilities {
        fn of(data_type: &DataType) -> ColumnCapabilities {
            if let DataType::Dictionary(_, values) = data_type {
                return ColumnCapabilities {
                    histogram: false,
                    scatter: false,
                    logarithmic: false,
                    ..of(values)
                };
            }
            let logical_type = LogicalType::from_arrow(data_type);
            ColumnCapabilities {
                contains: !logical_type.is_ordered() && logical_type != LogicalType::Boolean,
                range: logical_type.is_ordered(),
                histogram: logical_type.is_ordered(),
                scatter: logical_type.is_ordered(),
                logarithmic: logical_type.supports_logarithmic(),
                equals: logical_type.is_ordered()
                    || matches!(logical_type, LogicalType::Boolean | LogicalType::Text),
            }
        }
        of(self.array.data_type())
    }

    pub(crate) fn logical_data_type(self) -> &'a DataType {
        let mut data_type = self.array.data_type();
        while let DataType::Dictionary(_, values) = data_type {
            data_type = values;
        }
        data_type
    }

    pub(crate) fn filter_editor(self, low_cardinality: bool) -> FilterEditorKind {
        if low_cardinality {
            FilterEditorKind::Choices
        } else if self.capabilities().range {
            FilterEditorKind::Range
        } else if self.capabilities().contains {
            FilterEditorKind::Contains
        } else {
            FilterEditorKind::NullOnly
        }
    }

    pub(crate) fn formatter(self) -> Result<ArrayFormatter<'a>> {
        format::formatter(self.array.as_ref())
    }

    pub(crate) fn prepare_search(self, query: &SearchQuery) -> Result<Option<Predicate<'a>>> {
        if query.text().is_empty() {
            return Ok(None);
        }
        match self.array.data_type() {
            DataType::Dictionary(..)
            | DataType::List(_)
            | DataType::LargeList(_)
            | DataType::FixedSizeList(..)
            | DataType::ListView(_)
            | DataType::LargeListView(_)
            | DataType::Struct(_) => nested::search(self.array, query),
            DataType::Binary
            | DataType::LargeBinary
            | DataType::BinaryView
            | DataType::FixedSizeBinary(_) => Ok(None),
            data_type
                if LogicalType::from_arrow(data_type).is_ordered()
                    || *data_type == DataType::Boolean =>
            {
                scalar::search(self.array, query.text())
            }
            _ => text::contains(self.array, query, false).map(Some),
        }
    }

    /// Type-dependent filters. InValues is prepared separately once the
    /// document has obtained its lazy value index.
    pub(crate) fn prepare_filter(self, filter: &Filter) -> Result<Predicate<'a>> {
        let predicate = self.prepare_value_filter(filter)?;
        let include_nulls = matches!(
            filter,
            Filter::Range {
                include_nulls: true,
                ..
            } | Filter::Contains {
                include_nulls: true,
                ..
            }
        );
        let array = self.array;
        if !include_nulls || array.null_count() == 0 {
            return Ok(predicate);
        }
        Ok(match predicate {
            Predicate::Rows(test) => {
                Predicate::rows(Box::new(move |row| array.is_null(row) || test(row)))
            }
            predicate => Predicate::batch(Box::new(move |rows, hits, cancel| {
                predicate.evaluate_selected(rows.clone(), hits, cancel)?;
                for (index, row) in rows.iter().enumerate() {
                    hits[index] |= array.is_null(row);
                }
                Ok(())
            })),
        })
    }

    fn prepare_value_filter(self, filter: &Filter) -> Result<Predicate<'a>> {
        if let Filter::TextValues {
            values, exclude, ..
        } = filter
        {
            return text::selected_values(self.array, values, *exclude);
        }
        if let Filter::IsNull { negate, .. } = filter {
            let negate = *negate;
            let array = self.array;
            return Ok(Predicate::rows(Box::new(move |row| {
                array.is_null(row) != negate
            })));
        }
        if matches!(self.array.data_type(), DataType::Dictionary(..)) {
            return nested::dictionary(self.array, |values| {
                Column::new(values).prepare_filter(filter).map(Some)
            })?
            .ok_or_else(|| {
                Error::InvalidArgument("filter cannot be applied to dictionary values".into())
            });
        }
        let capabilities = self.capabilities();
        match filter {
            Filter::Contains {
                text: value,
                case_sensitive,
                ..
            } if capabilities.contains => {
                text::contains(self.array, &SearchQuery::new(value), *case_sensitive)
            }
            Filter::Range { min, max, .. } if capabilities.range => {
                bound::prepare_range(self.array, min.as_ref(), max.as_ref())
            }
            Filter::Equals { value, .. } if capabilities.equals => self.prepare_equals(value),
            Filter::EqualsAny { values, .. } if capabilities.equals => {
                let predicates = values
                    .iter()
                    .map(|value| self.prepare_equals(value))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Predicate::any_of(predicates))
            }
            _ => Err(Error::InvalidArgument(format!(
                "filter is not supported for {}",
                self.array.data_type()
            ))),
        }
    }

    fn prepare_equals(self, value: &str) -> Result<Predicate<'a>> {
        if LogicalType::from_arrow(self.array.data_type()) == LogicalType::Text {
            Ok(text::equals(self.array, value))
        } else {
            scalar::equals(self.array, value)
        }
    }

    pub(crate) fn prepare_values(self, index: Arc<ValueIndex>, keys: &[u64]) -> Predicate<'a> {
        index.prepare_predicate(keys)
    }
}

#[cfg(test)]
mod tests {
    use arrow::{
        array::{
            DictionaryArray, FixedSizeListArray, Int32Array, Int8Array, ListArray, StringArray,
            StructArray,
        },
        buffer::NullBuffer,
        datatypes::{Field, Int32Type, Int8Type},
    };

    use super::*;
    use crate::{
        engine::{CancelToken, ViewEngine},
        predicate::CHUNK,
    };

    fn matches(array: &ArrayRef, query: &str, range: std::ops::Range<usize>) -> Vec<bool> {
        let mut hits = vec![false; range.len()];
        if let Some(test) = Column::new(array)
            .prepare_search(&SearchQuery::new(query))
            .unwrap()
        {
            test.evaluate(range, &mut hits, &CancelToken::never())
                .unwrap();
        }
        hits
    }

    #[test]
    fn typed_membership_validates_every_input_and_empty_lists_match_nothing() {
        let array: ArrayRef = Arc::new(Int32Array::from(vec![Some(1), None, Some(2)]));
        let column = Column::new(&array);
        let test = column
            .prepare_filter(&Filter::EqualsAny {
                column: 0,
                values: Vec::new(),
            })
            .unwrap();
        let mut hits = [true; 3];
        test.evaluate(0..3, &mut hits, &CancelToken::never())
            .unwrap();
        assert_eq!(hits, [false; 3]);
        assert!(matches!(
            column.prepare_filter(&Filter::EqualsAny {
                column: 0,
                values: vec!["1".into(), "invalid".into()],
            }),
            Err(Error::InvalidArgument(_))
        ));

        let array: ArrayRef = Arc::new(
            DictionaryArray::<Int8Type>::try_new(
                Int8Array::from(vec![Some(0), None, Some(1), Some(2)]),
                Arc::new(StringArray::from(vec![Some("01"), None, Some("2")])),
            )
            .unwrap(),
        );
        let test = Column::new(&array)
            .prepare_filter(&Filter::EqualsAny {
                column: 0,
                values: vec!["01".into(), "2".into()],
            })
            .unwrap();
        let mut hits = [false; 4];
        test.evaluate(0..4, &mut hits, &CancelToken::never())
            .unwrap();
        assert_eq!(hits, [true, false, false, true]);
    }

    #[test]
    fn text_patterns_are_literal_and_batches_keep_source_offsets() {
        let array: ArrayRef = Arc::new(StringArray::from(vec![
            Some("prefix"),
            Some("A%_\\z"),
            None,
            Some("a%_\\Z trailing"),
        ]));
        assert_eq!(matches(&array, "%_\\z", 1..4), [true, false, true]);
        let test = Column::new(&array)
            .prepare_filter(&Filter::Equals {
                column: 0,
                value: "A%_\\z".into(),
            })
            .unwrap();
        let mut hits = vec![false; 3];
        test.evaluate(1..4, &mut hits, &CancelToken::never())
            .unwrap();
        assert_eq!(hits, [true, false, false]);
    }

    #[test]
    fn included_nulls_cover_dictionary_keys_values_and_nested_batches() {
        let dictionary: ArrayRef = Arc::new(
            DictionaryArray::<Int8Type>::try_new(
                Int8Array::from(vec![Some(0), None, Some(1), Some(2), Some(0)]),
                Arc::new(StringArray::from(vec![Some("keep"), None, Some("skip")])),
            )
            .unwrap(),
        );
        let list: ArrayRef = Arc::new(ListArray::from_iter_primitive::<Int32Type, _, _>([
            Some(vec![Some(12)]),
            None,
            Some(vec![]),
            Some(vec![Some(12)]),
            Some(vec![Some(34)]),
        ]));
        for (array, text, excluded, included) in [
            (dictionary, "keep", [false, false, false, true], [
                true, true, false, true,
            ]),
            (list, "12", [false, false, true, false], [
                true, false, true, false,
            ]),
        ] {
            for (include_nulls, expected) in [(false, excluded), (true, included)] {
                let test = Column::new(&array)
                    .prepare_filter(&Filter::Contains {
                        column: 0,
                        text: text.into(),
                        case_sensitive: true,
                        include_nulls,
                    })
                    .unwrap();
                let mut hits = [false; 4];
                test.evaluate(1..5, &mut hits, &CancelToken::never())
                    .unwrap();
                assert_eq!(hits, expected);
            }
        }
    }

    #[test]
    fn empty_contains_fallback_matches_non_null_without_panicking() {
        let array: ArrayRef = Arc::new(ListArray::from_iter_primitive::<Int32Type, _, _>([
            Some(vec![Some(12)]),
            None,
            Some(vec![]),
        ]));
        let test = Column::new(&array)
            .prepare_filter(&Filter::Contains {
                include_nulls: false,
                column: 0,
                text: String::new(),
                case_sensitive: false,
            })
            .unwrap();
        let mut hits = vec![false; 3];
        test.evaluate(0..3, &mut hits, &CancelToken::never())
            .unwrap();
        assert_eq!(hits, [true, false, true]);
    }

    #[test]
    fn list_search_uses_child_type_with_nulls_slices_and_large_children() {
        let array: ArrayRef = Arc::new(ListArray::from_iter_primitive::<Int32Type, _, _>([
            Some(vec![Some(25)]),
            Some(vec![Some(2)]),
            None,
            Some(vec![]),
            Some(vec![None]),
        ]));
        assert_eq!(matches(&array, "2", 1..5), [true, false, false, false]);
        assert_eq!(matches(&array.slice(1, 3), "2", 0..3), [true, false, false]);
        let mut big = vec![Some(1); CHUNK + 7];
        big[CHUNK + 6] = Some(2);
        let array: ArrayRef = Arc::new(ListArray::from_iter_primitive::<Int32Type, _, _>([
            Some(big),
            Some(vec![Some(25)]),
            Some(vec![Some(2)]),
        ]));
        assert_eq!(matches(&array, "2", 0..3), [true, false, true]);
    }

    #[test]
    fn fixed_lists_search_typed_values_and_respect_slices() {
        let array: ArrayRef = Arc::new(FixedSizeListArray::from_iter_primitive::<Int32Type, _, _>(
            [Some(vec![Some(25)]), Some(vec![Some(2)]), None],
            1,
        ));
        assert_eq!(matches(&array, "2", 0..3), [false, true, false]);
        assert_eq!(matches(&array.slice(1, 2), "2", 0..2), [true, false]);
    }

    #[test]
    fn struct_parent_null_masks_matching_children() {
        let fields = vec![
            Arc::new(Field::new("number", DataType::Int32, false)),
            Arc::new(Field::new("text", DataType::Utf8, false)),
        ];
        let array: ArrayRef = Arc::new(StructArray::new(
            fields.into(),
            vec![
                Arc::new(Int32Array::from(vec![2, 25, 2])),
                Arc::new(StringArray::from(vec!["no", "2", "2"])),
            ],
            Some(NullBuffer::from(vec![true, true, false])),
        ));
        assert_eq!(matches(&array, "2", 0..3), [true, true, false]);
    }

    #[test]
    fn dictionary_search_and_filter_keep_typed_values_and_key_offsets() {
        let array: ArrayRef = Arc::new(
            DictionaryArray::<Int8Type>::try_new(
                Int8Array::from(vec![Some(0), Some(1), None, Some(1)]),
                Arc::new(Int32Array::from(vec![25, 2])),
            )
            .unwrap(),
        );
        assert_eq!(matches(&array, "2", 1..4), [true, false, true]);
        let test = Column::new(&array)
            .prepare_filter(&Filter::Equals {
                column: 0,
                value: "2".into(),
            })
            .unwrap();
        let mut first = [false; 2];
        let mut second = [false; 2];
        test.evaluate(0..2, &mut first, &CancelToken::never())
            .unwrap();
        test.evaluate(2..4, &mut second, &CancelToken::never())
            .unwrap();
        assert_eq!(first, [false, true]);
        assert_eq!(second, [false, true]);
    }

    #[test]
    fn list_views_allow_overlapping_and_out_of_order_children() {
        use arrow::{array::ListViewArray, buffer::ScalarBuffer};
        let array: ArrayRef = Arc::new(ListViewArray::new(
            Arc::new(Field::new("item", DataType::Int32, true)),
            ScalarBuffer::from(vec![2_i32, 0, 1, 0]),
            ScalarBuffer::from(vec![1_i32, 2, 2, 0]),
            Arc::new(Int32Array::from(vec![25, 2, 30])),
            Some(NullBuffer::from(vec![true, true, false, true])),
        ));
        assert_eq!(matches(&array, "2", 0..4), [false, true, false, false]);
        assert_eq!(matches(&array.slice(1, 3), "2", 0..3), [true, false, false]);
    }

    #[test]
    fn dictionary_range_support_does_not_claim_histogram_support() {
        let array: ArrayRef = Arc::new(
            DictionaryArray::<Int8Type>::try_new(
                Int8Array::from(vec![0]),
                Arc::new(Int32Array::from(vec![2])),
            )
            .unwrap(),
        );
        assert_eq!(Column::new(&array).capabilities(), ColumnCapabilities {
            contains: false,
            range: true,
            equals: true,
            histogram: false,
            scatter: false,
            logarithmic: false,
        });
    }

    #[test]
    fn cancellation_is_checked_before_text_and_nested_execution() {
        let arrays: Vec<ArrayRef> = vec![
            Arc::new(StringArray::from(vec!["hello"])),
            Arc::new(ListArray::from_iter_primitive::<Int32Type, _, _>([Some(
                vec![Some(2)],
            )])),
        ];
        let engine = ViewEngine::new(1);
        let token = engine.begin();
        engine.cancel_pending();
        for array in arrays {
            let test = Column::new(&array)
                .prepare_search(&SearchQuery::new("2"))
                .unwrap()
                .unwrap();
            assert!(matches!(
                test.evaluate(0..1, &mut [false], &token),
                Err(Error::Cancelled)
            ));
        }
    }
}
