use std::sync::Arc;

use arrow::array::{Date32Array, Float64Array, Int64Array, StringArray, TimestampNanosecondArray};

use super::*;

#[test]
fn axis_kinds_describe_the_snapshot_even_without_points() {
    let dates: ArrayRef = Arc::new(Date32Array::from(vec![1]));
    let timestamps: ArrayRef = Arc::new(TimestampNanosecondArray::from(vec![0]));
    let c0 = ColumnId::new(0);
    let c1 = ColumnId::new(1);
    let result = analyze(
        Some((c0, &dates)),
        (c1, &timestamps),
        RowSlice::Selected(&[]),
        8,
        AxisScale::Linear,
        AxisScale::Linear,
    )
    .unwrap();
    assert!(result.points.is_empty());
    assert_eq!(result.x_axis_kind, ScatterAxisKind::Date {
        unit: TimeUnit::Day
    });
    assert_eq!(result.y_axis_kind, ScatterAxisKind::DateTime {
        unit: TimeUnit::Nanosecond
    });
    let result = analyze(
        None,
        (c0, &dates),
        RowSlice::All(1),
        8,
        AxisScale::Linear,
        AxisScale::Linear,
    )
    .unwrap();
    assert_eq!(result.x_axis_kind, ScatterAxisKind::Number);
    assert_eq!((result.points[0].x, result.points[0].y), (0.0, 1.0));
    for (x, y) in [
        (AxisScale::Log10, AxisScale::Linear),
        (AxisScale::Linear, AxisScale::Log10),
    ] {
        assert!(matches!(
            analyze(None, (c0, &dates), RowSlice::All(1), 8, x, y),
            Err(crate::AnalysisError::UnsupportedOperation { .. }
                | crate::AnalysisError::UnsupportedAxisScale { .. })
        ));
    }
}

#[test]
fn log_axes_classify_pairs_independently_and_keep_original_values() {
    let x: ArrayRef = Arc::new(Float64Array::from(vec![
        None,
        Some(f64::NAN),
        Some(f64::INFINITY),
        Some(0.0),
        Some(3.0),
        Some(-1.0),
        Some(2.0),
        Some(4.0),
    ]));
    let y: ArrayRef = Arc::new(Float64Array::from(vec![
        Some(1.0),
        Some(1.0),
        Some(1.0),
        Some(-1.0),
        Some(2.0),
        Some(-2.0),
        Some(0.0),
        None,
    ]));
    let c0 = ColumnId::new(0);
    let c1 = ColumnId::new(1);
    for (xs, ys, eligible, excluded) in [
        (AxisScale::Linear, AxisScale::Linear, 4, 0),
        (AxisScale::Log10, AxisScale::Linear, 2, 2),
        (AxisScale::Linear, AxisScale::Log10, 1, 3),
        (AxisScale::Log10, AxisScale::Log10, 1, 3),
    ] {
        let result = analyze(Some((c0, &x)), (c1, &y), RowSlice::All(8), 8, xs, ys).unwrap();
        assert_eq!(
            (result.eligible_rows, result.non_positive_rows),
            (eligible, excluded)
        );
        let point = result.points.iter().find(|p| p.source_row == 4).unwrap();
        assert_eq!((point.x, point.y), (3.0, 2.0));
        assert_eq!((&*point.x_label, &*point.y_label), ("3.0", "2.0"));
    }
}

#[test]
fn log_domain_is_applied_before_bounded_sampling() {
    let array: ArrayRef = Arc::new(Int64Array::from_iter_values((0..1000).map(|r| {
        if r >= 990 {
            r
        } else {
            -1
        }
    })));
    let c0 = ColumnId::new(0);
    let make = |rows: RowSlice<'_>| {
        analyze(
            None,
            (c0, &array),
            rows,
            5,
            AxisScale::Linear,
            AxisScale::Log10,
        )
        .unwrap()
    };
    let result = make(RowSlice::All(1000));
    assert_eq!((result.eligible_rows, result.non_positive_rows), (10, 990));
    assert!(result.eligible_rows > result.points.len() as u64);
    assert_eq!(result.points.len(), 5);
    assert!(result
        .points
        .iter()
        .all(|p| p.source_row >= 990 && p.y >= 990.0));
    let ids = |r: ScatterResult| {
        r.points
            .into_iter()
            .map(|p| p.source_row)
            .collect::<Vec<_>>()
    };
    let rev: Vec<u32> = (0..1000).rev().collect();
    assert_eq!(ids(result), ids(make(RowSlice::Selected(&rev))));
    let empty = make(RowSlice::All(990));
    assert_eq!((empty.eligible_rows, empty.non_positive_rows), (0, 990));
    assert!(empty.points.is_empty() && empty.eligible_rows == 0);
}

#[test]
fn scatter_preserves_pairs_filters_invalid_values_and_limits_memory() {
    let x: ArrayRef = Arc::new(Int64Array::from(vec![
        Some(9007199254740993),
        Some(2),
        Some(3),
        None,
        Some(5),
    ]));
    let y: ArrayRef = Arc::new(Float64Array::from(vec![
        1.5,
        f64::NAN,
        f64::INFINITY,
        4.0,
        5.5,
    ]));
    let c0 = ColumnId::new(0);
    let c1 = ColumnId::new(1);
    let result = analyze(
        Some((c0, &x)),
        (c1, &y),
        RowSlice::Selected(&[4, 3, 2, 1, 0]),
        8,
        AxisScale::Linear,
        AxisScale::Linear,
    )
    .unwrap();
    assert_eq!((result.eligible_rows, result.points.len()), (2, 2));
    assert_eq!(
        result
            .points
            .iter()
            .map(|p| p.source_row)
            .collect::<Vec<_>>(),
        [0, 4]
    );
    assert_eq!(result.points[0].x_label, "9007199254740993");
    assert_eq!((result.points[0].y, result.points[1].y), (1.5, 5.5));
    let row_axis = analyze(
        None,
        (c1, &y),
        RowSlice::Selected(&[4, 0]),
        8,
        AxisScale::Linear,
        AxisScale::Linear,
    )
    .unwrap();
    assert_eq!(
        (row_axis.points[1].x, row_axis.points[1].x_label.as_str()),
        (4.0, "4")
    );
    for limit in [0, MAX_SCATTER_POINTS + 1] {
        assert!(matches!(
            analyze(
                None,
                (c1, &y),
                RowSlice::Selected(&[0]),
                limit,
                AxisScale::Linear,
                AxisScale::Linear,
            ),
            Err(crate::AnalysisError::InvalidScatterLimit { .. })
        ));
    }
    let text: ArrayRef = Arc::new(StringArray::from(vec!["bad"]));
    assert!(matches!(
        analyze(
            None,
            (c0, &text),
            RowSlice::All(1),
            1,
            AxisScale::Linear,
            AxisScale::Linear,
        ),
        Err(crate::AnalysisError::UnsupportedOperation { .. })
    ));
    let many: ArrayRef = Arc::new(Int64Array::from_iter_values(0..20000));
    let sample = |rows: RowSlice<'_>| {
        analyze(
            None,
            (c0, &many),
            rows,
            64,
            AxisScale::Linear,
            AxisScale::Linear,
        )
    };
    let first = sample(RowSlice::All(20000)).unwrap();
    let rev: Vec<u32> = (0..20000).rev().collect();
    let reversed = sample(RowSlice::Selected(&rev)).unwrap();
    assert!(first.eligible_rows > first.points.len() as u64);
    assert_eq!((first.eligible_rows, first.points.len()), (20000, 64));
    assert_eq!(
        first
            .points
            .iter()
            .map(|p| p.source_row)
            .collect::<Vec<_>>(),
        reversed
            .points
            .iter()
            .map(|p| p.source_row)
            .collect::<Vec<_>>()
    );
}
