use gewu_core::TimeUnit;

use super::*;

#[test]
fn numeric_ticks_use_decimal_intervals_without_losing_extreme_domains() {
    assert_eq!(numeric_ticks(3., 93., 4, false), [20., 40., 60., 80.]);
    assert_eq!(numeric_ticks(-9., 21., 4, false), [0., 10., 20.]);
    assert_eq!(numeric_ticks(3., 4., 4, true), [3., 4.]);
    assert_eq!(numeric_ticks(-f64::MAX, f64::MAX, 3, false), [
        -f64::MAX,
        f64::MAX
    ]);
}
#[test]
fn log_ticks_stay_in_original_units_and_temporal_axes_in_seconds() {
    assert_eq!(logarithmic_ticks(1e-4, 1e10, 3), [1e-4, 1e3, 1e10]);
    for (lo, hi) in [(2., 3.), (7., 7.), (f64::from_bits(1), f64::MAX)] {
        let ticks = logarithmic_ticks(lo, hi, 4);
        assert!(
            !ticks.is_empty()
                && ticks.len() <= 4
                && ticks.iter().all(|v| v.is_finite() && *v >= lo && *v <= hi)
        );
    }
    let day = seconds(19723., ScatterAxisKind::Date {
        unit: TimeUnit::Day,
    });
    assert_eq!(day, 1704067200.);
    assert_eq!(
        seconds(1704067200123., ScatterAxisKind::DateTime {
            unit: TimeUnit::Millisecond
        }),
        1704067200.123
    );
    let x = Axis::for_points(
        &[0., 1.],
        |i| i.to_string(),
        ScatterAxisKind::Number,
        AxisScale::Linear,
        true,
        3,
    )
    .unwrap();
    assert_eq!(x.ticks[0].label.text(0), "0");
    assert_eq!(x.ticks[0].label.text(1), "1");
}

#[test]
fn log_ticks_format_ordinary_magnitudes_without_scientific_notation() {
    let ordinary = Axis::for_points(
        &[10.2, 19.2],
        |i| ["10.2", "19.2"][i].to_owned(),
        ScatterAxisKind::Number,
        AxisScale::Log10,
        false,
        4,
    )
    .unwrap();
    assert_eq!(
        ordinary
            .ticks
            .iter()
            .map(|t| t.label.text(0))
            .collect::<Vec<_>>(),
        ["10.2", "19.2"]
    );

    let decades = Axis::for_points(
        &[1., 1000.],
        |i| ["1", "1000"][i].to_owned(),
        ScatterAxisKind::Number,
        AxisScale::Log10,
        false,
        4,
    )
    .unwrap();
    assert_eq!(
        decades
            .ticks
            .iter()
            .map(|t| t.label.text(0))
            .collect::<Vec<_>>(),
        ["1", "10", "100", "1000"]
    );

    let extreme = Axis::for_points(
        &[1e-4, 1e10],
        |i| ["1e-4", "1e10"][i].to_owned(),
        ScatterAxisKind::Number,
        AxisScale::Log10,
        false,
        3,
    )
    .unwrap();
    assert_eq!(
        extreme
            .ticks
            .iter()
            .map(|t| t.label.text(0))
            .collect::<Vec<_>>(),
        ["1e-4", "1000", "1e10"]
    );
}
