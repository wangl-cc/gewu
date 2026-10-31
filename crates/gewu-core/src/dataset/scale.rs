//! Numeric plotting domains; transformations never change source data.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum AxisScale {
    #[default]
    Linear,
    /// Base-10 logarithmic coordinates over finite, strictly positive values.
    Log10,
}
