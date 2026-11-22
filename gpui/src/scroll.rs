//! Vertical scroll position as a row anchor: the top row index plus a pixel
//! offset into that row, in `u64` + `f64`. Unlike a single `f32` pixel offset
//! (GPUI's `ScrollHandle`), sub-pixel deltas accumulate exactly at any row
//! count, because the offset never grows beyond one row height.

/// The first visible row and how far (in px, `0 <= offset < row_height`) its
/// top lies above the viewport top.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Anchor {
    pub row: u64,
    pub offset: f64,
}

#[derive(Clone, Debug)]
pub struct VerticalScroll {
    rows: u64,
    row_height: f64,
    /// Height of the area rows scroll in, in px.
    viewport: f64,
    anchor: Anchor,
}

impl VerticalScroll {
    pub fn new(rows: u64, row_height: f64, viewport: f64) -> Self {
        Self {
            rows,
            row_height,
            viewport,
            anchor: Anchor::default(),
        }
    }

    pub fn anchor(&self) -> Anchor {
        self.anchor
    }

    pub fn rows(&self) -> u64 {
        self.rows
    }

    pub fn row_height(&self) -> f64 {
        self.row_height
    }

    pub fn viewport(&self) -> f64 {
        self.viewport
    }

    /// Keep the anchor's display row when the row count or viewport changes.
    pub fn set_rows(&mut self, rows: u64) {
        self.rows = rows;
        self.clamp();
    }

    pub fn set_viewport(&mut self, viewport: f64) {
        if viewport != self.viewport {
            self.viewport = viewport;
            self.clamp();
        }
    }

    /// The furthest anchor: the last row's bottom on the viewport bottom.
    pub fn max_anchor(&self) -> Anchor {
        let h = self.row_height;
        // Rows that fit entirely in the viewport, and the fraction of one more.
        let fit = self.viewport / h;
        if (self.rows as f64) <= fit {
            return Anchor::default();
        }
        // Position the top `fit` rows above `rows`, relative to row `rows`.
        let back = fit.ceil() as u64;
        let row = self.rows - back;
        Anchor {
            row,
            offset: (back as f64 - fit) * h,
        }
    }

    fn clamp(&mut self) {
        let max = self.max_anchor();
        if self.anchor.row > max.row
            || (self.anchor.row == max.row && self.anchor.offset > max.offset)
        {
            self.anchor = max;
        }
    }

    /// Move the content up by `dy` px (positive shows later rows).
    pub fn scroll_by(&mut self, dy: f64) {
        let h = self.row_height;
        let total = self.anchor.offset + dy;
        let whole = (total / h).floor();
        let mut offset = total - whole * h;
        if offset >= h {
            // Rounding of `total / h` just below an integer.
            offset -= h;
        }
        let row = self.anchor.row as i128 + whole as i128;
        self.anchor = if row < 0 {
            Anchor::default()
        } else {
            Anchor {
                row: row.min(u64::MAX as i128) as u64,
                offset: offset.max(0.0),
            }
        };
        self.clamp();
    }

    pub fn set_anchor(&mut self, anchor: Anchor) {
        self.anchor = anchor;
        self.clamp();
    }

    /// The position in rows, `row + offset / row_height`, as `f64`.
    pub fn position(&self) -> f64 {
        self.anchor.row as f64 + self.anchor.offset / self.row_height
    }

    pub fn max_position_rows(&self) -> f64 {
        self.max_position()
    }

    fn max_position(&self) -> f64 {
        let max = self.max_anchor();
        max.row as f64 + max.offset / self.row_height
    }

    /// The scroll fraction `0..=1`, for the scrollbar.
    pub fn fraction(&self) -> f64 {
        let max = self.max_position();
        if max <= 0.0 {
            0.0
        } else {
            (self.position() / max).clamp(0.0, 1.0)
        }
    }

    /// Jump to a scroll fraction; the scrollbar maps positions to rows.
    pub fn set_fraction(&mut self, fraction: f64) {
        let position = fraction.clamp(0.0, 1.0) * self.max_position();
        let row = position.floor();
        self.set_anchor(Anchor {
            row: row as u64,
            offset: (position - row) * self.row_height,
        });
    }

    /// Display rows that intersect the viewport.
    pub fn visible_rows(&self) -> std::ops::Range<u64> {
        let start = self.anchor.row.min(self.rows);
        let count = ((self.anchor.offset + self.viewport) / self.row_height).ceil() as u64;
        start..start.saturating_add(count).min(self.rows)
    }

    /// Rows that fit fully in the viewport, at least one.
    pub fn page_rows(&self) -> u64 {
        ((self.viewport / self.row_height).floor() as u64).max(1)
    }

    /// Y of a display row's top relative to the viewport top, if near it.
    pub fn row_top(&self, row: u64) -> f64 {
        (row as f64 - self.anchor.row as f64) * self.row_height - self.anchor.offset
    }

    /// The display row at `y` px below the viewport top.
    pub fn row_at(&self, y: f64) -> Option<u64> {
        let rel = ((self.anchor.offset + y) / self.row_height).floor();
        if rel < 0.0 {
            return None;
        }
        let row = self.anchor.row.checked_add(rel as u64)?;
        (row < self.rows).then_some(row)
    }

    /// Scroll the minimum so `row` is fully visible.
    pub fn reveal(&mut self, row: u64) {
        let h = self.row_height;
        if row < self.anchor.row || (row == self.anchor.row && self.anchor.offset > 0.0) {
            self.set_anchor(Anchor { row, offset: 0.0 });
        } else if self.row_top(row) + h > self.viewport {
            // Top so that the row's bottom meets the viewport bottom, relative to `row`.
            let target = h - self.viewport;
            let whole = (target / h).floor();
            let back = (-whole) as u64;
            self.set_anchor(Anchor {
                row: row.saturating_sub(back),
                offset: target - whole * h,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: f64 = 32.0;
    const VIEW: f64 = 768.0;

    fn accumulate(rows: u64, start: Anchor, step: f64, n: usize) {
        let mut s = VerticalScroll::new(rows, H, VIEW);
        s.set_anchor(start);
        let start = s.anchor();
        // Measure relative to the start so the check itself stays exact at 10^10 rows.
        let moved_from = |s: &VerticalScroll| {
            (s.anchor().row as i128 - start.row as i128) as f64 * H
                + (s.anchor().offset - start.offset)
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
}
