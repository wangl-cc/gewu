use gewu_core::RangeBound;

use super::{
    AxisScale, Histogram, RangeBounds, bin_interval, histogram_label, histogram_peak_label,
    middle_tick,
};

fn bound(coordinate: f64) -> RangeBound {
    let text = coordinate.to_string();
    RangeBound {
        value: text.clone(),
        label: text,
        coordinate,
    }
}

#[test]
fn floating_histogram_hints_use_the_real_exclusive_cut() {
    let low: f64 = 15.4;
    let high: f64 = 15.6;
    let upper = bound(high.next_down());
    let ranges = [
        RangeBounds {
            min: None,
            max: Some(bound(low.next_down())),
        },
        RangeBounds {
            min: Some(bound(low)),
            max: Some(upper.clone()),
        },
        RangeBounds {
            min: Some(bound(high)),
            max: None,
        },
    ];
    assert_eq!(bin_interval(&ranges, 0, true), "< 15.4");
    assert_eq!(bin_interval(&ranges, 1, true), "[15.4, 15.6)");
    assert_eq!(bin_interval(&ranges, 2, true), "≥ 15.6");
    assert_eq!(
        bin_interval(&ranges[..2], 1, true),
        format!("15.4 – {}", upper.label),
    );
}

#[test]
fn integral_histogram_hints_keep_inclusive_bounds() {
    let ranges = [
        RangeBounds {
            min: None,
            max: Some(bound(49999.)),
        },
        RangeBounds {
            min: Some(bound(50000.)),
            max: Some(bound(99999.)),
        },
    ];
    assert_eq!(bin_interval(&ranges, 0, false), "≤ 49999");
    assert_eq!(bin_interval(&ranges, 1, false), "50000 – 99999");
}

#[test]
fn large_histogram_tick_and_endpoint_labels_agree() {
    let histogram = Histogram {
        scale: AxisScale::Linear,
        lo: Some(0.),
        hi: Some(2_000_000.),
        min_label: Some("0".into()),
        max_label: Some("1999999".into()),
        integer: true,
        counts: vec![1_000_000; 2],
        bin_edges: Vec::new(),
        non_positive_count: 0,
        infinite_count: 0,
    };
    assert_eq!(
        middle_tick(&histogram, "0", "1999999"),
        Some((0.5, "1,000,000".into())),
    );
    assert_eq!(histogram_label("1999999"), "1,999,999");
    assert_eq!(histogram_label("-1200.5"), "-1,200.5");
    assert_eq!(histogram_peak_label(50_000, false), "max 50,000");
    assert_eq!(histogram_peak_label(50_000, true), "max 50,000 · log");
}
