//! Bounded scatter snapshots; workers compute points, UI chooses presentation.

use gewu_core as core;

/// Axis presentation semantics supplied with the scatter snapshot.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum ScatterAxisKind {
    Number,
    Date,
    DateTime,
}

impl From<core::ScatterAxisKind> for ScatterAxisKind {
    fn from(kind: core::ScatterAxisKind) -> Self {
        match kind {
            core::ScatterAxisKind::Number => Self::Number,
            core::ScatterAxisKind::Date => Self::Date,
            core::ScatterAxisKind::DateTime => Self::DateTime,
        }
    }
}

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
    pub x_axis_kind: ScatterAxisKind,
    pub y_axis_kind: ScatterAxisKind,
    pub eligible_rows: u64,
    pub non_positive_rows: u64,
    pub points: Vec<ScatterPoint>,
}

impl From<core::ScatterResult> for ScatterResult {
    fn from(result: core::ScatterResult) -> Self {
        Self {
            x_axis_kind: result.x_axis_kind.into(),
            y_axis_kind: result.y_axis_kind.into(),
            eligible_rows: result.eligible_rows,
            non_positive_rows: result.non_positive_rows,
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
        }
    }
}
