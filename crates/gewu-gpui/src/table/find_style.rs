//! Styling the Find matches within a cell's fitted line of text.
use std::ops::Range;

use gpui_kit::*;

/// A fitted preview can hide every match. In that case its ellipsis represents the hit.
pub(super) fn matched_line(
    line: ShapedLine,
    visible_source_bytes: usize,
    ranges: &[Range<usize>],
    normal: &TextRun,
    window: &Window,
    fill: Hsla,
) -> ShapedLine {
    let runs = matched_runs(line.text.len(), visible_source_bytes, ranges, normal, fill);
    window
        .text_system()
        .shape_line(line.text.clone(), line.font_size, &runs, None)
}

fn matched_runs(
    line_bytes: usize,
    visible_source_bytes: usize,
    ranges: &[Range<usize>],
    normal: &TextRun,
    fill: Hsla,
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut end = 0;
    let visible = ranges
        .iter()
        .take_while(|hit| hit.start < visible_source_bytes)
        .map(|hit| hit.start..hit.end.min(visible_source_bytes));
    let ellipsis_hit = (visible_source_bytes < line_bytes
        && ranges
            .first()
            .is_some_and(|hit| hit.start >= visible_source_bytes))
    .then_some(visible_source_bytes..line_bytes);
    for hit in visible.chain(ellipsis_hit) {
        if end < hit.start {
            runs.push(TextRun {
                len: hit.start - end,
                ..normal.clone()
            });
        }
        end = hit.end;
        runs.push(TextRun {
            len: end - hit.start,
            background_color: Some(fill),
            ..normal.clone()
        });
    }
    if end < line_bytes {
        runs.push(TextRun {
            len: line_bytes - end,
            ..normal.clone()
        });
    }
    runs
}

#[cfg(test)]
#[path = "find_style_tests.rs"]
mod tests;
