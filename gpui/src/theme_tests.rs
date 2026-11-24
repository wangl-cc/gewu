use gpui_kit::{Hsla, Rgba, white};

use super::{GewuPalette, ThemeAppearance};

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

#[test]
fn default_solid_controls_pair_white_ink_with_opaque_fills() {
    for mode in [ThemeAppearance::Light, ThemeAppearance::Dark] {
        let p = GewuPalette::default_for(mode);
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
            p.danger_hover(),
            p.danger_pressed(),
        ] {
            assert_eq!(fill.a, 1.);
        }
        assert!(p.find_span_current.a > p.find_span.a);
    }
}

#[test]
fn dark_surfaces_and_segment_thumb_rise_above_their_backing() {
    let p = GewuPalette::default_for(ThemeAppearance::Dark);
    assert!(luminance(p.elevated) > luminance(p.surface));
    assert!(luminance(p.surface) > luminance(p.canvas));
    let track = p.surface.blend(p.segment_track);
    assert!(luminance(track.blend(p.segment_thumb)) > luminance(track));
    assert_ne!(p.danger_fill, p.danger_text);
}

fn assert_legible(ink: Hsla, on: Hsla) {
    assert_eq!(on.a, 1.);
    let ink_lum = luminance(on.blend(ink));
    let on_lum = luminance(on);
    let contrast = (ink_lum.max(on_lum) + 0.05) / (ink_lum.min(on_lum) + 0.05);
    assert!(
        contrast >= 4.5,
        "expected 4.5:1, got {contrast:.2}:1 for {ink:?} over {on:?}"
    );
}

#[test]
fn high_contrast_text_and_control_ink_remain_legible_in_painted_states() {
    for appearance in [ThemeAppearance::Light, ThemeAppearance::Dark] {
        let p = GewuPalette::high_contrast_for(appearance);
        for backing in [p.canvas, p.surface, p.elevated] {
            for ink in [p.foreground, p.muted_foreground, p.danger_text] {
                assert_legible(ink, backing);
            }
            let track = backing.blend(p.segment_track);
            for substrate in [
                track,
                track.blend(p.segment_thumb),
                backing.blend(p.button_secondary),
                backing.blend(p.button_secondary_hover),
                backing.blend(p.secondary_pressed()),
            ] {
                assert_legible(p.foreground, substrate);
            }
            for backing in [backing, backing.blend(p.stripe)] {
                for selection in [
                    backing,
                    backing.blend(p.selection_active),
                    backing.blend(p.selection_inactive),
                ] {
                    assert_legible(p.foreground, selection);
                    assert_legible(p.muted_foreground, selection);
                    for find in [p.find_span, p.find_span_current] {
                        assert_legible(p.foreground, selection.blend(find));
                    }
                }
            }
        }
        assert_legible(p.foreground, p.menu_pressed());
        for fill in [p.accent, p.accent_hover, p.accent_pressed()] {
            assert_legible(p.accent_foreground, fill);
            assert_legible(p.switch_thumb, fill);
        }
        for fill in [p.danger_fill, p.danger_hover(), p.danger_pressed()] {
            assert_legible(p.danger_fill_foreground, fill);
        }
    }
}

#[test]
fn changing_contrast_restyles_an_open_window_and_its_toolkit_controls() {
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
            (HighContrast::On, GewuPalette::high_contrast_for(appearance)),
            (HighContrast::Off, GewuPalette::default_for(appearance)),
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
    });
    app.quit();
}
