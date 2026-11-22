use super::{FindColors, ThemeAppearance, document_colors, luminance};

#[test]
fn text_meets_contrast_on_document_and_find_surfaces() {
    for appearance in [ThemeAppearance::Light, ThemeAppearance::Dark] {
        let colors = document_colors(appearance, None);
        let find = FindColors::new(appearance);
        for (ink, surface) in [
            (colors.foreground, colors.background),
            (colors.muted_foreground, colors.surface),
            (colors.destructive, colors.surface),
            (colors.primary_foreground, colors.primary),
            (find.ink, find.marker),
        ] {
            let a = luminance(ink);
            let b = luminance(surface);
            assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5);
        }
    }
}
