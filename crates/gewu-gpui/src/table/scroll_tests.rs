use super::*;

const H: f64 = 32.0;
const VIEW: f64 = 768.0;

#[test]
fn a_chart_jump_centers_a_distant_row_and_keeps_visible_rows_still() {
    let mut scroll = VerticalScroll::new(100, H, VIEW);
    scroll.center_if_outside(62);
    assert_eq!(scroll.anchor().row, 50);
    scroll.center_if_outside(51);
    assert_eq!(scroll.anchor().row, 50);
    scroll.center_if_outside(0);
    assert_eq!(scroll.anchor().row, 0);
}

fn accumulate(rows: u64, start: Anchor, step: f64, n: usize) {
    let mut s = VerticalScroll::new(rows, H, VIEW);
    s.set_anchor(start);
    let start = s.anchor();
    // Measure relative to the start so the check itself stays exact at 10^10 rows.
    let moved_from = |s: &VerticalScroll| {
        (s.anchor().row as i128 - start.row as i128) as f64 * H + (s.anchor().offset - start.offset)
    };
    for _ in 0..n {
        s.scroll_by(step);
    }
    let moved = moved_from(&s);
    // Binary fractions (0.5, 0.25, 1.5, 3) are exact; 13.7 is not representable,
    // so allow its own rounding, which does not grow with the row index.
    let tolerance = if (step * 4.0).fract() == 0.0 {
        0.0
    } else {
        1e-6
    };
    assert!(
        (moved - step * n as f64).abs() <= tolerance,
        "rows={rows} start={start:?} step={step} moved={moved}"
    );
    for _ in 0..n {
        s.scroll_by(-step);
    }
    assert!(
        moved_from(&s).abs() <= tolerance,
        "round trip rows={rows} start={start:?}"
    );
}

#[test]
fn half_pixel_deltas_accumulate_at_top_middle_and_bottom() {
    for rows in [2_000_000u64, 50_000_000, 10_000_000_000] {
        let s = VerticalScroll::new(rows, H, VIEW);
        let max = s.max_anchor();
        let bottom = Anchor {
            row: max.row - 100,
            offset: max.offset,
        };
        for start in [
            Anchor {
                row: 0,
                offset: 0.0,
            },
            Anchor {
                row: rows / 2,
                offset: 7.0,
            },
            bottom,
        ] {
            for step in [0.5, -0.5, 0.25, 1.5, 3.0, 13.7] {
                // At the top, scroll down first so negative steps have room.
                let start = if start.row == 0 && step < 0.0 {
                    Anchor {
                        row: 200,
                        offset: 0.0,
                    }
                } else {
                    start
                };
                // Stay within the 100 rows (3200 px) of room before the bottom clamp.
                accumulate(
                    rows,
                    start,
                    step,
                    ((3000.0 / step.abs()) as usize).min(1000),
                );
            }
        }
    }
}

#[test]
fn clamps_at_both_ends() {
    let mut s = VerticalScroll::new(50_000_000, H, VIEW);
    s.scroll_by(-10.0);
    assert_eq!(s.anchor(), Anchor::default());
    s.scroll_by(1e15);
    assert_eq!(s.anchor(), s.max_anchor());
    let last = s.visible_rows();
    assert_eq!(last.end, 50_000_000);
    // Bottom of the last row is exactly the viewport bottom.
    assert_eq!(s.row_top(49_999_999) + H, VIEW);
    s.scroll_by(0.5);
    assert_eq!(s.anchor(), s.max_anchor());
    s.scroll_by(-0.5);
    assert_eq!(s.row_top(49_999_999) + H, VIEW + 0.5);
}

#[test]
fn fractional_viewport_bottom() {
    let mut s = VerticalScroll::new(2_000_000, H, 770.0);
    s.scroll_by(1e12);
    assert_eq!(s.row_top(1_999_999) + H, 770.0);
}

#[test]
fn scrollbar_fraction_maps_to_rows() {
    let mut s = VerticalScroll::new(50_000_000, H, VIEW);
    s.set_fraction(0.5);
    let mid = s.anchor().row;
    assert!((mid as i64 - 24_999_988).abs() <= 1, "{mid}");
    s.set_fraction(1.0);
    assert_eq!(s.anchor(), s.max_anchor());
    assert_eq!(s.fraction(), 1.0);
    s.set_fraction(0.0);
    assert_eq!(s.anchor(), Anchor::default());
}

#[test]
fn reveal_scrolls_minimally() {
    let mut s = VerticalScroll::new(2_000_000, H, VIEW);
    s.reveal(30);
    assert_eq!(s.row_top(30) + H, VIEW);
    s.reveal(5);
    assert_eq!(s.anchor(), Anchor {
        row: 5,
        offset: 0.0
    });
    s.reveal(1_999_999);
    assert_eq!(s.anchor(), s.max_anchor());
    assert_eq!(s.row_at(VIEW - 1.0), Some(1_999_999));
}

#[test]
fn a_subpixel_rounding_boundary_does_not_jump_a_whole_row() {
    let mut scroll = VerticalScroll::new(10_000_000_000, H, VIEW);
    scroll.set_anchor(Anchor {
        row: 2_345_678_901,
        offset: 0.,
    });
    scroll.scroll_by(-f64::EPSILON);
    assert_eq!(scroll.anchor(), Anchor {
        row: 2_345_678_901,
        offset: 0.
    });
}
