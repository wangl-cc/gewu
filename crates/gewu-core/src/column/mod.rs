//! First-class persistent Column model and column-level operations.
//!
//! Logical semantics, indexing, formatting, statistics and predicate compilation
//! are organized into five orthogonal modules:
//! - [`kind`]: LogicalType, ColumnCapabilities and ColumnInfo
//! - [`format`]: Cell and array formatting
//! - [`index`]: Distinct value indexing and counting
//! - [`stats`]: Histograms and distributions
//! - [`filter`]: Single-column filter and Find predicate compilation

pub mod filter;
pub mod format;
pub mod index;
pub mod kind;
pub mod stats;

use std::sync::Arc;

use arrow::{array::ArrayRef, datatypes::DataType, util::display::ArrayFormatter};
pub use filter::range::Bound;
pub use index::{DistinctResult, DistinctValue};
pub(crate) use index::{ValueIndex, INDEX_LIMIT};
pub use kind::{ColumnCapabilities, ColumnInfo, LogicalType};
pub use stats::{AxisScale, Histogram, RangeBounds, RangeHistogram};

use crate::{predicate::Predicate, Filter, Result};

/// Suggested cardinality threshold for displaying choices in interactive filters.
pub const CHOICE_LIMIT: usize = 500;

/// An immutable, first-class column owned by a [`crate::Dataset`].
pub struct Column {
    pub name: String,
    pub logical_type: LogicalType,
    pub nullable: bool,
    pub len: usize,
    pub array: ArrayRef,
}

impl Column {
    pub fn new(name: impl Into<String>, array: ArrayRef) -> Self {
        let logical_type = LogicalType::from_arrow(array.data_type());
        let len = array.len();
        let nullable = array.is_nullable();
        Self {
            name: name.into(),
            logical_type,
            nullable,
            len,
            array,
        }
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
    pub fn array(&self) -> &ArrayRef {
        &self.array
    }

    #[inline]
    pub fn logical_type(&self) -> &LogicalType {
        &self.logical_type
    }

    #[inline]
    pub fn is_nullable(&self) -> bool {
        self.nullable
    }

    #[inline]
    pub fn is_dictionary(&self) -> bool {
        matches!(self.array.data_type(), DataType::Dictionary(..))
    }

    #[inline]
    pub fn capabilities(&self) -> ColumnCapabilities {
        self.logical_type.capabilities(self.is_dictionary())
    }

    pub fn logical_data_type(&self) -> &DataType {
        let mut data_type = self.array.data_type();
        while let DataType::Dictionary(_, values) = data_type {
            data_type = values;
        }
        data_type
    }

    pub fn info(&self) -> ColumnInfo {
        ColumnInfo {
            name: self.name.clone(),
            display_type: self.logical_type.display_name(),
            technical_type: self.array.data_type().to_string(),
            logical_type: self.logical_type.clone(),
            nullable: self.nullable,
        }
    }

    pub fn formatter(&self) -> Result<ArrayFormatter<'_>> {
        format::formatter(self.array.as_ref())
    }

    pub fn format_cell(&self, row: usize) -> Result<String> {
        let formatter = self.formatter()?;
        Ok(formatter.value(row).to_string())
    }

    #[cfg(test)]
    pub(crate) fn prepare_find<'a>(
        &'a self,
        query: &str,
        matcher: Option<&regex::Regex>,
    ) -> Result<Option<Predicate<'a>>> {
        filter::prepare_find(&self.array, &self.logical_type, query, matcher)
    }

    pub(crate) fn prepare_filter<'a>(&'a self, filter: &Filter) -> Result<Predicate<'a>> {
        filter::prepare_filter(&self.array, &self.logical_type, self.capabilities(), filter)
    }

    pub(crate) fn prepare_values<'a>(&self, index: Arc<ValueIndex>, keys: &[u64]) -> Predicate<'a> {
        index.prepare_predicate(keys)
    }

    pub fn parse_bound(&self, text: &str) -> Result<Bound> {
        Bound::parse(self.logical_data_type(), text)
    }

    pub fn make_bound_i64(&self, value: i64) -> Result<Bound> {
        Bound::from_i64(self.logical_data_type(), value)
    }

    pub fn make_bound_u64(&self, value: u64) -> Result<Bound> {
        Bound::from_u64(self.logical_data_type(), value)
    }

    pub fn make_bound_f64(&self, value: f64) -> Result<Bound> {
        Bound::from_f64(self.logical_data_type(), value)
    }

    pub fn make_bound_decimal(&self, coefficient: &str, scale: i8) -> Result<Bound> {
        let coeff = coefficient
            .parse::<arrow::datatypes::i256>()
            .map_err(|_| crate::Error::InvalidArgument("invalid decimal coefficient".into()))?;
        Bound::from_decimal(self.logical_data_type(), coeff, scale)
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
        Column::new("test", array)
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
    fn typed_membership_validates_every_input_and_empty_lists_match_nothing() {
        let array: ArrayRef = Arc::new(Int32Array::from(vec![Some(1), None, Some(2)]));
        let column = col(array);
        let test = column
            .prepare_filter(&Filter::EqualsAny { values: Vec::new() })
            .unwrap();
        let mut hits = [true; 3];
        test.evaluate(0..3, &mut hits, &CancelToken::never())
            .unwrap();
        assert_eq!(hits, [false; 3]);
        assert!(matches!(
            column.prepare_filter(&Filter::EqualsAny {
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
        let column = col(array);
        let test = column
            .prepare_filter(&Filter::EqualsAny {
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
        let column = col(array);
        let test = column
            .prepare_filter(&Filter::Equals {
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
            let column = col(array.clone());
            for (include_nulls, expected) in [(false, excluded), (true, included)] {
                let test = column
                    .prepare_filter(&Filter::Contains {
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
        let column = col(array);
        let test = column
            .prepare_filter(&Filter::Contains {
                include_nulls: false,
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
        let column = col(array);
        let test = column
            .prepare_filter(&Filter::Equals { value: "2".into() })
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
        assert_eq!(col(array).capabilities(), ColumnCapabilities {
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
            let column = col(array);
            let test = column.prepare_find("2", None).unwrap().unwrap();
            assert!(matches!(
                test.evaluate(0..1, &mut [false], &token),
                Err(Error::Cancelled)
            ));
        }
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
