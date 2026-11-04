//! Analysis modules and their public result projections.

pub(crate) mod analysis;
mod error;
mod histogram;
mod lengths;
mod median;
pub(crate) mod numeric;
mod scale;
pub(crate) mod scatter;
mod values;

pub(crate) use analysis::{capabilities, count_nulls};
pub use analysis::{AnalysisCapabilities, AnalysisField};
pub(crate) use error::Result;
pub use error::{AnalysisError, AnalysisOperation};
pub(crate) use histogram::{build_scaled, counts_scaled, supports_histogram};
pub use histogram::{Histogram, RangeBound, RangeBounds, RangeHistogram};
pub(crate) use lengths::analyze as analyze_lengths;
pub use lengths::{LengthSummary, LengthUnit};
pub(crate) use numeric::analyze as analyze_statistics;
pub use numeric::NumericStatistics;
pub(crate) use scale::supports_logarithmic;
pub use scale::AxisScale;
pub use scatter::{ScatterAxisKind, ScatterPoint, ScatterResult, MAX_SCATTER_POINTS};
pub(crate) use values::analyze as analyze_values;
pub use values::{ValueFrequency, ValueSummary};
