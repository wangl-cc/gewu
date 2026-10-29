//! The one place that sorts Arrow types into the kinds gewu treats
//! differently. Filters, search, histograms and the UI all ask this.

use arrow::datatypes::{DataType, TimeUnit};

/// What sort of values a column holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataKind {
    /// Strings, including dictionary-encoded strings.
    Text,
    Integer,
    Float,
    Decimal,
    Boolean,
    Date,
    /// Time of day.
    Time,
    Timestamp,
    Duration,
    /// Calendar intervals (months, days, nanoseconds). Not ordered.
    Interval,
    Binary,
    /// Lists of any element type.
    List,
    /// Structs, maps, unions, nulls and anything else.
    Other,
}

impl DataKind {
    pub fn of(data_type: &DataType) -> Self {
        use DataType::*;
        match data_type {
            Utf8 | LargeUtf8 | Utf8View => Self::Text,
            Dictionary(_, value) if Self::of(value) == Self::Text => Self::Text,
            Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64 => Self::Integer,
            Float16 | Float32 | Float64 => Self::Float,
            Decimal32(..) | Decimal64(..) | Decimal128(..) | Decimal256(..) => Self::Decimal,
            Boolean => Self::Boolean,
            Date32 | Date64 => Self::Date,
            Time32(TimeUnit::Second | TimeUnit::Millisecond)
            | Time64(TimeUnit::Microsecond | TimeUnit::Nanosecond) => Self::Time,
            Timestamp(..) => Self::Timestamp,
            Duration(_) => Self::Duration,
            Interval(_) => Self::Interval,
            Binary | LargeBinary | BinaryView | FixedSizeBinary(_) => Self::Binary,
            List(_) | LargeList(_) | ListView(_) | LargeListView(_) | FixedSizeList(..) => {
                Self::List
            }
            _ => Self::Other,
        }
    }

    /// Ordered single values: range filters and histograms work on these.
    pub fn is_ordered(self) -> bool {
        matches!(
            self,
            Self::Integer
                | Self::Float
                | Self::Decimal
                | Self::Date
                | Self::Time
                | Self::Timestamp
                | Self::Duration
        )
    }

    /// Dates and times, shown and typed as text but stored as integers.
    pub fn is_temporal(self) -> bool {
        matches!(
            self,
            Self::Date | Self::Time | Self::Timestamp | Self::Duration
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::new_null_array,
        datatypes::{Field, IntervalUnit},
    };

    use super::*;
    use crate::numeric::Numeric;

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
            Interval(IntervalUnit::YearMonth),
            Interval(IntervalUnit::DayTime),
            Interval(IntervalUnit::MonthDayNano),
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

    /// Ordered kinds and the numeric operations must cover exactly the same
    /// types, or a range filter would fail on a column the UI offers it for.
    #[test]
    fn ordered_kinds_match_numeric_support() {
        for t in every_type() {
            let array = new_null_array(&t, 1);
            let numeric = Numeric::new(&array);
            assert_eq!(
                DataKind::of(&t).is_ordered(),
                numeric.is_some(),
                "{t} is {:?}",
                DataKind::of(&t)
            );
        }
    }

    #[test]
    fn dictionary_strings_are_text() {
        let t = DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::LargeUtf8));
        assert_eq!(DataKind::of(&t), DataKind::Text);
    }
}
