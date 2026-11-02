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
    use crate::types::{direct_domain, Domain};
    matches!(
        direct_domain(data_type),
        Some(Domain::Integer | Domain::Float | Domain::Decimal)
    )
}
