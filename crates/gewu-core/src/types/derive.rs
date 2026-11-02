//! Operation policies projected directly from Arrow storage types.

use arrow::datatypes::DataType;

use super::{direct_domain, domain, is_supported, value_type, ColumnCapabilities, Domain};

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

/// Whether the native numeric/temporal kernels support this storage type.
/// Dictionary values retain their domain but need a different execution path.
pub(crate) fn supports_ordered(data_type: &DataType) -> bool {
    matches!(
        direct_domain(data_type),
        Some(
            Domain::Integer
                | Domain::Float
                | Domain::Decimal
                | Domain::Date
                | Domain::Time
                | Domain::Timestamp
                | Domain::Duration
        )
    )
}

/// Supported operations for the actual storage type. Dictionary values can be
/// searched and selected; ordered filters and charts require native arrays.
pub fn capabilities(data_type: &DataType) -> ColumnCapabilities {
    let supported = is_supported(data_type);
    let values = value_type(data_type);
    let ordered_values = supports_ordered(values);
    let ordered = supports_ordered(data_type);
    let value_domain = domain(data_type);
    ColumnCapabilities {
        contains: supported && !(ordered_values || value_domain == Some(Domain::Boolean)),
        range: ordered,
        equals: ordered_values || matches!(value_domain, Some(Domain::Boolean | Domain::Text)),
        histogram: ordered,
        scatter: ordered,
        logarithmic: matches!(
            direct_domain(data_type),
            Some(Domain::Integer | Domain::Float | Domain::Decimal)
        ),
    }
}

/// Inspector sections in presentation order, without inspecting column values.
pub fn analysis_sections(data_type: &DataType) -> Vec<AnalysisSection> {
    use AnalysisSection::*;
    let mut sections = vec![Overview];
    if !is_supported(data_type) {
        return sections;
    }
    if supports_ordered(data_type) {
        sections.extend([Statistics, Distribution]);
    }
    if supports_ordered(value_type(data_type))
        || matches!(domain(data_type), Some(Domain::Text | Domain::Boolean))
    {
        sections.push(Values);
    }
    if length_unit(data_type).is_some() {
        sections.push(Lengths);
    }
    if matches!(
        data_type,
        DataType::List(_)
            | DataType::LargeList(_)
            | DataType::ListView(_)
            | DataType::LargeListView(_)
            | DataType::FixedSizeList(..)
            | DataType::Struct(_)
            | DataType::Map(..)
            | DataType::Dictionary(..)
    ) {
        sections.push(Structure);
    }
    sections
}

/// Lengths measure directly stored values; dictionary columns list choices.
pub fn length_unit(data_type: &DataType) -> Option<LengthUnit> {
    use DataType::*;
    match data_type {
        Utf8 | LargeUtf8 | Utf8View => Some(LengthUnit::Characters),
        Binary | LargeBinary | BinaryView | FixedSizeBinary(_) => Some(LengthUnit::Bytes),
        List(_) | LargeList(_) | ListView(_) | LargeListView(_) | FixedSizeList(..) | Map(..) => {
            Some(LengthUnit::Elements)
        }
        _ => None,
    }
}

/// How Find searches a loaded column. Nested types borrow their Arrow children;
/// loading validates supported types before predicate preparation.
pub fn find_strategy(data_type: &DataType) -> FindStrategy {
    use DataType::*;
    match data_type {
        Dictionary(_, values) => match find_strategy(values) {
            FindStrategy::Unsupported => FindStrategy::Unsupported,
            _ => FindStrategy::Dictionary,
        },
        List(field)
        | LargeList(field)
        | ListView(field)
        | LargeListView(field)
        | FixedSizeList(field, _) => match find_strategy(field.data_type()) {
            FindStrategy::Unsupported => FindStrategy::Unsupported,
            _ => FindStrategy::List,
        },
        Struct(fields)
            if fields
                .iter()
                .any(|field| find_strategy(field.data_type()) != FindStrategy::Unsupported) =>
        {
            FindStrategy::Struct
        }
        _ => match direct_domain(data_type) {
            Some(Domain::Text) => FindStrategy::Text,
            Some(Domain::Binary | Domain::Interval) | None => FindStrategy::Unsupported,
            Some(_) => FindStrategy::Scalar,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::datatypes::{DataType, Field, Fields, TimeUnit};

    use super::*;

    fn dictionary(value: DataType) -> DataType {
        DataType::Dictionary(Box::new(DataType::Int32), Box::new(value))
    }

    fn item(data_type: DataType) -> Arc<Field> {
        Arc::new(Field::new("item", data_type, true))
    }

    #[test]
    fn capabilities_distinguish_value_semantics_from_storage_support() {
        for (data_type, contains, equals, ordered, logarithmic) in [
            (DataType::Int32, false, true, true, true),
            (DataType::Date32, false, true, true, false),
            (DataType::Utf8, true, true, false, false),
            (DataType::Boolean, false, true, false, false),
            (DataType::Binary, true, false, false, false),
            (dictionary(DataType::Int64), false, true, false, false),
            (dictionary(DataType::Utf8), true, true, false, false),
        ] {
            assert_eq!(
                capabilities(&data_type),
                ColumnCapabilities {
                    contains,
                    range: ordered,
                    equals,
                    histogram: ordered,
                    scatter: ordered,
                    logarithmic,
                },
                "{data_type}"
            );
        }
    }

    #[test]
    fn analysis_sections_preserve_presentation_order() {
        use AnalysisSection::*;
        for (data_type, expected) in [
            (DataType::Int32, vec![
                Overview,
                Statistics,
                Distribution,
                Values,
            ]),
            (DataType::Utf8, vec![Overview, Values, Lengths]),
            (DataType::Binary, vec![Overview, Lengths]),
            (DataType::List(item(DataType::Int32)), vec![
                Overview, Lengths, Structure,
            ]),
            (
                DataType::Struct(Fields::from(vec![Field::new("a", DataType::Int32, true)])),
                vec![Overview, Structure],
            ),
            (dictionary(DataType::Int64), vec![
                Overview, Values, Structure,
            ]),
            (DataType::Null, vec![Overview]),
        ] {
            assert_eq!(analysis_sections(&data_type), expected, "{data_type}");
        }
    }

    #[test]
    fn lengths_measure_native_values() {
        assert_eq!(
            length_unit(&DataType::Utf8View),
            Some(LengthUnit::Characters)
        );
        assert_eq!(length_unit(&DataType::Binary), Some(LengthUnit::Bytes));
        assert_eq!(
            length_unit(&DataType::List(item(DataType::Int32))),
            Some(LengthUnit::Elements)
        );
        assert_eq!(length_unit(&DataType::Int32), None);
        assert_eq!(length_unit(&dictionary(DataType::Utf8)), None);
    }

    #[test]
    fn find_follows_searchable_children_and_dictionary_values() {
        for (data_type, expected) in [
            (DataType::Utf8, FindStrategy::Text),
            (DataType::Int32, FindStrategy::Scalar),
            (DataType::Boolean, FindStrategy::Scalar),
            (DataType::Binary, FindStrategy::Unsupported),
            (
                DataType::Interval(arrow::datatypes::IntervalUnit::DayTime),
                FindStrategy::Unsupported,
            ),
            (dictionary(DataType::Utf8), FindStrategy::Dictionary),
            (dictionary(DataType::Binary), FindStrategy::Unsupported),
            (DataType::List(item(DataType::Utf8)), FindStrategy::List),
            (
                DataType::List(item(DataType::Binary)),
                FindStrategy::Unsupported,
            ),
            (
                DataType::Struct(Fields::from(vec![
                    Field::new("a", DataType::Binary, true),
                    Field::new("b", DataType::Utf8, true),
                ])),
                FindStrategy::Struct,
            ),
            (
                DataType::Struct(Fields::from(vec![Field::new("a", DataType::Binary, true)])),
                FindStrategy::Unsupported,
            ),
        ] {
            assert_eq!(find_strategy(&data_type), expected, "{data_type}");
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
    fn supported_types_have_display_projections_and_native_dispatch() {
        for data_type in every_type() {
            assert!(
                super::super::LogicalType::from_arrow(&data_type).is_some(),
                "{data_type}"
            );
            let array = arrow::array::new_null_array(&data_type, 1);
            if capabilities(&data_type).scatter {
                let numeric = crate::stats::numeric::Numeric::new(&array).unwrap();
                // Exercise the physical dispatch table, not just its type guard.
                assert_eq!(numeric.accessor()(0), None, "{data_type}");
            }
        }
    }
}
