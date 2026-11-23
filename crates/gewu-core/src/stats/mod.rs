//! Analysis modules and their public result projections.

pub(crate) mod analysis;
mod error;
mod histogram;
mod lengths;
mod median;
pub(crate) mod numeric;
mod scale;
mod scan;
pub(crate) mod scatter;
mod values;

pub use analysis::{AnalysisCapabilities, AnalysisField};
pub(crate) use analysis::{capabilities, count_nulls};
pub(crate) use error::Result;
pub use error::{AnalysisError, AnalysisOperation};
pub use histogram::{Histogram, RangeBound, RangeBounds, RangeHistogram};
pub(crate) use histogram::{
    Output as HistogramOutput, build_scaled, counts_scaled, supports_histogram,
};
pub(crate) use lengths::analyze as analyze_lengths;
pub use lengths::{LengthSummary, LengthUnit};
pub use numeric::NumericStatistics;
pub(crate) use numeric::analyze as analyze_statistics;
pub use scale::AxisScale;
pub(crate) use scale::supports_logarithmic;
pub use scatter::{MAX_SCATTER_POINTS, ScatterAxisKind, ScatterPoint, ScatterResult, TimeUnit};
pub(crate) use values::analyze as analyze_values;
pub use values::{ValueFrequency, ValueSummary};
