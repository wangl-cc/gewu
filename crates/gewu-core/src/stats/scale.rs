//! Numeric plotting domains; transformations never change source data.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum AxisScale {
    #[default]
    Linear,
    /// Base-10 logarithmic coordinates over finite, strictly positive values.
    Log10,
}

/// Logarithmic axes apply to native numbers, not temporal coordinates.
pub(crate) fn supports_logarithmic(data_type: &arrow::datatypes::DataType) -> bool {
    use arrow::datatypes::DataType::*;
    matches!(
        data_type,
        Int8 | Int16
            | Int32
            | Int64
            | UInt8
            | UInt16
            | UInt32
            | UInt64
            | Float16
            | Float32
            | Float64
            | Decimal32(..)
            | Decimal64(..)
            | Decimal128(..)
            | Decimal256(..)
    )
}
