//! Gewu's semantic colors extend the neutral GPUI theme with a document accent and Find.
use gpui_kit::{
    base::{ColorTokens, ThemeAppearance},
    *,
};

pub(crate) fn tabular_digits() -> FontFeatures {
    static DIGITS: std::sync::LazyLock<FontFeatures> =
        std::sync::LazyLock::new(|| FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])));
    DIGITS.clone()
}

pub(crate) fn document_colors(appearance: ThemeAppearance, accent: Option<Hsla>) -> ColorTokens {
    let (mut colors, fallback) = match appearance {
        ThemeAppearance::Light => (ColorTokens::light(), 0x0068da),
        ThemeAppearance::Dark => (ColorTokens::dark(), 0x559dff),
    };
    match appearance {
        ThemeAppearance::Light => {
            colors.surface = rgb(0xf7f7f8).into();
            colors.muted_foreground = rgb(0x585b60).into();
            colors.destructive = rgb(0xb42324).into();
        }
        ThemeAppearance::Dark => {
            colors.background = rgb(0x1a1a1a).into();
            colors.surface = rgb(0x212123).into();
            colors.secondary = rgb(0x333336).into();
            colors.muted = rgb(0x353537).into();
            colors.accent = colors.muted;
            colors.border = rgb(0x343436).into();
        }
    }
    let accent = accent.unwrap_or_else(|| rgb(fallback).into());
    colors.primary = accent;
    colors.primary_foreground = contrasting_ink(accent);
    colors.destructive_foreground = contrasting_ink(colors.destructive);
    colors.ring = accent;
    colors.selection = accent.alpha(0.18);
    colors
}

fn luminance(color: Hsla) -> f32 {
    let rgb: Rgba = color.into();
    let linear = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(rgb.r) + 0.7152 * linear(rgb.g) + 0.0722 * linear(rgb.b)
}

fn contrasting_ink(color: Hsla) -> Hsla {
    if contrast_ratio(black(), color) > contrast_ratio(white(), color) {
        black()
    } else {
        white()
    }
}

fn contrast_ratio(a: Hsla, b: Hsla) -> f32 {
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Change a filled control's background in the direction that increases ink contrast.
/// Keeping the fill opaque makes the result independent of the surface under the button.
pub(crate) fn hover_fill(mut fill: Hsla, ink: Hsla) -> Hsla {
    let step = if luminance(ink) < luminance(fill) {
        0.045
    } else {
        -0.045
    };
    fill.l = (fill.l + step).clamp(0., 1.);
    fill
}

pub(crate) fn modal_backdrop() -> Hsla {
    black().alpha(0.24)
}

#[derive(Clone, Copy)]
pub(crate) struct FindColors {
    pub marker: Hsla,
    pub ink: Hsla,
    pub edge: Hsla,
}

impl FindColors {
    pub fn new(appearance: ThemeAppearance) -> Self {
        let (marker, ink, edge) = match appearance {
            ThemeAppearance::Light => (0xffd561, 0x352600, 0x805400),
            ThemeAppearance::Dark => (0x856009, 0xfff7d9, 0xffd061),
        };
        Self {
            marker: rgb(marker).into(),
            ink: rgb(ink).into(),
            edge: rgb(edge).into(),
        }
    }
}

#[cfg(test)]
#[path = "theme_tests.rs"]
mod tests;
