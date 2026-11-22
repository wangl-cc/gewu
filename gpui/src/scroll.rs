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

    pub fn set_row_height(&mut self, height: f64) {
        self.anchor.offset *= height / self.row_height;
        self.row_height = height;
        self.clamp();
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
        let mut whole = (total / h).floor();
        let mut offset = total - whole * h;
        if offset >= h {
            // Rounding of `total / h` just below an integer.
            offset -= h;
            whole += 1.;
        }
        let row = i128::from(self.anchor.row).saturating_add(whole as i128);
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
        ((i128::from(row) - i128::from(self.anchor.row)) as f64) * self.row_height
            - self.anchor.offset
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
#[path = "scroll_tests.rs"]
mod tests;
