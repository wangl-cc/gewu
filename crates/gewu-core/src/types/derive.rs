//! The single definition of what a column type supports.
//!
//! Every policy question — which filters, which inspector sections, which
//! charts, how Find searches, how the distinct-value dictionary encodes — is
//! answered here from the four dimensions of [`Type`]. Consumers call these
//! methods instead of matching Arrow types or re-deriving rules.

use arrow::datatypes::DataType;

use super::{Domain, Encoding, Scalar, Shape, Type};
use crate::types::logical::ColumnCapabilities;

/// A section of the column inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisSection {
    Overview,
    Statistics,
    Distribution,
    Values,
    Lengths,
    Structure,
}

/// What a length measurement counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthUnit {
    Characters,
    Bytes,
    Elements,
}

/// How Find searches a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindStrategy {
    /// Find never matches this column; it is skipped.
    Unsupported,
    /// Case-insensitive substring on native string storage.
    Text,
    /// Parsed typed search (numbers, booleans, temporal values).
    Scalar,
    /// Any element matches.
    List,
    /// Any field matches.
    Struct,
    /// Follow the dictionary keys to their values.
    Dictionary,
}

/// Scalar class of a directly stored column. Ordered operations and typing of
/// explicit inputs currently require native arrays rather than dictionary keys.
pub(crate) fn direct_domain(data_type: &DataType) -> Option<Domain> {
    Type::from_arrow(data_type)?.direct_domain()
}

impl Type {
    /// The scalar class when the values are stored directly. Ordered operations
    /// and typed explicit inputs currently require native arrays, not
    /// dictionary keys.
    pub fn direct_domain(&self) -> Option<Domain> {
        if self.encoding == Encoding::Direct {
            self.domain()
        } else {
            None
        }
    }

    /// Ordered single values: sorting, range filters, statistics, histograms
    /// and charts work on these. Dictionary-encoded columns stay discrete.
    pub fn is_ordered(&self) -> bool {
        matches!(
            self.domain(),
            Some(
                Domain::Integer
                    | Domain::Float
                    | Domain::Decimal
                    | Domain::Date
                    | Domain::Time
                    | Domain::Timestamp
                    | Domain::Duration
            )
        ) && self.encoding == Encoding::Direct
    }

    /// Numeric values: means, medians and logarithmic binning apply.
    pub fn is_numeric(&self) -> bool {
        matches!(
            self.domain(),
            Some(Domain::Integer | Domain::Float | Domain::Decimal)
        ) && self.encoding == Encoding::Direct
    }

    /// Whether substring or element containment applies. Ordered values and
    /// booleans have no useful displayed text to search.
    pub fn supports_contains(&self) -> bool {
        let value = self.value_type();
        !(value.is_ordered() || value.domain() == Some(Domain::Boolean))
    }

    /// Whether equality and equality-any filters apply.
    pub fn supports_equals(&self) -> bool {
        let value = self.value_type();
        value.is_ordered() || matches!(value.domain(), Some(Domain::Boolean | Domain::Text))
    }

    /// Every operation this type supports.
    pub fn capabilities(&self) -> ColumnCapabilities {
        match self.encoding {
            // A dictionary is storage for a value type. Ordered operations need
            // a directly stored array today, so they stay off; Find and the
            // value-list filters follow the keys instead.
            Encoding::Dictionary => ColumnCapabilities {
                contains: self.supports_contains(),
                range: false,
                equals: self.supports_equals(),
                histogram: false,
                scatter: false,
                logarithmic: false,
            },
            Encoding::Direct => ColumnCapabilities {
                contains: self.supports_contains(),
                range: self.is_ordered(),
                equals: self.supports_equals(),
                histogram: self.is_ordered(),
                scatter: self.is_ordered(),
                logarithmic: self.is_numeric(),
            },
        }
    }

    /// The inspector sections that apply, in presentation order.
    pub fn analysis_sections(&self) -> Vec<AnalysisSection> {
        use AnalysisSection::*;
        let mut sections = vec![Overview];
        if self.is_ordered() {
            sections.extend([Statistics, Distribution]);
        }
        let value = self.value_type();
        if value.is_ordered() || matches!(value.domain(), Some(Domain::Text | Domain::Boolean)) {
            sections.push(Values);
        }
        if self.length_unit().is_some() {
            sections.push(Lengths);
        }
        if matches!(
            self.shape,
            Shape::List { .. } | Shape::Struct { .. } | Shape::Map { .. }
        ) || self.is_dictionary()
        {
            sections.push(Structure);
        }
        sections
    }

    /// What a length measurement counts. Only directly stored values have one;
    /// dictionary values are enumerated by their keys instead.
    pub fn length_unit(&self) -> Option<LengthUnit> {
        if self.encoding != Encoding::Direct {
            return None;
        }
        match &self.shape {
            Shape::Scalar(Scalar::Text) => Some(LengthUnit::Characters),
            Shape::Scalar(Scalar::Binary) => Some(LengthUnit::Bytes),
            Shape::List { .. } | Shape::Map { .. } => Some(LengthUnit::Elements),
            _ => None,
        }
    }

    /// How Find searches this column.
    pub fn find_strategy(&self) -> FindStrategy {
        if self.is_dictionary() {
            return match self.value_type().find_strategy() {
                FindStrategy::Unsupported => FindStrategy::Unsupported,
                _ => FindStrategy::Dictionary,
            };
        }
        let findable = |ty: &Type| ty.find_strategy() != FindStrategy::Unsupported;
        match &self.shape {
            Shape::Scalar(Scalar::Text) => FindStrategy::Text,
            // Binary has no typed input syntax; intervals are not ordered
            // scalars. Both are excluded from typed Find.
            Shape::Scalar(Scalar::Binary) | Shape::Scalar(Scalar::Interval { .. }) => {
                FindStrategy::Unsupported
            }
            Shape::Scalar(_) => FindStrategy::Scalar,
            Shape::Null | Shape::Map { .. } => FindStrategy::Unsupported,
            Shape::List { element, .. } => {
                if findable(element) {
                    FindStrategy::List
                } else {
                    FindStrategy::Unsupported
                }
            }
            Shape::Struct { fields } => {
                if fields.iter().any(|field| findable(&field.ty)) {
                    FindStrategy::Struct
                } else {
                    FindStrategy::Unsupported
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::datatypes::{DataType, Field, Fields, TimeUnit};

    use super::*;
    use crate::types::{LogicalType, Type};

    fn ty(data_type: &DataType) -> Type {
        Type::from_arrow(data_type).unwrap()
    }

    fn dictionary(value: DataType) -> DataType {
        DataType::Dictionary(Box::new(DataType::Int32), Box::new(value))
    }

    fn item(data_type: DataType) -> Arc<Field> {
        Arc::new(Field::new("item", data_type, true))
    }

    #[test]
    fn ordered_and_numeric_follow_the_value_domain() {
        for ordered in [
            DataType::Int32,
            DataType::Float64,
            DataType::Decimal128(10, 2),
            DataType::Date32,
            DataType::Time32(TimeUnit::Second),
            DataType::Timestamp(TimeUnit::Second, None),
            DataType::Duration(TimeUnit::Second),
        ] {
            assert!(ty(&ordered).is_ordered(), "{ordered}");
        }
        for unordered in [
            DataType::Utf8,
            DataType::Boolean,
            DataType::Binary,
            DataType::List(item(DataType::Int32)),
        ] {
            assert!(!ty(&unordered).is_ordered(), "{unordered}");
        }
        assert!(ty(&DataType::Int32).is_numeric());
        assert!(!ty(&DataType::Date32).is_numeric());
        assert!(!ty(&DataType::Utf8).is_numeric());
        // Storage does not change the value domain: only the direct array
        // supports ordered operations today.
        let encoded = ty(&dictionary(DataType::Int64));
        assert_eq!(encoded.domain(), Some(Domain::Integer));
        assert!(!encoded.is_ordered());
        assert!(!encoded.is_numeric());
    }

    #[test]
    fn capabilities_match_the_documented_operations() {
        assert_eq!(ty(&DataType::Int32).capabilities(), ColumnCapabilities {
            contains: false,
            range: true,
            equals: true,
            histogram: true,
            scatter: true,
            logarithmic: true,
        });
        assert_eq!(ty(&DataType::Utf8).capabilities(), ColumnCapabilities {
            contains: true,
            range: false,
            equals: true,
            histogram: false,
            scatter: false,
            logarithmic: false,
        });
        assert_eq!(ty(&DataType::Boolean).capabilities(), ColumnCapabilities {
            contains: false,
            range: false,
            equals: true,
            histogram: false,
            scatter: false,
            logarithmic: false,
        });
        // A dictionary column is discrete: it offers choices, Find and value
        // lists, but not ordered filters or charts.
        assert_eq!(
            ty(&dictionary(DataType::Int64)).capabilities(),
            ColumnCapabilities {
                contains: false,
                range: false,
                equals: true,
                histogram: false,
                scatter: false,
                logarithmic: false,
            }
        );
        let binary: Type = ty(&DataType::Binary);
        assert!(binary.supports_contains());
        assert!(!binary.supports_equals());
    }

    #[test]
    fn analysis_sections_follow_shape_and_domain() {
        use AnalysisSection::*;
        assert_eq!(ty(&DataType::Int32).analysis_sections(), [
            Overview,
            Statistics,
            Distribution,
            Values
        ]);
        assert_eq!(ty(&DataType::Utf8).analysis_sections(), [
            Overview, Values, Lengths
        ]);
        // Binary has length but no value-list grouping: its values have no
        // comparable or selectable labels.
        assert_eq!(ty(&DataType::Binary).analysis_sections(), [
            Overview, Lengths
        ]);
        assert_eq!(
            ty(&DataType::List(item(DataType::Int32))).analysis_sections(),
            [Overview, Lengths, Structure]
        );
        assert_eq!(
            ty(&DataType::Struct(Fields::from(vec![Field::new(
                "a",
                DataType::Int32,
                true
            )])))
            .analysis_sections(),
            [Overview, Structure]
        );
        assert_eq!(ty(&dictionary(DataType::Int64)).analysis_sections(), [
            Overview, Values, Structure
        ]);
        assert_eq!(ty(&DataType::Null).analysis_sections(), [Overview]);
    }

    #[test]
    fn length_units_follow_shape() {
        assert_eq!(
            ty(&DataType::Utf8).length_unit(),
            Some(LengthUnit::Characters)
        );
        assert_eq!(ty(&DataType::Binary).length_unit(), Some(LengthUnit::Bytes));
        assert_eq!(
            ty(&DataType::List(item(DataType::Int32))).length_unit(),
            Some(LengthUnit::Elements)
        );
        assert_eq!(ty(&DataType::Int32).length_unit(), None);
        assert_eq!(
            ty(&dictionary(DataType::Utf8)).length_unit(),
            None,
            "dictionary columns list values instead of measuring them"
        );
    }

    #[test]
    fn find_strategies_follow_shape_and_domain() {
        assert_eq!(ty(&DataType::Utf8).find_strategy(), FindStrategy::Text);
        assert_eq!(ty(&DataType::Int32).find_strategy(), FindStrategy::Scalar);
        assert_eq!(ty(&DataType::Boolean).find_strategy(), FindStrategy::Scalar);
        // Documented exclusions: typed Find skips these columns.
        for excluded in [
            DataType::Binary,
            DataType::Interval(arrow::datatypes::IntervalUnit::DayTime),
        ] {
            assert_eq!(ty(&excluded).find_strategy(), FindStrategy::Unsupported);
        }
        assert_eq!(
            ty(&dictionary(DataType::Utf8)).find_strategy(),
            FindStrategy::Dictionary
        );
        assert_eq!(
            ty(&dictionary(DataType::Binary)).find_strategy(),
            FindStrategy::Unsupported
        );
        let list = ty(&DataType::List(item(DataType::Utf8)));
        assert_eq!(list.find_strategy(), FindStrategy::List);
        // A list whose elements have no typed search cannot be searched.
        let binary_list = ty(&DataType::List(item(DataType::Binary)));
        assert_eq!(binary_list.find_strategy(), FindStrategy::Unsupported);
        // A struct is searchable when at least one field is.
        let fields = Fields::from(vec![
            Field::new("a", DataType::Binary, true),
            Field::new("b", DataType::Utf8, true),
        ]);
        assert_eq!(
            ty(&DataType::Struct(fields)).find_strategy(),
            FindStrategy::Struct
        );
        let binary_only = Fields::from(vec![Field::new("a", DataType::Binary, true)]);
        assert_eq!(
            ty(&DataType::Struct(binary_only)).find_strategy(),
            FindStrategy::Unsupported
        );
    }

    #[test]
    fn logical_projection_agrees_with_the_model() {
        for data_type in [
            DataType::Int32,
            DataType::Utf8,
            DataType::Date32,
            dictionary(DataType::Int64),
            DataType::List(item(DataType::Utf8)),
        ] {
            let model = ty(&data_type);
            assert_eq!(
                LogicalType::from_arrow(&data_type),
                Some(model.logical()),
                "{data_type}"
            );
        }
    }

    /// Every Arrow type a loaded document can carry. Ordered kinds and the
    /// numeric kernels must cover exactly the same types, or a range filter
    /// would be offered for a column whose statistics then fail.
    fn every_type() -> Vec<DataType> {
        use DataType::*;
        let item = Arc::new(Field::new("item", Int32, true));
        let mut types = vec![
            Null,
            Boolean,
            Int8,
            Int16,
            Int32,
            Int64,
            UInt8,
            UInt16,
            UInt32,
            UInt64,
            Float16,
            Float32,
            Float64,
            Date32,
            Date64,
            Interval(arrow::datatypes::IntervalUnit::YearMonth),
            Interval(arrow::datatypes::IntervalUnit::DayTime),
            Interval(arrow::datatypes::IntervalUnit::MonthDayNano),
            Binary,
            LargeBinary,
            BinaryView,
            FixedSizeBinary(4),
            Utf8,
            LargeUtf8,
            Utf8View,
            List(item.clone()),
            LargeList(item.clone()),
            FixedSizeList(item, 2),
            Decimal32(9, 2),
            Decimal64(18, 2),
            Decimal128(38, 2),
            Decimal256(76, 2),
            Dictionary(Box::new(Int32), Box::new(Utf8)),
            Dictionary(Box::new(Int32), Box::new(Int64)),
            Struct(Fields::from(vec![Field::new("f", Int32, true)])),
        ];
        for unit in [
            TimeUnit::Second,
            TimeUnit::Millisecond,
            TimeUnit::Microsecond,
            TimeUnit::Nanosecond,
        ] {
            types.push(Timestamp(unit, None));
            types.push(Timestamp(unit, Some("UTC".into())));
            types.push(Duration(unit));
        }
        types.push(Time32(TimeUnit::Second));
        types.push(Time32(TimeUnit::Millisecond));
        types.push(Time64(TimeUnit::Microsecond));
        types.push(Time64(TimeUnit::Nanosecond));
        types
    }

    #[test]
    fn ordered_kinds_match_numeric_support() {
        for data_type in every_type() {
            let array = arrow::array::new_null_array(&data_type, 1);
            let model = Type::from_arrow(&data_type);
            assert_eq!(
                model.as_ref().is_some_and(Type::is_ordered),
                crate::analysis::numeric::Numeric::new(&array).is_some(),
                "{data_type}"
            );
        }
    }
}
