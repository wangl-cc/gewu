//! Analysis modules and their public result projections.

pub(crate) mod analysis;
mod array_view;
mod histogram;
mod lengths;
mod median;
pub(crate) mod numeric;
mod scale;
pub(crate) mod scatter;
mod values;

pub(crate) use analysis::capabilities;
pub use analysis::{AnalysisCapabilities, AnalysisField};
pub(crate) use array_view::ArrayView;
pub(crate) use histogram::{build_scaled, supports_histogram};
pub use histogram::{Histogram, RangeBound, RangeBounds, RangeHistogram};
pub use lengths::{LengthSummary, LengthUnit};
pub use numeric::NumericStatistics;
pub(crate) use scale::supports_logarithmic;
pub use scale::AxisScale;
pub use scatter::{ScatterAxisKind, ScatterPoint, ScatterResult, MAX_SCATTER_POINTS};
pub use values::{ValueFrequency, ValueSummary};
