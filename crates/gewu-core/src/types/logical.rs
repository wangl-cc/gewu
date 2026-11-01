//! The stable, display-oriented projection of [`Type`].
//!
//! [`LogicalType`] is the value semantics of a column: what the FFI, the app
//! and [`ColumnInfo`] show. Storage encoding is deliberately not part of it —
//! a dictionary of text is text with [`Encoding::Dictionary`] — so type badges
//! and value editors do not need to recurse through storage wrappers.
//!
//! It is derived from [`Type`] and must not be used for policy decisions;
//! those live in [`super::derive`].

use arrow::datatypes::{DataType, TimeUnit};

use super::{Domain, Encoding, Scalar, Shape, Type};

/// A column's value semantics with the parameters that make it display exact.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LogicalType {
    /// The Arrow null type: every value is missing.
    Null,
    /// Strings.
    Text,
    /// Signed or unsigned integer with bit width (8, 16, 32, 64).
    Integer { bits: u8, signed: bool },
    /// Floating point number with bit width (16, 32, 64).
    Float { bits: u8 },
    /// Fixed-point decimal with precision and scale.
    Decimal { precision: u8, scale: i8 },
    /// Boolean flag.
    Boolean,
    /// Calendar date (Date32, Date64).
    Date,
    /// Time of day (Time32, Time64).
    Time,
    /// Timestamp with time unit and optional timezone.
    Timestamp {
        unit: TimeUnit,
        timezone: Option<String>,
    },
    /// Duration with time unit.
    Duration { unit: TimeUnit },
    /// Calendar intervals (months, days, nanoseconds). Not ordered.
    Interval,
    /// Raw binary sequences.
    Binary,
    /// Lists of elements (List, LargeList, ListView, LargeListView, FixedSizeList).
    List,
    /// Nested structures.
    Struct,
    /// Key/value entries.
    Map,
}

/// Operations supported by a column's type. Value-list availability
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

/// Metadata describing a single column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub logical_type: LogicalType,
    /// How Arrow stores the values. A dictionary column is discrete: it offers
    /// value choices rather than ordered filters or charts.
    pub encoding: Encoding,
    pub display_type: String,
    pub technical_type: String,
    pub nullable: bool,
}

impl LogicalType {
    /// Build the display projection from Arrow, or `None` when the type has no
    /// Gewu semantics. Documents reject such columns when they are opened, so
    /// a column that exists always has a logical type.
    pub fn from_arrow(data_type: &DataType) -> Option<Self> {
        Type::from_arrow(data_type).map(|ty| ty.logical())
    }

    /// This projection's parameter-free scalar class.
    pub fn domain(&self) -> Option<Domain> {
        Some(match self {
            Self::Text => Domain::Text,
            Self::Boolean => Domain::Boolean,
            Self::Integer { .. } => Domain::Integer,
            Self::Float { .. } => Domain::Float,
            Self::Decimal { .. } => Domain::Decimal,
            Self::Date => Domain::Date,
            Self::Time => Domain::Time,
            Self::Timestamp { .. } => Domain::Timestamp,
            Self::Duration { .. } => Domain::Duration,
            Self::Interval => Domain::Interval,
            Self::Binary => Domain::Binary,
            Self::Null | Self::List | Self::Struct | Self::Map => return None,
        })
    }

    /// Normalized, concise display text for UI badges and type labels.
    pub fn display_name(&self) -> String {
        match self {
            Self::Null => "null".into(),
            Self::Text => "string".into(),
            Self::Integer { bits, signed } => {
                format!("{}{bits}", if *signed { "int" } else { "uint" })
            }
            Self::Float { bits } => format!("float{bits}"),
            Self::Decimal { precision, scale } => format!("decimal({precision},{scale})"),
            Self::Boolean => "bool".into(),
            Self::Date => "date".into(),
            Self::Time => "time".into(),
            Self::Timestamp {
                timezone: Some(tz), ..
            } => format!("timestamp({tz})"),
            Self::Timestamp { timezone: None, .. } => "timestamp".into(),
            Self::Duration { .. } => "duration".into(),
            Self::Interval => "interval".into(),
            Self::Binary => "binary".into(),
            Self::List => "list".into(),
            Self::Struct => "struct".into(),
            Self::Map => "map".into(),
        }
    }
}

impl Type {
    /// The display projection of this type. Every supported Arrow type has one;
    /// storage encoding is left out on purpose.
    pub fn logical(&self) -> LogicalType {
        match &self.shape {
            Shape::Null => LogicalType::Null,
            Shape::Scalar(scalar) => scalar_logical(scalar),
            Shape::List { .. } => LogicalType::List,
            Shape::Struct { .. } => LogicalType::Struct,
            Shape::Map { .. } => LogicalType::Map,
        }
    }
}

fn scalar_logical(scalar: &Scalar) -> LogicalType {
    match scalar {
        Scalar::Text => LogicalType::Text,
        Scalar::Boolean => LogicalType::Boolean,
        Scalar::Integer { bits, signed } => LogicalType::Integer {
            bits: *bits,
            signed: *signed,
        },
        Scalar::Float { bits } => LogicalType::Float { bits: *bits },
        Scalar::Decimal { precision, scale } => LogicalType::Decimal {
            precision: *precision,
            scale: *scale,
        },
        Scalar::Date => LogicalType::Date,
        Scalar::Time { .. } => LogicalType::Time,
        Scalar::Timestamp { unit, timezone } => LogicalType::Timestamp {
            unit: *unit,
            timezone: timezone.as_ref().map(|tz| tz.to_string()),
        },
        Scalar::Duration { unit } => LogicalType::Duration { unit: *unit },
        Scalar::Interval { .. } => LogicalType::Interval,
        Scalar::Binary => LogicalType::Binary,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::datatypes::{Field, Fields, IntervalUnit};

    use super::*;
    use crate::types::Type;

    fn field(name: &str, data_type: DataType) -> Arc<Field> {
        Arc::new(Field::new(name, data_type, true))
    }

    #[test]
    fn from_arrow_parameters_preserved() {
        use DataType::*;

        assert_eq!(LogicalType::from_arrow(&Utf8), Some(LogicalType::Text));
        assert_eq!(
            LogicalType::from_arrow(&UInt32),
            Some(LogicalType::Integer {
                bits: 32,
                signed: false
            })
        );
        assert_eq!(
            LogicalType::from_arrow(&Float16),
            Some(LogicalType::Float { bits: 16 })
        );
        assert_eq!(
            LogicalType::from_arrow(&Decimal256(76, -4)),
            Some(LogicalType::Decimal {
                precision: 76,
                scale: -4
            })
        );
        assert_eq!(
            LogicalType::from_arrow(&Timestamp(TimeUnit::Microsecond, Some("UTC".into()))),
            Some(LogicalType::Timestamp {
                unit: TimeUnit::Microsecond,
                timezone: Some("UTC".into())
            })
        );
        assert_eq!(
            LogicalType::from_arrow(&Interval(IntervalUnit::DayTime)),
            Some(LogicalType::Interval)
        );
        assert_eq!(
            LogicalType::from_arrow(&List(field("item", Int32))),
            Some(LogicalType::List)
        );
        assert_eq!(
            LogicalType::from_arrow(&Struct(Fields::from(vec![field("f", Int32)]))),
            Some(LogicalType::Struct)
        );
        assert_eq!(LogicalType::from_arrow(&Null), Some(LogicalType::Null));
    }

    #[test]
    fn dictionary_encoding_is_not_part_of_the_display_type() {
        let dictionary = DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8));
        assert_eq!(
            LogicalType::from_arrow(&dictionary),
            Some(LogicalType::Text)
        );
        assert_eq!(
            Type::from_arrow(&dictionary).unwrap().encoding(),
            Encoding::Dictionary
        );
        assert_eq!(
            LogicalType::from_arrow(&dictionary),
            LogicalType::from_arrow(&DataType::Utf8)
        );
    }

    #[test]
    fn map_has_its_own_display_type() {
        let map = DataType::Map(
            Arc::new(Field::new(
                "entries",
                DataType::Struct(Fields::from(vec![
                    Field::new("keys", DataType::Utf8, false),
                    Field::new("values", DataType::Int32, true),
                ])),
                false,
            )),
            false,
        );
        assert_eq!(LogicalType::from_arrow(&map), Some(LogicalType::Map));
        assert!(matches!(
            Type::from_arrow(&map).unwrap().shape(),
            Shape::Map { .. }
        ));
    }

    #[test]
    fn display_name_formatting() {
        assert_eq!(LogicalType::Null.display_name(), "null");
        assert_eq!(LogicalType::Text.display_name(), "string");
        assert_eq!(
            LogicalType::Integer {
                bits: 64,
                signed: true
            }
            .display_name(),
            "int64"
        );
        assert_eq!(LogicalType::Float { bits: 64 }.display_name(), "float64");
        assert_eq!(
            LogicalType::Decimal {
                precision: 10,
                scale: 2
            }
            .display_name(),
            "decimal(10,2)"
        );
        assert_eq!(LogicalType::Boolean.display_name(), "bool");
        assert_eq!(
            LogicalType::Timestamp {
                unit: TimeUnit::Microsecond,
                timezone: Some("UTC".into())
            }
            .display_name(),
            "timestamp(UTC)"
        );
        assert_eq!(
            LogicalType::Duration {
                unit: TimeUnit::Second
            }
            .display_name(),
            "duration"
        );
        assert_eq!(LogicalType::Interval.display_name(), "interval");
        assert_eq!(LogicalType::List.display_name(), "list");
        assert_eq!(LogicalType::Struct.display_name(), "struct");
        assert_eq!(LogicalType::Map.display_name(), "map");
    }

    #[test]
    fn scalar_domains_are_parameter_free() {
        use DataType::*;
        for (data_type, domain) in [
            (Utf8, Domain::Text),
            (Boolean, Domain::Boolean),
            (Int8, Domain::Integer),
            (Float64, Domain::Float),
            (Decimal128(10, 2), Domain::Decimal),
            (Date32, Domain::Date),
            (Time32(TimeUnit::Second), Domain::Time),
            (Timestamp(TimeUnit::Second, None), Domain::Timestamp),
            (Duration(TimeUnit::Second), Domain::Duration),
            (Binary, Domain::Binary),
        ] {
            assert_eq!(
                LogicalType::from_arrow(&data_type).unwrap().domain(),
                Some(domain),
                "{data_type}"
            );
        }
        for nothing in [
            Null,
            List(field("item", Int32)),
            Struct(Fields::from(vec![field("f", Int32)])),
        ] {
            assert_eq!(LogicalType::from_arrow(&nothing).unwrap().domain(), None);
        }
    }
}
