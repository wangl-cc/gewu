//! First-class persistent Column model and column-level operations.
//!
//! Type semantics live in [`crate::types`]; this module owns the column's data,
//! its lazy distinct-value dictionary, and the per-column kernels:
//! - [`dictionary`]: compact distinct-value dictionary encoding
//! - [`filter`]: Single-column filter and Find predicate compilation

pub mod bound;
pub mod dictionary;
pub mod filter;
mod metadata;
pub(crate) mod value_text;

use std::sync::{Arc, OnceLock};

use arrow::{array::ArrayRef, datatypes::DataType};
pub use bound::Bound;
pub(crate) use dictionary::Dictionary;
pub(crate) use metadata::display_type;
pub use metadata::{ColumnInfo, ValueCategory};

use crate::{predicate::Predicate, scope::Selection, types, Filter, Result};

/// Operations supported by a column's type. Value-list availability
/// additionally depends on cardinality and is reported by `DistinctResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnCapabilities {
    pub contains: bool,
    pub range: bool,
    pub exact: bool,
    /// Histograms currently require a directly stored numeric array.
    pub histogram: bool,
    /// Direct numeric and temporal arrays support approximate plot coordinates.
    pub scatter: bool,
    /// Direct numeric values support positive-only logarithmic charts.
    pub logarithmic: bool,
}

/// A single distinct value in a column or summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistinctValue {
    /// Identifier within this distinct-value report.
    /// Codes `0..=254` name non-null values and `255` names null.
    pub key: u64,
    /// Displayed text; `None` is the null value.
    pub label: Option<String>,
    pub count: u64,
}

/// Distinct values of a column, with total count and truncation state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistinctResult {
    /// Sorted by value, nulls first.
    pub values: Vec<DistinctValue>,
    /// Number of distinct values before `limit` was applied.
    pub total: u64,
    pub truncated: bool,
    /// Whether this report comes from the bounded dictionary. False for
    /// columns with too many distinct values to encode; `values` then holds
    /// the most common ones.
    pub keyed: bool,
}

/// An immutable, first-class column owned by a [`crate::Dataset`].
pub struct Column {
    pub name: String,
    pub nullable: bool,
    pub len: usize,
    pub array: ArrayRef,
    /// Lazy compact encoding of the column's distinct values. `None` is
    /// cached when the type cannot be encoded or has too many values.
    dictionary: OnceLock<Option<Arc<Dictionary>>>,
}

impl Column {
    /// Build a column. Fails for Arrow types without Gewu semantics rather
    /// than exposing a column no operation can describe.
    pub fn new(name: impl Into<String>, array: ArrayRef) -> Result<Self> {
        let name = name.into();
        if !types::is_describable(array.data_type()) {
            return Err(crate::Error::Unsupported(format!(
                "column {name:?} has unsupported type {}",
                array.data_type()
            )));
        }
        let len = array.len();
        let nullable = array.is_nullable();
        Ok(Self {
            name,
            nullable,
            len,
            array,
            dictionary: OnceLock::new(),
        })
    }

    /// The column's compact distinct-value encoding, built once and shared by
    /// every session over the same dataset. Columns that are too high
    /// cardinality (or whose type cannot be encoded) return `None`.
    pub(crate) fn dictionary(&self) -> Option<&Arc<Dictionary>> {
        self.dictionary
            .get_or_init(|| Dictionary::from_array(&self.array).map(Arc::new))
            .as_ref()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    pub fn array(&self) -> ArrayRef {
        self.array.clone()
    }

    #[inline]
    pub fn is_nullable(&self) -> bool {
        self.nullable
    }

    #[inline]
    pub fn capabilities(&self) -> ColumnCapabilities {
        let data_type = self.array.data_type();
        ColumnCapabilities {
            contains: filter::supports_contains(data_type),
            range: filter::range::supports(data_type),
            exact: filter::supports_equals(data_type),
            histogram: crate::stats::supports_histogram(data_type),
            scatter: crate::stats::scatter::supports(data_type),
            logarithmic: crate::stats::supports_logarithmic(data_type),
        }
    }

    pub fn logical_data_type(&self) -> &DataType {
        types::value_type(self.array.data_type())
    }

    pub fn info(&self) -> ColumnInfo {
        ColumnInfo::from_column(self)
    }

    #[cfg(test)]
    pub(crate) fn prepare_find<'a>(
        &'a self,
        query: &str,
        matcher: Option<&regex::Regex>,
    ) -> Result<Option<Predicate<'a>>> {
        filter::prepare_find(&self.array, query, matcher)
    }

    /// Precomputed choices with counts, or `None` if the column is not dictionary-encoded.
    pub(crate) fn choices(
        &self,
        rows: &Selection,
        include_zero: bool,
    ) -> Option<Vec<DistinctValue>> {
        self.dictionary()
            .map(|d| d.distinct_values(rows, include_zero))
    }

    /// Distinct values of this column with their counts.
    /// Fast path: uses dictionary encoding if available.
    /// Slow path: counts values via streaming scan.
    pub(crate) fn distinct_values(&self, rows: &Selection, limit: usize) -> Result<DistinctResult> {
        if let Some(dictionary) = self.dictionary() {
            let choices = dictionary.distinct_values(rows, false);
            let total = choices.len() as u64;
            Ok(DistinctResult {
                total,
                truncated: total > limit as u64,
                values: choices.into_iter().take(limit).collect(),
                keyed: true,
            })
        } else {
            let counted = crate::stats::ArrayView::new(&self.array, rows).count_values(limit)?;
            Ok(DistinctResult {
                total: counted.distinct,
                truncated: counted.distinct > limit as u64,
                values: counted.top.into_iter().take(limit).collect(),
                keyed: false,
            })
        }
    }

    pub(crate) fn prepare_filter<'a>(&'a self, filter: &Filter) -> Result<Predicate<'a>> {
        filter::prepare_filter(self, filter)
    }

    pub fn parse_bound(&self, text: &str) -> Result<Bound> {
        Bound::parse(self.logical_data_type(), text)
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
        Error,
    };

    fn col(array: ArrayRef) -> Column {
        Column::new("test", array).unwrap()
    }

    fn matches(array: &ArrayRef, query: &str, range: std::ops::Range<usize>) -> Vec<bool> {
        let mut hits = vec![false; range.len()];
        if let Some(test) = col(array.clone()).prepare_find(query, None).unwrap() {
            test.evaluate(range, &mut hits, &CancelToken::never())
                .unwrap();
        }
        hits
    }

    #[test]
    fn capabilities_distinguish_value_semantics_from_storage_support() {
        let dictionary = |value| DataType::Dictionary(Box::new(DataType::Int32), Box::new(value));
        for (data_type, contains, equals, ordered, logarithmic) in [
            (DataType::Int32, false, true, true, true),
            (DataType::Date32, false, true, true, false),
            (DataType::Utf8, true, true, false, false),
            (DataType::Boolean, false, true, false, false),
            (DataType::Binary, true, false, false, false),
            (dictionary(DataType::Int64), false, true, true, false),
            (dictionary(DataType::Utf8), true, true, false, false),
        ] {
            assert_eq!(
                col(arrow::array::new_null_array(&data_type, 1)).capabilities(),
                ColumnCapabilities {
                    contains,
                    range: ordered,
                    exact: equals,
                    histogram: ordered && !matches!(data_type, DataType::Dictionary(..)),
                    scatter: ordered && !matches!(data_type, DataType::Dictionary(..)),
                    logarithmic,
                },
                "{data_type}"
            );
        }
    }

    #[test]
    fn unsupported_children_are_rejected_at_loading() {
        use arrow::datatypes::{Fields, UnionFields, UnionMode};
        let unsupported = DataType::Union(UnionFields::empty(), UnionMode::Sparse);
        let child = Arc::new(Field::new("child", unsupported.clone(), true));
        for data_type in [
            unsupported.clone(),
            DataType::List(child.clone()),
            DataType::Struct(Fields::from(vec![child.clone()])),
            DataType::Dictionary(Box::new(DataType::Int8), Box::new(unsupported)),
            DataType::RunEndEncoded(Arc::new(Field::new("runs", DataType::Int32, false)), child),
        ] {
            let array = arrow::array::new_empty_array(&data_type);
            assert!(
                matches!(
                    crate::Column::new("unsupported", array),
                    Err(crate::Error::Unsupported(_))
                ),
                "{data_type}"
            );
        }
    }

    #[test]
    fn typed_membership_validates_every_input_and_empty_lists_match_nothing() {
        let array: ArrayRef = Arc::new(Int32Array::from(vec![Some(1), None, Some(2)]));
        let column = col(array);
        let test = column
            .prepare_filter(&Filter::exact(Vec::new(), false, false))
            .unwrap();
        let mut hits = [true; 3];
        test.evaluate(0..3, &mut hits, &CancelToken::never())
            .unwrap();
        assert_eq!(hits, [false; 3]);
        assert!(matches!(
            column.prepare_filter(&Filter::exact(
                vec!["1".into(), "invalid".into()],
                false,
                false
            )),
            Err(Error::InvalidArgument(_))
        ));

        let array: ArrayRef = Arc::new(
            DictionaryArray::<Int8Type>::try_new(
                Int8Array::from(vec![Some(0), None, Some(1), Some(2)]),
                Arc::new(StringArray::from(vec![Some("01"), None, Some("2")])),
            )
            .unwrap(),
        );
        let column = col(array);
        let test = column
            .prepare_filter(&Filter::exact(vec!["01".into(), "2".into()], false, false))
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
        let column = col(array);
        let test = column
            .prepare_filter(&Filter::exact(vec!["A%_\\z".into()], false, false))
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
            let column = col(array.clone());
            for (include_nulls, expected) in [(false, excluded), (true, included)] {
                let test = column
                    .prepare_filter(&Filter::contains(text.into(), true, include_nulls))
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
        let column = col(array);
        let test = column
            .prepare_filter(&Filter::contains(String::new(), false, false))
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
        let column = col(array);
        let test = column
            .prepare_filter(&Filter::exact(vec!["2".into()], false, false))
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
    fn dictionary_filter_capabilities_follow_the_value_type() {
        let array: ArrayRef = Arc::new(
            DictionaryArray::<Int8Type>::try_new(
                Int8Array::from(vec![0]),
                Arc::new(Int32Array::from(vec![2])),
            )
            .unwrap(),
        );
        assert_eq!(col(array).capabilities(), ColumnCapabilities {
            contains: false,
            range: true,
            exact: true,
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
        let engine = ViewEngine::new(1, 1);
        let token = engine.begin();
        engine.cancel_pending();
        for array in arrays {
            let column = col(array);
            let test = column.prepare_find("2", None).unwrap().unwrap();
            assert!(matches!(
                test.evaluate(0..1, &mut [false], &token),
                Err(Error::Cancelled)
            ));
        }
    }

    #[test]
    fn dictionary_is_built_once_and_cached() {
        let column = col(Arc::new(StringArray::from(vec![
            Some("a"),
            None,
            Some("b"),
        ])));
        let first = column.dictionary().cloned().expect("dictionary");
        let second = column.dictionary().cloned().expect("dictionary");
        assert!(Arc::ptr_eq(&first, &second));

        let high_cardinality: ArrayRef = Arc::new(Int32Array::from_iter_values(0..1000));
        assert!(col(high_cardinality).dictionary().is_none());
    }

    #[test]
    fn column_len_and_empty() {
        let empty_col = col(Arc::new(Int32Array::from(Vec::<i32>::new())));
        assert_eq!(empty_col.len(), 0);
        assert!(empty_col.is_empty());

        let array: ArrayRef = Arc::new(StringArray::from(vec![
            Some("apple"),
            Some("banana"),
            Some("apple"),
            None,
        ]));
        let column = col(array);
        assert_eq!(column.len(), 4);
        assert!(!column.is_empty());
    }
}
