//! Borrowed arrays and selected rows shared by analysis kernels.

use arrow::{array::ArrayRef, datatypes::DataType};

use super::{
    histogram, lengths, numeric, values, AxisScale, Histogram, LengthSummary, NumericStatistics,
    ValueSummary,
};
use crate::{scope::Selection, Cancellation, Result};

/// A borrowed view of an Arrow array over a row selection.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArrayView<'a> {
    pub(crate) array: &'a ArrayRef,
    pub(crate) rows: &'a Selection,
}

impl<'a> ArrayView<'a> {
    #[inline]
    pub(crate) fn new(array: &'a ArrayRef, rows: &'a Selection) -> Self {
        Self { array, rows }
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    #[inline]
    pub(crate) fn data_type(&self) -> &DataType {
        self.array.data_type()
    }

    /// Numeric statistics (min, max, mean, std dev, median, etc.).
    #[inline]
    pub(crate) fn numeric_statistics(&self, context: &Cancellation) -> Result<NumericStatistics> {
        numeric::analyze(*self, context)
    }

    /// Histogram distribution with the given bins and scale.
    #[inline]
    pub(crate) fn histogram(
        &self,
        bins: u32,
        scale: AxisScale,
        context: &Cancellation,
    ) -> Result<Histogram> {
        histogram::counts_scaled(*self, bins, scale, context)
    }

    /// Text or binary length statistics.
    #[inline]
    pub(crate) fn length_summary(
        &self,
        bins: u32,
        context: &Cancellation,
    ) -> Result<LengthSummary> {
        lengths::analyze(*self, bins, context)
    }

    /// Distinct value frequency summary via sketch.
    #[inline]
    pub(crate) fn value_summary(&self, context: &Cancellation) -> Result<ValueSummary> {
        values::analyze(*self, context)
    }
}
