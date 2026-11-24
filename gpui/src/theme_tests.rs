use gpui_kit::{black, rgb};

use super::{FindColors, ThemeAppearance, contrast_ratio, document_colors, hover_fill};

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
            assert!(contrast_ratio(ink, surface) >= 4.5);
        }
    }
}

#[test]
fn filled_controls_keep_legible_ink_when_hovered() {
    for appearance in [ThemeAppearance::Light, ThemeAppearance::Dark] {
        for accent in [0x007aff, 0xffcc00, 0xa4d6ff, 0x563dd9] {
            let colors = document_colors(appearance, Some(rgb(accent).into()));
            for (ink, fill) in [
                (colors.primary_foreground, colors.primary),
                (colors.destructive_foreground, colors.destructive),
            ] {
                let hovered = hover_fill(fill, ink);
                assert!(contrast_ratio(ink, fill) >= 4.5);
                assert!(contrast_ratio(ink, hovered) >= 4.5);
                assert_eq!(hovered.a, 1.);
            }
            if accent == 0x007aff {
                assert_eq!(colors.primary_foreground, black());
            }
        }
    }
}
