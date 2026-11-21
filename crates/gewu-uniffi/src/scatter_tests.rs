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
