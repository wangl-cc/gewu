//! The stable, display-oriented projection of [`Type`].
//!
//! [`LogicalType`] is what the FFI, [`ColumnInfo`] and the app see: it keeps
//! semantic parameters and the dictionary encoding so type badges and labels
//! stay informative. It is derived from [`Type`] and must not be used for
//! policy decisions — those live in [`super::derive`].

use arrow::datatypes::{DataType, TimeUnit};

use super::{Domain, Encoding, Scalar, Shape, Type};

/// A logical data type with semantic parameters preserved from Arrow.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LogicalType {
    /// Strings, including dictionary-encoded strings.
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
    /// Categorical enumeration encoded as a dictionary of distinct values.
    Dictionary { value_type: Box<LogicalType> },
    /// Unknown or rare physical types, preserving raw type name.
    Other { raw_name: String },
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

/// Metadata describing a single column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub logical_type: LogicalType,
    pub display_type: String,
    pub technical_type: String,
    pub nullable: bool,
}

impl LogicalType {
    /// Build the display projection from Arrow, preserving parameters and the
    /// dictionary encoding. Types the model does not support remain visible as
    /// [`Self::Other`], exactly as before the model existed.
    pub fn from_arrow(data_type: &DataType) -> Self {
        Type::from_arrow(data_type)
            .and_then(|ty| ty.logical())
            .unwrap_or_else(|| Self::Other {
                raw_name: data_type.to_string(),
            })
    }

    /// This projection's parameter-free domain, ignoring the encoding.
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
            Self::Dictionary { value_type } => return value_type.domain(),
            Self::List | Self::Struct | Self::Other { .. } => return None,
        })
    }

    /// Normalized, concise display text for UI badges and type labels.
    pub fn display_name(&self) -> String {
        match self {
            Self::Dictionary { value_type } => format!("dict<{}>", value_type.display_name()),
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
            Self::Other { raw_name } => raw_name.clone(),
        }
    }
}

impl Type {
    /// The display projection of this type, or `None` for shapes that only have
    /// an [`LogicalType::Other`] representation (null, map).
    pub fn logical(&self) -> Option<LogicalType> {
        let value = match &self.shape {
            Shape::Null | Shape::Map { .. } => return None,
            Shape::Scalar(scalar) => scalar_logical(scalar),
            Shape::List { .. } => LogicalType::List,
            Shape::Struct { .. } => LogicalType::Struct,
        };
        Some(match self.encoding {
            Encoding::Direct => value,
            Encoding::Dictionary => LogicalType::Dictionary {
                value_type: Box::new(value),
            },
        })
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

        assert_eq!(LogicalType::from_arrow(&Utf8), LogicalType::Text);
        assert_eq!(LogicalType::from_arrow(&LargeUtf8), LogicalType::Text);
        assert_eq!(LogicalType::from_arrow(&Utf8View), LogicalType::Text);
        assert_eq!(LogicalType::from_arrow(&Int8), LogicalType::Integer {
            bits: 8,
            signed: true
        });
        assert_eq!(LogicalType::from_arrow(&UInt32), LogicalType::Integer {
            bits: 32,
            signed: false
        });
        assert_eq!(LogicalType::from_arrow(&Float16), LogicalType::Float {
            bits: 16
        });
        assert_eq!(
            LogicalType::from_arrow(&Decimal256(76, -4)),
            LogicalType::Decimal {
                precision: 76,
                scale: -4
            }
        );
        assert_eq!(LogicalType::from_arrow(&Boolean), LogicalType::Boolean);
        assert_eq!(LogicalType::from_arrow(&Date32), LogicalType::Date);
        assert_eq!(LogicalType::from_arrow(&Date64), LogicalType::Date);
        assert_eq!(
            LogicalType::from_arrow(&Time32(TimeUnit::Second)),
            LogicalType::Time
        );
        assert_eq!(
            LogicalType::from_arrow(&Timestamp(TimeUnit::Microsecond, Some("UTC".into()))),
            LogicalType::Timestamp {
                unit: TimeUnit::Microsecond,
                timezone: Some("UTC".into())
            }
        );
        assert_eq!(
            LogicalType::from_arrow(&Duration(TimeUnit::Millisecond)),
            LogicalType::Duration {
                unit: TimeUnit::Millisecond
            }
        );
        assert_eq!(
            LogicalType::from_arrow(&Interval(IntervalUnit::DayTime)),
            LogicalType::Interval
        );
        assert_eq!(LogicalType::from_arrow(&Binary), LogicalType::Binary);
        assert_eq!(
            LogicalType::from_arrow(&List(field("item", Int32))),
            LogicalType::List
        );
        assert_eq!(
            LogicalType::from_arrow(&Struct(Fields::from(vec![field("f", Int32)]))),
            LogicalType::Struct
        );
        assert_eq!(LogicalType::from_arrow(&Null), LogicalType::Other {
            raw_name: "Null".into()
        });
    }

    #[test]
    fn dictionary_projection_keeps_the_value_type() {
        for (data_type, expected) in [
            (
                DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::LargeUtf8)),
                LogicalType::Text,
            ),
            (
                DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Int64)),
                LogicalType::Integer {
                    bits: 64,
                    signed: true,
                },
            ),
        ] {
            assert_eq!(
                LogicalType::from_arrow(&data_type),
                LogicalType::Dictionary {
                    value_type: Box::new(expected)
                }
            );
        }
    }

    #[test]
    fn map_projects_to_other_and_keeps_its_raw_name() {
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
        assert_eq!(LogicalType::from_arrow(&map), LogicalType::Other {
            raw_name: map.to_string()
        });
        // The model itself still knows the shape, so analysis can follow it.
        assert!(matches!(
            Type::from_arrow(&map).unwrap().shape(),
            Shape::Map { .. }
        ));
    }

    #[test]
    fn display_name_formatting() {
        assert_eq!(LogicalType::Text.display_name(), "string");
        assert_eq!(
            LogicalType::Dictionary {
                value_type: Box::new(LogicalType::Text)
            }
            .display_name(),
            "dict<string>"
        );
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
        assert_eq!(
            LogicalType::Other {
                raw_name: "Null".into()
            }
            .display_name(),
            "Null"
        );
    }
}
