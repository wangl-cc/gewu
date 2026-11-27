use gpui_kit::{TextRun, base::ThemeAppearance, font};

use super::matched_runs;
use crate::theme::Theme;

#[test]
fn fitted_hits_preserve_ink_and_only_mark_the_ellipsis_when_every_hit_is_hidden() {
    // A real ellipsis follows the visible prefix in the source. The fitted
    // ellipsis represents hidden bytes, regardless of their spelling.
    let line = "αA2…";
    let visible = "αA2".len();
    for appearance in [ThemeAppearance::Light, ThemeAppearance::Dark] {
        let colors = Theme::default_for(appearance).palette;
        let normal = TextRun {
            len: line.len(),
            font: font("System UI"),
            color: colors.foreground,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        for marker in [colors.find_span, colors.find_span_current] {
            let segments = |visible, ranges: &[std::ops::Range<usize>]| {
                let runs = matched_runs(line.len(), visible, ranges, &normal, marker);
                let mut start = 0;
                runs.into_iter()
                    .map(|run| {
                        assert_eq!(run.color, normal.color);
                        let text = &line[start..start + run.len];
                        start += run.len;
                        (text, run.background_color)
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(segments(visible, &[0..2, 3..7, 7..9]), [
                ("α", Some(marker)),
                ("A", None),
                ("2", Some(marker)),
                ("…", None)
            ]);
            assert_eq!(segments(visible, &[4..7, 7..9]), [
                ("αA2", None),
                ("…", Some(marker))
            ]);
            assert_eq!(segments(visible, &[]), [(line, None)]);
        }
    }
}
