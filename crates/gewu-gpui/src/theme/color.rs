//! Compile-time color constructors for built-in palettes.

use gpui_kit::{Hsla, hsla};

pub(super) const fn white() -> Hsla {
    hsla(0., 0., 1., 1.)
}

pub(super) const fn black() -> Hsla {
    hsla(0., 0., 0., 1.)
}

pub(super) const fn white_alpha(a: f32) -> Hsla {
    hsla(0., 0., 1., a)
}

pub(super) const fn black_alpha(a: f32) -> Hsla {
    hsla(0., 0., 0., a)
}

const fn fmax(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

const fn fmin(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

const fn rgb_floats(r: f32, g: f32, b: f32, a: f32) -> Hsla {
    let max = fmax(r, fmax(g, b));
    let min = fmin(r, fmin(g, b));
    let delta = max - min;

    let l = (max + min) / 2.0;
    let s = if l == 0.0 || l == 1.0 {
        0.0
    } else if l < 0.5 {
        delta / (2.0 * l)
    } else {
        delta / (2.0 - 2.0 * l)
    };

    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        let mut hue = (g - b) / delta;
        if hue < 0.0 {
            hue += 6.0;
        }
        hue / 6.0
    } else if max == g {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };

    hsla(h, s, l, a)
}

/// Converts a `0xRRGGBB` hex color and alpha into [`Hsla`] at compile time,
/// matching GPUI's `From<Rgba> for Hsla` conversion.
pub(super) const fn rgba(hex: u32, a: f32) -> Hsla {
    let r = ((hex >> 16) & 0xff) as f32 / 255.0;
    let g = ((hex >> 8) & 0xff) as f32 / 255.0;
    let b = (hex & 0xff) as f32 / 255.0;
    rgb_floats(r, g, b, a)
}

pub(super) const fn rgb(hex: u32) -> Hsla {
    rgba(hex, 1.0)
}

/// Linearly mixes two `0xRRGGBB` sRGB colors (`t = 0.0` returns `a`, `t = 1.0` returns `b`).
pub(super) const fn mix_rgb(a: u32, b: u32, t: f32) -> Hsla {
    let ar = ((a >> 16) & 0xff) as f32 / 255.0;
    let ag = ((a >> 8) & 0xff) as f32 / 255.0;
    let ab = (a & 0xff) as f32 / 255.0;

    let br = ((b >> 16) & 0xff) as f32 / 255.0;
    let bg = ((b >> 8) & 0xff) as f32 / 255.0;
    let bb = (b & 0xff) as f32 / 255.0;

    rgb_floats(
        ar + (br - ar) * t,
        ag + (bg - ag) * t,
        ab + (bb - ab) * t,
        1.0,
    )
}
