//! The stable, display-oriented projection of Arrow [`DataType`].
//!
//! [`LogicalType`] is the value semantics of a column: what the FFI, the app
//! and [`crate::ColumnInfo`] show. Storage encoding is deliberately not part of it —
//! a dictionary of text is text with [`super::Encoding::Dictionary`] — so type badges
//! and value editors do not need to recurse through storage wrappers.
//!
//! Operations define their own support and execution rules.

use arrow::datatypes::{DataType, TimeUnit};

use super::is_describable;

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

impl LogicalType {
    /// Build the display projection from Arrow, or `None` when the type has no
    /// Gewu semantics. Documents reject such columns when they are opened, so
    /// a column that exists always has a logical type.
    pub fn from_arrow(data_type: &DataType) -> Option<Self> {
        use DataType::*;
        Some(match data_type {
            Null => Self::Null,
            Boolean => Self::Boolean,
            Int8 => Self::Integer {
                bits: 8,
                signed: true,
            },
            Int16 => Self::Integer {
                bits: 16,
                signed: true,
            },
            Int32 => Self::Integer {
                bits: 32,
                signed: true,
            },
            Int64 => Self::Integer {
                bits: 64,
                signed: true,
            },
            UInt8 => Self::Integer {
                bits: 8,
                signed: false,
            },
            UInt16 => Self::Integer {
                bits: 16,
                signed: false,
            },
            UInt32 => Self::Integer {
                bits: 32,
                signed: false,
            },
            UInt64 => Self::Integer {
                bits: 64,
                signed: false,
            },
            Float16 => Self::Float { bits: 16 },
            Float32 => Self::Float { bits: 32 },
            Float64 => Self::Float { bits: 64 },
            Decimal32(precision, scale)
            | Decimal64(precision, scale)
            | Decimal128(precision, scale)
            | Decimal256(precision, scale) => Self::Decimal {
                precision: *precision,
                scale: *scale,
            },
            Date32 | Date64 => Self::Date,
            Time32(_) | Time64(_) => Self::Time,
            Timestamp(unit, timezone) => Self::Timestamp {
                unit: *unit,
                timezone: timezone.as_ref().map(|tz| tz.to_string()),
            },
            Duration(unit) => Self::Duration { unit: *unit },
            Interval(_) => Self::Interval,
            Binary | LargeBinary | BinaryView | FixedSizeBinary(_) => Self::Binary,
            Utf8 | LargeUtf8 | Utf8View => Self::Text,
            Dictionary(_, values) => return Self::from_arrow(values),
            List(_) | LargeList(_) | ListView(_) | LargeListView(_) | FixedSizeList(..)
                if is_describable(data_type) =>
            {
                Self::List
            }
            Struct(_) if is_describable(data_type) => Self::Struct,
            Map(..) if is_describable(data_type) => Self::Map,
            _ => return None,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::datatypes::{Field, Fields, IntervalUnit};

    use super::*;
    use crate::types::Encoding;

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
        assert_eq!(crate::types::encoding(&dictionary), Encoding::Dictionary);
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
}
