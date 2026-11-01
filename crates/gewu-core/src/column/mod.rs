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

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, OnceLock,
};

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
    pub array: ArrayRef,
    pub(crate) index: OnceLock<index::Indexed>,
    /// Lazily computed distinct count cache.
    ///
    /// 0 indicates uncomputed (unless `array.is_empty()`, where distinct count is genuinely 0).
    pub(crate) distinct_count: AtomicUsize,
}

impl Column {
    pub fn new(name: impl Into<String>, array: ArrayRef, nullable: bool) -> Self {
        let logical_type = LogicalType::from_arrow(array.data_type());
        Self {
            name: name.into(),
            logical_type,
            nullable,
            array,
            index: OnceLock::new(),
            distinct_count: AtomicUsize::new(0),
        }
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

    pub(crate) fn index(&self) -> Result<Option<Arc<ValueIndex>>> {
        let found = match self.index.get() {
            Some(found) => found,
            None => {
                let built = match ValueIndex::build(&self.array, INDEX_LIMIT)? {
                    Some(idx) => index::Indexed::Built(Arc::new(idx)),
                    None => index::Indexed::TooMany,
                };
                self.index.get_or_init(|| built)
            }
        };
        Ok(match found {
            index::Indexed::Built(idx) => Some(idx.clone()),
            index::Indexed::TooMany => None,
        })
    }

    pub(crate) fn keyed_index(&self) -> Result<Arc<ValueIndex>> {
        self.index()?.ok_or_else(|| {
            crate::Error::InvalidArgument(format!(
                "column {} has more than {INDEX_LIMIT} distinct values; filter it with contains or a range",
                self.name
            ))
        })
    }

    /// Return the exact number of distinct values in the column, lazily computed and cached.
    ///
    /// If the column is empty, returns 0 immediately without scanning.
    /// If an inverted index is already built, returns its exact dictionary length.
    pub fn distinct_count(&self) -> Result<usize> {
        if self.array.is_empty() {
            return Ok(0);
        }
        if let Some(index::Indexed::Built(index)) = self.index.get() {
            return Ok(index.len());
        }
        let cached = self.distinct_count.load(Ordering::Relaxed);
        if cached > 0 {
            return Ok(cached);
        }
        let count = index::count_distinct(&self.array)?;
        self.distinct_count.store(count, Ordering::Relaxed);
        Ok(count)
    }

    /// Check whether the number of distinct values in the column is at most `limit`.
    ///
    /// Short-circuits if the array is empty, if the inverted index is built, or if the
    /// exact distinct count has already been cached. Otherwise, uses a bounded probe
    /// that stops immediately upon encountering `limit + 1` distinct values.
    pub fn distinct_count_at_most(&self, limit: usize) -> Result<bool> {
        if self.array.is_empty() {
            return Ok(true);
        }
        if let Some(index::Indexed::Built(index)) = self.index.get() {
            return Ok(index.len() <= limit);
        }
        let cached = self.distinct_count.load(Ordering::Relaxed);
        if cached > 0 {
            return Ok(cached <= limit);
        }
        let supported = arrow::row::RowConverter::supports_fields(&[arrow::row::SortField::new(
            self.array.data_type().clone(),
        )]);
        if !supported {
            return Ok(false);
        }
        index::has_at_most(&self.array, limit)
    }

    /// Check whether the column has low cardinality (at most [`CHOICE_LIMIT`] distinct values).
    #[inline]
    pub fn is_low_cardinality(&self) -> Result<bool> {
        self.distinct_count_at_most(CHOICE_LIMIT)
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
        Column::new("test", array, true)
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
    fn distinct_count_and_probe_caching() {
        // 1. Empty column
        let empty_col = col(Arc::new(Int32Array::from(Vec::<i32>::new())));
        assert_eq!(empty_col.distinct_count().unwrap(), 0);
        assert!(empty_col.distinct_count_at_most(0).unwrap());

        // 2. Non-empty column lazy distinct count and atomic cache
        let array: ArrayRef = Arc::new(StringArray::from(vec![
            Some("apple"),
            Some("banana"),
            Some("apple"),
            None,
        ]));
        let column = col(array);
        assert_eq!(column.distinct_count.load(Ordering::Relaxed), 0);
        assert!(column.distinct_count_at_most(10).unwrap());
        assert!(!column.distinct_count_at_most(2).unwrap());
        assert_eq!(column.distinct_count().unwrap(), 3);
        // After distinct_count(), it is cached in the AtomicUsize
        assert_eq!(column.distinct_count.load(Ordering::Relaxed), 3);
        // Repeated call reads the atomic cache directly
        assert_eq!(column.distinct_count().unwrap(), 3);
    }
}
