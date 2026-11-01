//! The column type model: four orthogonal dimensions, and the single place
//! that derives what a type supports.
//!
//! A type is described by independent dimensions so that every consumer can
//! ask the question it actually has, instead of matching Arrow's flat
//! [`DataType`] again and again:
//!
//! - [`Shape`]: how many values a cell holds — null, one scalar, a list, a struct or a map. Nested
//!   shapes carry their children.
//! - [`Scalar`]: what a scalar leaf is — text, number, temporal, binary — with the parameters (bit
//!   width, precision and scale, time unit, time zone) that make the value domain and its display
//!   exact.
//! - [`Encoding`]: how Arrow stores the type — directly or as a dictionary. Encoding is storage,
//!   not semantics: a dictionary of text is still text.
//! - [`Domain`]: the parameter-free projection of [`Scalar`] that policy code matches on.
//!
//! Everything a consumer is allowed to do with a column is derived from those
//! dimensions in [`derive`], so "which filters, analyses, charts and indexing
//! apply" has exactly one definition. Kernels still match [`DataType`] to pick
//! a monomorphized native path; that is a physical decision, not a policy one.
//!
//! [`LogicalType`] remains the stable, display-oriented projection used by the
//! FFI and [`ColumnInfo`]; it is derived from this model instead of duplicating
//! its classification.

pub(crate) mod derive;
pub(crate) mod identity;
mod logical;

use std::sync::Arc;

use arrow::datatypes::{DataType, IntervalUnit, TimeUnit};
pub use derive::{AnalysisSection, FindStrategy, IndexStrategy, LengthUnit};
pub use logical::{ColumnCapabilities, ColumnInfo, LogicalType};

/// A column type: one shape in one storage encoding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Type {
    shape: Shape,
    encoding: Encoding,
}

/// What a cell holds.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Shape {
    /// The Arrow null type: every value is missing.
    Null,
    /// Exactly one value per cell.
    Scalar(Scalar),
    /// Elements addressed by offset.
    List {
        element: Box<Type>,
        /// `Some` for fixed-size lists.
        fixed: Option<usize>,
    },
    /// Named, typed children.
    Struct { fields: Arc<[FieldType]> },
    /// Key/value entries.
    Map { key: Box<Type>, value: Box<Type> },
}

/// A struct field.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FieldType {
    pub name: String,
    pub ty: Type,
    pub nullable: bool,
}

/// The semantic class of a scalar value, with the parameters that make its
/// value domain and display exact.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scalar {
    Text,
    Boolean,
    Integer {
        bits: u8,
        signed: bool,
    },
    Float {
        bits: u8,
    },
    Decimal {
        precision: u8,
        scale: i8,
    },
    Date,
    Time {
        unit: TimeUnit,
    },
    Timestamp {
        unit: TimeUnit,
        timezone: Option<Arc<str>>,
    },
    Duration {
        unit: TimeUnit,
    },
    Interval {
        unit: IntervalUnit,
    },
    Binary,
}

/// How Arrow stores a type. Dictionary encoding is lossless and does not
/// change the value domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoding {
    Direct,
    Dictionary,
}

/// The parameter-free scalar class that policy code matches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Domain {
    Text,
    Boolean,
    Integer,
    Float,
    Decimal,
    Date,
    Time,
    Timestamp,
    Duration,
    Interval,
    Binary,
}

/// Scalar parameters as a value, for callers that parse or format one domain
/// without caring which class it belongs to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Params {
    pub bits: Option<u8>,
    pub signed: Option<bool>,
    pub precision: Option<u8>,
    pub scale: Option<i8>,
    pub time_unit: Option<TimeUnit>,
    pub interval_unit: Option<IntervalUnit>,
    pub fixed_size: Option<usize>,
    pub timezone: Option<Arc<str>>,
}

impl Scalar {
    /// The parameter-free class of this scalar.
    pub fn domain(&self) -> Domain {
        match self {
            Self::Text => Domain::Text,
            Self::Boolean => Domain::Boolean,
            Self::Integer { .. } => Domain::Integer,
            Self::Float { .. } => Domain::Float,
            Self::Decimal { .. } => Domain::Decimal,
            Self::Date => Domain::Date,
            Self::Time { .. } => Domain::Time,
            Self::Timestamp { .. } => Domain::Timestamp,
            Self::Duration { .. } => Domain::Duration,
            Self::Interval { .. } => Domain::Interval,
            Self::Binary => Domain::Binary,
        }
    }

    /// This scalar's parameters, with the absent ones left empty.
    pub fn params(&self) -> Params {
        match self {
            Self::Text | Self::Boolean | Self::Date | Self::Binary => Params::default(),
            Self::Integer { bits, signed } => Params {
                bits: Some(*bits),
                signed: Some(*signed),
                ..Params::default()
            },
            Self::Float { bits } => Params {
                bits: Some(*bits),
                ..Params::default()
            },
            Self::Decimal { precision, scale } => Params {
                precision: Some(*precision),
                scale: Some(*scale),
                ..Params::default()
            },
            Self::Time { unit } | Self::Duration { unit } => Params {
                time_unit: Some(*unit),
                ..Params::default()
            },
            Self::Timestamp { unit, timezone } => Params {
                time_unit: Some(*unit),
                timezone: timezone.clone(),
                ..Params::default()
            },
            Self::Interval { unit } => Params {
                interval_unit: Some(*unit),
                ..Params::default()
            },
        }
    }
}

impl Type {
    /// Build the type model for an Arrow type, or `None` when the type has no
    /// Gewu semantics (unions, run-end encoded, and future Arrow additions).
    /// Callers decide policy for unsupported types; [`LogicalType::from_arrow`]
    /// keeps them visible as [`LogicalType::Other`].
    pub fn from_arrow(data_type: &DataType) -> Option<Self> {
        use DataType::*;
        let shape = match data_type {
            Null => Shape::Null,
            Boolean => Shape::Scalar(Scalar::Boolean),
            Int8 => integer(8, true),
            Int16 => integer(16, true),
            Int32 => integer(32, true),
            Int64 => integer(64, true),
            UInt8 => integer(8, false),
            UInt16 => integer(16, false),
            UInt32 => integer(32, false),
            UInt64 => integer(64, false),
            Float16 => Shape::Scalar(Scalar::Float { bits: 16 }),
            Float32 => Shape::Scalar(Scalar::Float { bits: 32 }),
            Float64 => Shape::Scalar(Scalar::Float { bits: 64 }),
            Decimal32(precision, scale)
            | Decimal64(precision, scale)
            | Decimal128(precision, scale)
            | Decimal256(precision, scale) => Shape::Scalar(Scalar::Decimal {
                precision: *precision,
                scale: *scale,
            }),
            Date32 | Date64 => Shape::Scalar(Scalar::Date),
            Time32(unit) | Time64(unit) => Shape::Scalar(Scalar::Time { unit: *unit }),
            Timestamp(unit, timezone) => Shape::Scalar(Scalar::Timestamp {
                unit: *unit,
                timezone: timezone.as_deref().map(Arc::from),
            }),
            Duration(unit) => Shape::Scalar(Scalar::Duration { unit: *unit }),
            Interval(unit) => Shape::Scalar(Scalar::Interval { unit: *unit }),
            Binary | LargeBinary | BinaryView | FixedSizeBinary(_) => Shape::Scalar(Scalar::Binary),
            Utf8 | LargeUtf8 | Utf8View => Shape::Scalar(Scalar::Text),
            List(field) | LargeList(field) | ListView(field) | LargeListView(field) => {
                Shape::List {
                    element: Box::new(Self::from_arrow(field.data_type())?),
                    fixed: None,
                }
            }
            FixedSizeList(field, size) => Shape::List {
                element: Box::new(Self::from_arrow(field.data_type())?),
                fixed: Some(*size as usize),
            },
            Struct(fields) => Shape::Struct {
                fields: fields
                    .iter()
                    .map(|field| {
                        Some(FieldType {
                            name: field.name().clone(),
                            ty: Self::from_arrow(field.data_type())?,
                            nullable: field.is_nullable(),
                        })
                    })
                    .collect::<Option<Vec<_>>>()?
                    .into(),
            },
            Map(field, _) => {
                let Struct(children) = field.data_type() else {
                    return None;
                };
                let [key, value] = children.iter().collect::<Vec<_>>()[..] else {
                    return None;
                };
                Shape::Map {
                    key: Box::new(Self::from_arrow(key.data_type())?),
                    value: Box::new(Self::from_arrow(value.data_type())?),
                }
            }
            Dictionary(_, values) => {
                return Self::from_arrow(values).map(|ty| ty.in_encoding(Encoding::Dictionary));
            }
            _ => return None,
        };
        Some(Self {
            shape,
            encoding: Encoding::Direct,
        })
    }

    /// The type model with the given storage encoding. Only dictionary
    /// encoding can be applied; repeating it is a no-op.
    fn in_encoding(self, encoding: Encoding) -> Self {
        Self {
            shape: self.shape,
            encoding,
        }
    }

    #[inline]
    pub fn shape(&self) -> &Shape {
        &self.shape
    }

    #[inline]
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    #[inline]
    pub fn is_dictionary(&self) -> bool {
        self.encoding == Encoding::Dictionary
    }

    /// The parameter-free scalar class, for policy dispatch.
    pub fn domain(&self) -> Option<Domain> {
        match &self.shape {
            Shape::Scalar(scalar) => Some(scalar.domain()),
            _ => None,
        }
    }

    /// The scalar parameters, if this type is a scalar.
    pub fn params(&self) -> Option<Params> {
        match &self.shape {
            Shape::Scalar(scalar) => Some(scalar.params()),
            _ => None,
        }
    }

    /// This type as a value type: the same shape without dictionary encoding.
    pub fn value_type(&self) -> Self {
        Self {
            shape: self.shape.clone(),
            encoding: Encoding::Direct,
        }
    }

    /// The list element type.
    pub fn element(&self) -> Option<&Type> {
        match &self.shape {
            Shape::List { element, .. } => Some(element),
            _ => None,
        }
    }

    /// The struct fields.
    pub fn fields(&self) -> &[FieldType] {
        match &self.shape {
            Shape::Struct { fields } => fields,
            _ => &[],
        }
    }

    /// The map key and value types.
    pub fn map_key_value(&self) -> Option<(&Type, &Type)> {
        match &self.shape {
            Shape::Map { key, value } => Some((key, value)),
            _ => None,
        }
    }
}

fn integer(bits: u8, signed: bool) -> Shape {
    Shape::Scalar(Scalar::Integer { bits, signed })
}

#[cfg(test)]
mod tests {
    use arrow::datatypes::{Field, Fields, UnionFields};

    use super::*;

    fn item() -> Arc<Field> {
        Arc::new(Field::new("item", DataType::Int32, true))
    }

    #[test]
    fn supported_types_are_modeled() {
        let types = [
            DataType::Null,
            DataType::Boolean,
            DataType::Int64,
            DataType::UInt8,
            DataType::Float16,
            DataType::Float64,
            DataType::Decimal128(38, 2),
            DataType::Date32,
            DataType::Date64,
            DataType::Time64(TimeUnit::Nanosecond),
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            DataType::Duration(TimeUnit::Second),
            DataType::Interval(IntervalUnit::DayTime),
            DataType::Utf8,
            DataType::LargeUtf8,
            DataType::Binary,
            DataType::FixedSizeBinary(4),
            DataType::List(item()),
            DataType::LargeList(item()),
            DataType::FixedSizeList(item(), 3),
            DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::Utf8)),
        ];
        for data_type in types {
            assert!(Type::from_arrow(&data_type).is_some(), "{data_type}");
        }
    }

    #[test]
    fn unsupported_types_keep_their_logical_projection() {
        for data_type in [
            DataType::RunEndEncoded(
                Arc::new(Field::new("run_ends", DataType::Int32, false)),
                Arc::new(Field::new("values", DataType::Int32, true)),
            ),
            DataType::Union(UnionFields::empty(), arrow::datatypes::UnionMode::Sparse),
        ] {
            assert_eq!(Type::from_arrow(&data_type), None, "{data_type}");
            assert_eq!(LogicalType::from_arrow(&data_type), LogicalType::Other {
                raw_name: data_type.to_string()
            });
        }
    }

    #[test]
    fn dictionary_encoding_does_not_change_the_value_type() {
        let plain = Type::from_arrow(&DataType::Utf8).unwrap();
        let encoded = Type::from_arrow(&DataType::Dictionary(
            Box::new(DataType::Int32),
            Box::new(DataType::Utf8),
        ))
        .unwrap();
        assert_eq!(encoded.domain(), Some(Domain::Text));
        assert_eq!(encoded.encoding(), Encoding::Dictionary);
        assert_eq!(encoded.value_type(), plain);
        assert!(!plain.is_dictionary());
    }

    #[test]
    fn nested_shapes_carry_their_children() {
        let field = Arc::new(Field::new("item", DataType::Utf8, true));
        let list = Type::from_arrow(&DataType::List(field)).unwrap();
        assert_eq!(list.element().unwrap().domain(), Some(Domain::Text));

        let fields = Fields::from(vec![
            Field::new("a", DataType::Int32, false),
            Field::new("b", DataType::Boolean, true),
        ]);
        let structure = Type::from_arrow(&DataType::Struct(fields)).unwrap();
        let names: Vec<_> = structure.fields().iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert!(!structure.fields()[0].nullable);
        assert_eq!(structure.fields()[1].ty.domain(), Some(Domain::Boolean));

        let map = Type::from_arrow(&DataType::Map(
            Arc::new(Field::new(
                "entries",
                DataType::Struct(Fields::from(vec![
                    Field::new("keys", DataType::Utf8, false),
                    Field::new("values", DataType::Int32, true),
                ])),
                false,
            )),
            false,
        ))
        .unwrap();
        let (key, value) = map.map_key_value().unwrap();
        assert_eq!(key.domain(), Some(Domain::Text));
        assert_eq!(value.domain(), Some(Domain::Integer));
    }

    #[test]
    fn scalar_parameters_are_available_without_matching_the_class() {
        let ty = Type::from_arrow(&DataType::Timestamp(
            TimeUnit::Millisecond,
            Some("Europe/London".into()),
        ))
        .unwrap();
        let params = ty.params().unwrap();
        assert_eq!(params.time_unit, Some(TimeUnit::Millisecond));
        assert_eq!(params.timezone.as_deref(), Some("Europe/London"));

        let decimal = Type::from_arrow(&DataType::Decimal256(76, -4)).unwrap();
        let params = decimal.params().unwrap();
        assert_eq!(params.precision, Some(76));
        assert_eq!(params.scale, Some(-4));

        assert_eq!(
            Type::from_arrow(&DataType::Int32).unwrap().params(),
            Some(Params {
                bits: Some(32),
                signed: Some(true),
                ..Params::default()
            })
        );
        assert_eq!(
            Type::from_arrow(&DataType::Int32).unwrap().domain(),
            Some(Domain::Integer)
        );
        assert!(Type::from_arrow(&DataType::List(item()))
            .unwrap()
            .params()
            .is_none());
    }
}
