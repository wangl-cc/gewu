use gpui_kit::{Hsla, Rgba, white};

use super::{GewuPalette, Theme, ThemeAppearance, color, palettes};

fn luminance(color: Hsla) -> f32 {
    let channel = |x: f32| {
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    let Rgba { r, g, b, .. } = color.to_rgb();
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

fn contrast_ratio(ink: Hsla, on: Hsla) -> f32 {
    assert_eq!(on.a, 1.);
    let ink_lum = luminance(on.blend(ink));
    let on_lum = luminance(on);
    (ink_lum.max(on_lum) + 0.05) / (ink_lum.min(on_lum) + 0.05)
}

fn assert_contrast(ink: Hsla, on: Hsla, minimum: f32) {
    let ratio = contrast_ratio(ink, on);
    assert!(
        ratio >= minimum,
        "expected {minimum:.1}:1, got {ratio:.2}:1 for {ink:?} over {on:?}"
    );
}

#[test]
fn const_rgb_and_rgba_match_gpui() {
    for (hex, alpha) in [
        (0x000000, 0.25),
        (0xffffff, 1.0),
        (0x007aff, 0.75),
        (0xe5b84b, 0.48),
        (0x1e66f5, 1.0),
        (0x8caaee, 0.22),
        (0x8aadf4, 0.18),
        (0x89b4fa, 1.0),
        (0xf38ba8, 1.0),
    ] {
        let actual_rgba = color::rgba(hex, alpha);
        let expected_rgba: Hsla = gpui_kit::rgb(hex).alpha(alpha).into();
        let actual_rgb = color::rgb(hex);
        let expected_rgb: Hsla = gpui_kit::rgb(hex).into();
        for (actual, expected, kind) in [
            (actual_rgba, expected_rgba, "rgba"),
            (actual_rgb, expected_rgb, "rgb"),
        ] {
            assert!(
                (actual.h - expected.h).abs() < 1e-5,
                "{kind} hue for {hex:#x}"
            );
            assert!(
                (actual.s - expected.s).abs() < 1e-5,
                "{kind} saturation for {hex:#x}"
            );
            assert!(
                (actual.l - expected.l).abs() < 1e-5,
                "{kind} lightness for {hex:#x}"
            );
            assert!(
                (actual.a - expected.a).abs() < 1e-5,
                "{kind} alpha for {hex:#x}"
            );
        }
    }

    for (actual, expected) in [
        (
            color::mix_rgb(0x1e66f5, 0x4c4f69, 0.0),
            color::rgb(0x1e66f5),
        ),
        (
            color::mix_rgb(0x1e66f5, 0x4c4f69, 1.0),
            color::rgb(0x4c4f69),
        ),
    ] {
        assert!((actual.h - expected.h).abs() < 1e-5);
        assert!((actual.s - expected.s).abs() < 1e-5);
        assert!((actual.l - expected.l).abs() < 1e-5);
        assert!((actual.a - expected.a).abs() < 1e-5);
    }
    let mid = color::mix_rgb(0x000000, 0xffffff, 0.5).to_rgb();
    assert!((mid.r - 0.5).abs() < 1e-3);
    assert!((mid.g - 0.5).abs() < 1e-3);
    assert!((mid.b - 0.5).abs() < 1e-3);
}

#[test]
fn themes_classify_light_and_dark_with_unique_ids() {
    let mut ids = std::collections::HashSet::new();
    for theme in Theme::LIGHT {
        assert_eq!(theme.appearance, ThemeAppearance::Light);
        assert!(ids.insert(theme.id), "duplicate id {}", theme.id);
        assert!(!theme.name.is_empty());
    }
    for theme in Theme::DARK {
        assert_eq!(theme.appearance, ThemeAppearance::Dark);
        assert!(ids.insert(theme.id), "duplicate id {}", theme.id);
        assert!(!theme.name.is_empty());
    }
    for (theme, appearance) in [
        (&palettes::high_contrast::LIGHT, ThemeAppearance::Light),
        (&palettes::high_contrast::DARK, ThemeAppearance::Dark),
    ] {
        assert_eq!(theme.appearance, appearance);
        assert!(ids.insert(theme.id), "duplicate id {}", theme.id);
        assert!(!theme.name.is_empty());
    }
    assert_eq!(
        Theme::default_for(ThemeAppearance::Light),
        &palettes::default::LIGHT
    );
    assert_eq!(
        Theme::default_for(ThemeAppearance::Dark),
        &palettes::default::DARK
    );
    assert_eq!(
        Theme::high_contrast_for(ThemeAppearance::Light),
        &palettes::high_contrast::LIGHT
    );
    assert_eq!(
        Theme::high_contrast_for(ThemeAppearance::Dark),
        &palettes::high_contrast::DARK
    );
}

#[test]
fn default_solid_controls_pair_white_ink_with_opaque_fills() {
    for mode in [ThemeAppearance::Light, ThemeAppearance::Dark] {
        let p = &Theme::default_for(mode).palette;
        for ink in [
            p.accent_foreground,
            p.danger_fill_foreground,
            p.switch_thumb,
        ] {
            assert_eq!(ink, white());
        }
        for fill in [
            p.accent,
            p.accent_hover,
            p.accent_pressed(),
            p.danger_fill,
            p.danger_hover,
            p.danger_pressed,
        ] {
            assert_eq!(fill.a, 1.);
        }
        assert!(p.find_span_current.a > p.find_span.a);
    }
}

#[test]
fn dark_surfaces_and_segment_thumb_rise_above_their_backing() {
    let p = &Theme::default_for(ThemeAppearance::Dark).palette;
    assert!(luminance(p.elevated) > luminance(p.surface));
    assert!(luminance(p.surface) > luminance(p.canvas));
    let track = p.surface.blend(p.segment_track);
    assert!(luminance(track.blend(p.segment_thumb)) > luminance(track));
    assert_ne!(p.danger_fill, p.danger_text);
}

#[test]
fn catppuccin_flavors_keep_text_and_control_ink_legible() {
    for theme in [
        &palettes::catppuccin::LATTE,
        &palettes::catppuccin::FRAPPE,
        &palettes::catppuccin::MACCHIATO,
        &palettes::catppuccin::MOCHA,
    ] {
        let p = &theme.palette;
        for backing in [p.canvas, p.surface, p.elevated] {
            assert_contrast(p.foreground, backing, 4.5);
            for button in [
                p.button_secondary,
                p.button_secondary_hover,
                p.secondary_pressed(),
            ] {
                assert_contrast(p.foreground, backing.blend(button), 4.5);
            }
        }
        for backing in [
            p.segment_thumb,
            p.surface.blend(p.segment_track).blend(p.segment_thumb),
            p.canvas.blend(p.selection_active),
            p.canvas.blend(p.selection_inactive),
            p.canvas.blend(p.find_span),
            p.canvas.blend(p.find_span_current),
        ] {
            assert_contrast(p.foreground, backing, 4.5);
        }
        for backing in [p.canvas, p.surface] {
            assert_contrast(p.muted_foreground, backing, 4.0);
        }
        for fill in [p.accent, p.accent_hover, p.accent_pressed()] {
            assert_contrast(p.accent_foreground, fill, 4.5);
            assert_contrast(p.switch_thumb, fill, 4.5);
        }
        for fill in [p.danger_fill, p.danger_hover, p.danger_pressed] {
            assert_contrast(p.danger_fill_foreground, fill, 4.5);
        }
    }
    assert_ne!(palettes::catppuccin::LATTE.palette.elevated, white());
}

#[test]
fn high_contrast_text_and_control_ink_remain_legible_in_painted_states() {
    for appearance in [ThemeAppearance::Light, ThemeAppearance::Dark] {
        let p = &Theme::high_contrast_for(appearance).palette;
        for backing in [p.canvas, p.surface, p.elevated] {
            for ink in [p.foreground, p.muted_foreground, p.danger_text] {
                assert_contrast(ink, backing, 4.5);
            }
            let track = backing.blend(p.segment_track);
            for substrate in [
                track,
                track.blend(p.segment_thumb),
                backing.blend(p.button_secondary),
                backing.blend(p.button_secondary_hover),
                backing.blend(p.secondary_pressed()),
            ] {
                assert_contrast(p.foreground, substrate, 4.5);
            }
            for backing in [backing, backing.blend(p.stripe)] {
                for selection in [
                    backing,
                    backing.blend(p.selection_active),
                    backing.blend(p.selection_inactive),
                ] {
                    assert_contrast(p.foreground, selection, 4.5);
                    assert_contrast(p.muted_foreground, selection, 4.5);
                    for find in [p.find_span, p.find_span_current] {
                        assert_contrast(p.foreground, selection.blend(find), 4.5);
                    }
                }
            }
        }
        assert_contrast(p.foreground, p.menu_pressed(), 4.5);
        assert_contrast(p.muted_foreground, p.surface.blend(p.button_secondary), 4.5);
        for fill in [p.accent, p.accent_hover, p.accent_pressed()] {
            assert_contrast(p.accent_foreground, fill, 4.5);
            assert_contrast(p.switch_thumb, fill, 4.5);
        }
        for fill in [p.danger_fill, p.danger_hover, p.danger_pressed] {
            assert_contrast(p.danger_fill_foreground, fill, 4.5);
        }
    }
}

#[test]
fn changing_contrast_and_theme_preference_restyles_an_open_window() {
    use gpui_kit::TestAppContext;

    use crate::{
        platform,
        preferences::{HighContrast, Preferences},
        theme::VisualCues,
    };
    let mut app = TestAppContext::single();
    app.update(gpui_kit::init);
    let window = app.add_empty_window();
    window.update(|w, cx| {
        platform::follow_appearance(w, cx);
        let appearance = gpui_kit::base::Theme::global(cx).appearance;
        for (choice, expected) in [
            (
                HighContrast::On,
                Theme::high_contrast_for(appearance).palette,
            ),
            (HighContrast::Off, Theme::default_for(appearance).palette),
        ] {
            Preferences::update(cx, |prefs| prefs.high_contrast = choice);
            assert_eq!(GewuPalette::get(cx), expected);
            assert_eq!(
                gpui_kit::base::Theme::global(cx).tokens.colors.primary,
                expected.accent
            );
            assert_eq!(
                gpui_kit::component::Theme::global(cx).popover,
                expected.elevated
            );
            if choice == HighContrast::On {
                assert_eq!(VisualCues::get(cx), VisualCues::for_enhancement(true));
            }
        }

        let target = match appearance {
            ThemeAppearance::Light => &palettes::catppuccin::LATTE,
            ThemeAppearance::Dark => &palettes::catppuccin::MOCHA,
        };
        Preferences::update(cx, |prefs| match appearance {
            ThemeAppearance::Light => prefs.light_theme = target,
            ThemeAppearance::Dark => prefs.dark_theme = target,
        });
        assert_eq!(GewuPalette::get(cx), target.palette);
        assert_eq!(
            gpui_kit::base::Theme::global(cx).tokens.colors.background,
            target.palette.canvas
        );
    });
    app.quit();
}
