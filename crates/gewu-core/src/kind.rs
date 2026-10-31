//! Logical column types preserving semantic parameters from Arrow.
//! Filters, search, histograms and the UI all query this model.

use arrow::datatypes::{DataType, TimeUnit};

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
    /// Unknown or rare physical types, preserving raw type name.
    Other { raw_name: String },
}

impl LogicalType {
    /// From Arrow DataType conversion (masks storage differences while retaining logical
    /// parameters).
    pub fn from_arrow(data_type: &DataType) -> Self {
        use DataType::*;
        match data_type {
            Utf8 | LargeUtf8 | Utf8View => Self::Text,
            Dictionary(_, value) if Self::from_arrow(value) == Self::Text => Self::Text,
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
            Boolean => Self::Boolean,
            Date32 | Date64 => Self::Date,
            Time32(TimeUnit::Second | TimeUnit::Millisecond)
            | Time64(TimeUnit::Microsecond | TimeUnit::Nanosecond) => Self::Time,
            Timestamp(unit, tz) => Self::Timestamp {
                unit: *unit,
                timezone: tz.as_ref().map(|s| s.to_string()),
            },
            Duration(unit) => Self::Duration { unit: *unit },
            Interval(_) => Self::Interval,
            Binary | LargeBinary | BinaryView | FixedSizeBinary(_) => Self::Binary,
            List(_) | LargeList(_) | ListView(_) | LargeListView(_) | FixedSizeList(..) => {
                Self::List
            }
            Struct(_) => Self::Struct,
            _ => Self::Other {
                raw_name: data_type.to_string(),
            },
        }
    }

    /// Ordered single values: range filters and histograms work on these.
    pub fn is_ordered(&self) -> bool {
        matches!(
            self,
            Self::Integer { .. }
                | Self::Float { .. }
                | Self::Decimal { .. }
                | Self::Date
                | Self::Time
                | Self::Timestamp { .. }
                | Self::Duration { .. }
        )
    }

    /// Numeric axes whose positive values can be displayed logarithmically.
    pub fn supports_logarithmic(&self) -> bool {
        matches!(
            self,
            Self::Integer { .. } | Self::Float { .. } | Self::Decimal { .. }
        )
    }

    /// Backwards-compatible alias for `supports_logarithmic`.
    pub fn supports_log(&self) -> bool {
        self.supports_logarithmic()
    }

    /// Dates and times, shown and typed as text but stored as integers.
    pub fn is_temporal(&self) -> bool {
        matches!(
            self,
            Self::Date | Self::Time | Self::Timestamp { .. } | Self::Duration { .. }
        )
    }

    /// Normalized, concise display text for UI badges and type labels.
    pub fn display_name(&self) -> String {
        match self {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::new_null_array,
        datatypes::{Field, IntervalUnit},
    };

    use super::*;
    use crate::analysis::numeric::Numeric;

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
                LogicalType::from_arrow(&t).is_ordered(),
                numeric.is_some(),
                "{t} is {:?}",
                LogicalType::from_arrow(&t)
            );
        }
    }

    #[test]
    fn dictionary_strings_are_text() {
        let t = DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::LargeUtf8));
        assert_eq!(LogicalType::from_arrow(&t), LogicalType::Text);
    }

    #[test]
    fn from_arrow_parameters_preserved() {
        use arrow::datatypes::Field;
        use DataType::*;

        assert_eq!(LogicalType::from_arrow(&Utf8), LogicalType::Text);
        assert_eq!(LogicalType::from_arrow(&LargeUtf8), LogicalType::Text);
        assert_eq!(LogicalType::from_arrow(&Utf8View), LogicalType::Text);

        assert_eq!(LogicalType::from_arrow(&Int8), LogicalType::Integer {
            bits: 8,
            signed: true
        });
        assert_eq!(LogicalType::from_arrow(&Int64), LogicalType::Integer {
            bits: 64,
            signed: true
        });
        assert_eq!(LogicalType::from_arrow(&UInt32), LogicalType::Integer {
            bits: 32,
            signed: false
        });

        assert_eq!(LogicalType::from_arrow(&Float16), LogicalType::Float {
            bits: 16
        });
        assert_eq!(LogicalType::from_arrow(&Float32), LogicalType::Float {
            bits: 32
        });
        assert_eq!(LogicalType::from_arrow(&Float64), LogicalType::Float {
            bits: 64
        });

        assert_eq!(
            LogicalType::from_arrow(&Decimal128(10, 2)),
            LogicalType::Decimal {
                precision: 10,
                scale: 2
            }
        );
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
            LogicalType::from_arrow(&Timestamp(TimeUnit::Nanosecond, None)),
            LogicalType::Timestamp {
                unit: TimeUnit::Nanosecond,
                timezone: None
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
            LogicalType::from_arrow(&List(Arc::new(Field::new("item", Int32, true)))),
            LogicalType::List
        );
        assert_eq!(
            LogicalType::from_arrow(&Struct(vec![Field::new("f", Int32, true)].into())),
            LogicalType::Struct
        );
        assert_eq!(LogicalType::from_arrow(&Null), LogicalType::Other {
            raw_name: "Null".into()
        });
    }

    #[test]
    fn display_name_formatting() {
        assert_eq!(LogicalType::Text.display_name(), "string");
        assert_eq!(
            LogicalType::Integer {
                bits: 64,
                signed: true
            }
            .display_name(),
            "int64"
        );
        assert_eq!(
            LogicalType::Integer {
                bits: 32,
                signed: false
            }
            .display_name(),
            "uint32"
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
        assert_eq!(LogicalType::Date.display_name(), "date");
        assert_eq!(LogicalType::Time.display_name(), "time");
        assert_eq!(
            LogicalType::Timestamp {
                unit: TimeUnit::Microsecond,
                timezone: Some("UTC".into())
            }
            .display_name(),
            "timestamp(UTC)"
        );
        assert_eq!(
            LogicalType::Timestamp {
                unit: TimeUnit::Microsecond,
                timezone: None
            }
            .display_name(),
            "timestamp"
        );
        assert_eq!(
            LogicalType::Duration {
                unit: TimeUnit::Second
            }
            .display_name(),
            "duration"
        );
        assert_eq!(LogicalType::Interval.display_name(), "interval");
        assert_eq!(LogicalType::Binary.display_name(), "binary");
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

    #[test]
    fn classification_predicates() {
        let int = LogicalType::Integer {
            bits: 32,
            signed: true,
        };
        let float = LogicalType::Float { bits: 64 };
        let decimal = LogicalType::Decimal {
            precision: 10,
            scale: 2,
        };
        let date = LogicalType::Date;
        let ts = LogicalType::Timestamp {
            unit: TimeUnit::Second,
            timezone: None,
        };
        let duration = LogicalType::Duration {
            unit: TimeUnit::Second,
        };
        let text = LogicalType::Text;
        let boolean = LogicalType::Boolean;

        assert!(int.is_ordered());
        assert!(float.is_ordered());
        assert!(decimal.is_ordered());
        assert!(date.is_ordered());
        assert!(ts.is_ordered());
        assert!(duration.is_ordered());
        assert!(!text.is_ordered());
        assert!(!boolean.is_ordered());

        assert!(int.supports_logarithmic());
        assert!(float.supports_logarithmic());
        assert!(decimal.supports_logarithmic());
        assert!(!date.supports_logarithmic());
        assert!(!ts.supports_logarithmic());
        assert!(!text.supports_logarithmic());

        assert!(date.is_temporal());
        assert!(ts.is_temporal());
        assert!(duration.is_temporal());
        assert!(!int.is_temporal());
        assert!(!text.is_temporal());
    }
}
