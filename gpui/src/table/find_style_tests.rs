use gpui_kit::{TextRun, base, font};

use super::matched_runs;

#[test]
fn visible_parts_of_matches_use_the_theme_pair_and_leave_the_ellipsis_plain() {
    // Source: "αA2…B5". Fitted line: "αA2…". The ellipsis was inserted by
    // the layout engine even though the next source character is also an ellipsis.
    let visible = "αA2".len();
    let line = "αA2…";
    for appearance in [base::ThemeAppearance::Light, base::ThemeAppearance::Dark] {
        let colors = crate::theme::document_colors(appearance, None);
        let find = crate::theme::FindColors::new(appearance);
        let normal = TextRun {
            len: line.len(),
            font: font("System UI"),
            color: colors.foreground,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let spans = [0..2, 3..7, 7..9].into_iter();
        let runs = matched_runs(line.len(), visible, spans, &normal, find);
        let mut start = 0;
        let actual: Vec<_> = runs
            .into_iter()
            .map(|run| {
                let text = &line[start..start + run.len];
                start += run.len;
                (text, run.color, run.background_color)
            })
            .collect();
        assert_eq!(actual, [
            ("α", find.ink, Some(find.marker)),
            ("A", colors.foreground, None),
            ("2", find.ink, Some(find.marker)),
            ("…", colors.foreground, None),
        ]);
        assert!(
            matched_runs(line.len(), visible, [4..7, 7..9].into_iter(), &normal, find).is_empty()
        );
    }
}
