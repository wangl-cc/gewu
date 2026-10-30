//! Bounded scatter snapshots; workers compute points, UI chooses presentation.

use gewu_core as core;

use crate::GewuError;

#[derive(uniffi::Record)]
pub struct ScatterPoint {
    pub source_row: u64,
    pub x: f64,
    pub y: f64,
    pub x_label: String,
    pub y_label: String,
}

#[derive(uniffi::Record)]
pub struct ScatterResult {
    pub version: u64,
    pub x_column: Option<u32>,
    pub y_column: u32,
    pub total_rows: u64,
    pub eligible_rows: u64,
    pub non_positive_rows: u64,
    pub sampled: bool,
    pub points: Vec<ScatterPoint>,
}

#[derive(uniffi::Object)]
pub struct ScatterAnalysis {
    pub(crate) inner: core::ScatterAnalysis,
}

#[uniffi::export]
impl ScatterAnalysis {
    /// Run off the UI thread. Coordinates may round; labels retain native
    /// precision. Filtering applies; null, non-finite and values outside the
    /// selected logarithmic domains are excluded before sampling.
    pub fn run(&self) -> Result<ScatterResult, GewuError> {
        let result = self.inner.run()?;
        Ok(ScatterResult {
            version: result.version,
            x_column: result.x_column,
            y_column: result.y_column,
            total_rows: result.total_rows,
            eligible_rows: result.eligible_rows,
            non_positive_rows: result.non_positive_rows,
            sampled: result.sampled,
            points: result
                .points
                .into_iter()
                .map(|point| ScatterPoint {
                    source_row: point.source_row,
                    x: point.x,
                    y: point.y,
                    x_label: point.x_label,
                    y_label: point.y_label,
                })
                .collect(),
        })
    }

    /// Terminal cancellation independent of table queries and other jobs.
    pub fn cancel(&self) {
        self.inner.cancel();
    }
}
