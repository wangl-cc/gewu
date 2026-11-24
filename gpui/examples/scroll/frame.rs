//! Read divider and stripe geometry from the completed GPUI scene in device pixels.
//! Inspect the first rasterized frame after each scroll, using Core as the source of expected
//! cells.
use std::path::Path;

use gewu_core::{ColumnId, OrderedView};
use gpui_kit::{test::TestWindowExt, *};

use super::Result;

pub(super) struct Grid {
    pub size: Size<Pixels>,
    pub columns: Vec<(u32, u32)>,
    top: u32,
    bottom: u32,
    pub row_height: u32,
    gutter_left: u32,
}

impl Grid {
    pub fn read(window: &Window) -> Result<Self> {
        let quads = window.painted_quads();
        let scale = window.scale_factor();
        // The data columns share a body mask; the gutter and headers have their own masks.
        let dividers = quads
            .iter()
            .filter(|q| {
                q.bounds.size.width.0 == 1.
                    && q.bounds.size.height.0 > 2. * 32. * scale
                    && q.bounds.origin.y == q.content_mask.bounds.origin.y
                    && q.content_mask.bounds.origin.x.0 > 0.
            })
            .collect::<Vec<_>>();
        let divider = dividers.first().ok_or("no painted grid column dividers")?;
        let body = divider.content_mask.bounds;
        let header = quads
            .iter()
            .find(|q| {
                q.bounds.origin.x == divider.bounds.origin.x
                    && q.bounds.size.width == divider.bounds.size.width
                    && q.bounds.bottom() == divider.bounds.top()
                    && q.bounds.size.height.0 > scale
            })
            .ok_or("missing grid header divider")?;
        let background = quads
            .iter()
            .find(|q| {
                q.bounds.top() == header.bounds.top()
                    && q.bounds.size.height == header.bounds.size.height
                    && q.bounds.size.width >= body.size.width
                    && q.bounds.left() < body.left()
            })
            .ok_or("missing header background")?;
        let stripe = quads
            .iter()
            .find(|q| {
                q.content_mask.bounds == body
                    && q.bounds.left() == body.left()
                    && q.bounds.size.width >= body.size.width
                    && q.bounds.size.height.0 > 5. * scale
                    && q.bounds.size.height.0 < body.size.height.0 / 2.
            })
            .ok_or("no body row stripes")?;
        // Derive expected column extents from the header, independently of painted data cells.
        let mut ends = quads
            .iter()
            .filter(|q| {
                q.bounds.top() == header.bounds.top()
                    && q.bounds.size == header.bounds.size
                    && q.content_mask.bounds == header.content_mask.bounds
            })
            .map(|q| q.bounds.right().0.round() as u32)
            .collect::<Vec<_>>();
        ends.sort_unstable();
        ends.dedup();
        let mut left = body.left().0.round() as u32;
        let mut columns = Vec::new();
        for end in ends {
            if end as f32 <= body.right().0 + 0.5 {
                columns.push((left, end));
            }
            left = end;
        }
        let grid = Self {
            size: size(
                px(background.bounds.size.width.0 / scale),
                px((divider.bounds.bottom() - header.bounds.top()).0 / scale),
            ),
            columns,
            top: body.top().0.round() as u32,
            bottom: body.bottom().0.round() as u32,
            row_height: stripe.bounds.size.height.0.round() as u32,
            gutter_left: background.bounds.left().0.round() as u32,
        };
        if grid.columns.is_empty() || grid.full_rows() == 0 {
            return Err("no fully visible data cells".into());
        }
        Ok(grid)
    }

    pub fn full_rows(&self) -> u32 {
        (self.bottom - self.top) / self.row_height
    }

    pub fn capture_drag(&self, window: &mut Window, cx: &mut App, path: &Path) -> Result<()> {
        let center = |column: usize, row: u32| {
            let (left, right) = self.columns[column.min(self.columns.len() - 1)];
            let y = self.top + row.min(self.full_rows() - 1) * self.row_height;
            let scale = window.scale_factor();
            point(
                px((left + right) as f32 / (2. * scale)),
                px((y + self.row_height / 2) as f32 / scale),
            )
        };
        window.drag(center(1, 1), center(4, 5), cx);
        window.render_frame(cx);
        Ok(window.render_to_image()?.save(path)?)
    }

    pub fn verify(
        &self,
        window: &Window,
        source: &OrderedView,
        requested_row: u64,
        png: Option<&Path>,
    ) -> Result<u64> {
        let image = window.render_to_image()?;
        if let Some(path) = png {
            image.save(path)?;
        }
        let margin = (2. * window.scale_factor()).ceil() as u32;
        let count = u64::from(self.full_rows()).min(source.row_count()) as u32;
        let max_first_full = source.row_count() - u64::from(count);
        // A bottom-clamped scroll can leave a partial first row above the complete rows.
        let first = requested_row.min(max_first_full);
        let top = if requested_row >= max_first_full && source.row_count() > u64::from(count) {
            self.bottom - count * self.row_height
        } else {
            self.top
        };
        let has_text = |left: u32, right: u32, y: u32| {
            if right > image.width()
                || y + self.row_height > image.height()
                || right <= left + margin * 2
            {
                return false;
            }
            has_ink(
                left + margin..right - margin,
                y + margin..y + self.row_height - margin,
                |x, y| image.get_pixel(x, y).0,
            )
        };
        let mut checked = 0;
        for (index, &(left, right)) in self.columns.iter().enumerate() {
            let cells = source.format_column(ColumnId::from_raw(index as u32), first, count)?;
            if cells.row_count() != count {
                return Err("requested row lies beyond source data".into());
            }
            for offset in 0..count {
                let expected = cells.cell(offset).unwrap_or("null");
                if expected.trim().is_empty() {
                    continue;
                }
                let y = top + offset * self.row_height;
                if !has_text(left, right, y) {
                    return Err(format!(
                        "blank painted cell at source row {} column {} (expected {:?})",
                        first + u64::from(offset),
                        index,
                        expected.chars().take(32).collect::<String>()
                    )
                    .into());
                }
                checked += 1;
            }
        }
        let gutter_right = self.columns[0].0;
        for offset in 0..count {
            if !has_text(
                self.gutter_left,
                gutter_right,
                top + offset * self.row_height,
            ) {
                return Err(format!(
                    "missing painted row number at source row {}",
                    first + u64::from(offset)
                )
                .into());
            }
        }
        Ok(checked)
    }
}

// A uniform stripe, row boundary or out-of-cell divider cannot satisfy the text check.
fn has_ink(
    x: std::ops::Range<u32>,
    mut y: std::ops::Range<u32>,
    pixel: impl Fn(u32, u32) -> [u8; 4],
) -> bool {
    y.any(|py| {
        let bg = pixel(x.start, py);
        (x.start + 1..x.end).any(|px| {
            let p = pixel(px, py);
            (0..3).any(|c| p[c].abs_diff(bg[c]) >= 24)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::has_ink;

    #[test]
    fn blank_striped_cell_stays_blank_beside_ink_and_dividers() {
        let texture = |x, y| {
            let value = if x == 0 || x == 11 || (x == 5 && y == 4) {
                25
            } else if y < 3 {
                210
            } else {
                250
            };
            [value, value, value, 255]
        };
        assert!(has_ink(1..10, 1..8, texture));
        assert!(!has_ink(6..11, 1..8, texture));
    }
}
