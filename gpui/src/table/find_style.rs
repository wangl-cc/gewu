use std::ops::Range;

use gpui_kit::*;

use crate::theme::FindColors;

/// Decorate the fitted line while keeping any synthetic truncation suffix plain.
pub(super) fn matched_line(
    line: ShapedLine,
    visible_source_bytes: usize,
    ranges: &[Range<usize>],
    normal: &TextRun,
    window: &Window,
    colors: FindColors,
) -> ShapedLine {
    let runs = matched_runs(
        line.text.len(),
        visible_source_bytes,
        ranges.iter().cloned(),
        normal,
        colors,
    );
    if runs.is_empty() {
        return line;
    }
    window
        .text_system()
        .shape_line(line.text.clone(), line.font_size, &runs, None)
}

fn matched_runs(
    line_bytes: usize,
    visible_source_bytes: usize,
    ranges: impl Iterator<Item = Range<usize>>,
    normal: &TextRun,
    colors: FindColors,
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut end = 0;
    for hit in ranges.take_while(|hit| hit.start < visible_source_bytes) {
        if end < hit.start {
            runs.push(TextRun {
                len: hit.start - end,
                ..normal.clone()
            });
        }
        end = hit.end.min(visible_source_bytes);
        runs.push(TextRun {
            len: end - hit.start,
            color: colors.ink,
            background_color: Some(colors.marker),
            ..normal.clone()
        });
    }
    if !runs.is_empty() && end < line_bytes {
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
