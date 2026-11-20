//! Bounded scatter samples with temporal coordinates in seconds.

use gewu_core as core;

/// How to read an axis's coordinates. Temporal coordinates are seconds since
/// the Unix epoch, whatever the column's own unit.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum ScatterAxisKind {
    Number,
    Date,
    DateTime,
}

#[derive(uniffi::Record, Clone)]
pub struct ScatterPoint {
    pub source_row: u64,
    pub x: f64,
    pub y: f64,
    pub x_label: String,
    pub y_label: String,
}

#[derive(uniffi::Record, Clone)]
pub struct ScatterResult {
    pub x_axis_kind: ScatterAxisKind,
    pub y_axis_kind: ScatterAxisKind,
    pub eligible_rows: u64,
    pub non_positive_rows: u64,
    pub points: Vec<ScatterPoint>,
}

/// An axis's kind, and the factor converting its coordinates to the units
/// that kind promises.
fn axis(kind: core::ScatterAxisKind) -> (ScatterAxisKind, f64) {
    match kind {
        core::ScatterAxisKind::Number => (ScatterAxisKind::Number, 1.0),
        core::ScatterAxisKind::Date { unit } => (ScatterAxisKind::Date, unit.seconds()),
        core::ScatterAxisKind::DateTime { unit } => (ScatterAxisKind::DateTime, unit.seconds()),
    }
}

impl From<core::ScatterResult> for ScatterResult {
    fn from(result: core::ScatterResult) -> Self {
        let (x_axis_kind, x_factor) = axis(result.x_axis_kind);
        let (y_axis_kind, y_factor) = axis(result.y_axis_kind);
        Self {
            x_axis_kind,
            y_axis_kind,
            eligible_rows: result.eligible_rows,
            non_positive_rows: result.non_positive_rows,
            points: result
                .points
                .into_iter()
                .map(|point| ScatterPoint {
                    source_row: point.source_row,
                    x: point.x * x_factor,
                    y: point.y * y_factor,
                    x_label: point.x_label,
                    y_label: point.y_label,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporal_coordinates_become_seconds() {
        let result = ScatterResult::from(core::ScatterResult {
            x_axis_kind: core::ScatterAxisKind::Date {
                unit: core::TimeUnit::Day,
            },
            y_axis_kind: core::ScatterAxisKind::DateTime {
                unit: core::TimeUnit::Millisecond,
            },
            eligible_rows: 1,
            non_positive_rows: 0,
            points: vec![core::ScatterPoint {
                source_row: 0,
                x: 2.0,
                y: 1500.0,
                x_label: String::new(),
                y_label: String::new(),
            }],
        });
        assert!(result.x_axis_kind == ScatterAxisKind::Date);
        assert!(result.y_axis_kind == ScatterAxisKind::DateTime);
        assert_eq!((result.points[0].x, result.points[0].y), (172_800.0, 1.5));
    }
}
