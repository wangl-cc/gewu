//! The Default document palette and its projection onto GPUI's built-in controls.
use gpui_kit::{
    base::{ColorTokens, ThemeAppearance},
    *,
};

pub(crate) const DISABLED_OPACITY: f32 = 0.45;

pub(crate) fn tabular_digits() -> FontFeatures {
    static DIGITS: std::sync::LazyLock<FontFeatures> =
        std::sync::LazyLock::new(|| FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])));
    DIGITS.clone()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GewuPalette {
    appearance: ThemeAppearance,
    pub canvas: Hsla,
    pub stripe: Hsla,
    pub surface: Hsla,
    pub elevated: Hsla,
    pub backdrop: Hsla,
    pub border: Hsla,
    pub grid: Hsla,
    pub elevated_border: Hsla,
    pub foreground: Hsla,
    pub muted_foreground: Hsla,
    pub segment_track: Hsla,
    pub segment_thumb: Hsla,
    pub switch_track_off: Hsla,
    pub switch_thumb: Hsla,
    pub button_secondary: Hsla,
    pub button_secondary_hover: Hsla,
    pub accent: Hsla,
    pub accent_foreground: Hsla,
    pub accent_hover: Hsla,
    pub selection_active: Hsla,
    pub selection_inactive: Hsla,
    pub focus_ring: Hsla,
    pub find_span: Hsla,
    pub find_span_current: Hsla,
    pub find_current_border: Hsla,
    pub danger_text: Hsla,
    pub danger_fill: Hsla,
    pub danger_fill_foreground: Hsla,
    pub chart_primary: Hsla,
    pub chart_primary_hover: Hsla,
    pub chart_muted: Hsla,
    pub brush_fill: Hsla,
    pub chart_grid: Hsla,
    pub bar_track: Hsla,
}

impl Global for GewuPalette {}

/// State geometry also responds to Differentiate Without Color when the palette stays Default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VisualCues {
    pub cell_outline: Pixels,
    pub find_outline: Pixels,
    pub brush_outline: Pixels,
    pub unselected_bin_outline: Pixels,
}

impl Global for VisualCues {}

impl VisualCues {
    pub fn for_enhancement(enhanced: bool) -> Self {
        let (cell, find, brush, bin) = if enhanced {
            (2.5, 2.5, 2., 1.)
        } else {
            (1.5, 1.5, 1., 0.)
        };
        Self {
            cell_outline: px(cell),
            find_outline: px(find),
            brush_outline: px(brush),
            unselected_bin_outline: px(bin),
        }
    }

    pub fn get(cx: &App) -> Self {
        cx.try_global::<Self>()
            .copied()
            .unwrap_or_else(|| Self::for_enhancement(false))
    }
}

fn color(hex: u32) -> Hsla {
    rgb(hex).into()
}

impl GewuPalette {
    pub fn default_for(appearance: ThemeAppearance) -> Self {
        match appearance {
            ThemeAppearance::Light => Self {
                appearance,
                canvas: white(),
                stripe: black().alpha(0.035),
                surface: color(0xf6f6f6),
                elevated: white(),
                backdrop: black().alpha(0.24),
                border: black().alpha(0.11),
                grid: black().alpha(0.07),
                elevated_border: black().alpha(0.12),
                foreground: color(0x262626),
                muted_foreground: color(0x68686d),
                segment_track: black().alpha(0.06),
                segment_thumb: white(),
                switch_track_off: black().alpha(0.16),
                switch_thumb: white(),
                button_secondary: black().alpha(0.06),
                button_secondary_hover: black().alpha(0.1),
                accent: color(0x007aff),
                accent_foreground: white(),
                accent_hover: color(0x006ce6),
                selection_active: color(0x007aff).alpha(0.16),
                selection_inactive: color(0xdcdcdc).alpha(0.55),
                focus_ring: color(0x007aff),
                find_span: color(0xe5b84b).alpha(0.28),
                find_span_current: color(0xe5b84b).alpha(0.48),
                find_current_border: color(0xc48f00).alpha(0.78),
                danger_text: color(0xc92a2a),
                danger_fill: color(0xd9383a),
                danger_fill_foreground: white(),
                chart_primary: color(0x007aff).alpha(0.75),
                chart_primary_hover: color(0x007aff),
                chart_muted: black().alpha(0.2),
                brush_fill: color(0x007aff).alpha(0.14),
                chart_grid: black().alpha(0.07),
                bar_track: black().alpha(0.07),
            },
            ThemeAppearance::Dark => Self {
                appearance,
                canvas: color(0x1e1e1e),
                stripe: white().alpha(0.04),
                surface: color(0x252527),
                elevated: color(0x2c2c2e),
                backdrop: black().alpha(0.46),
                border: white().alpha(0.12),
                grid: white().alpha(0.07),
                elevated_border: white().alpha(0.13),
                foreground: color(0xe5e5e7),
                muted_foreground: color(0x98989d),
                segment_track: black().alpha(0.28),
                segment_thumb: white().alpha(0.16),
                switch_track_off: white().alpha(0.2),
                switch_thumb: white(),
                button_secondary: white().alpha(0.11),
                button_secondary_hover: white().alpha(0.16),
                accent: color(0x0a84ff),
                accent_foreground: white(),
                accent_hover: color(0x2693ff),
                selection_active: color(0x0a84ff).alpha(0.22),
                selection_inactive: white().alpha(0.1),
                focus_ring: color(0x0a84ff),
                find_span: color(0xe5b84b).alpha(0.26),
                find_span_current: color(0xe5b84b).alpha(0.44),
                find_current_border: color(0xf0c24b).alpha(0.78),
                danger_text: color(0xff6961),
                danger_fill: color(0xd9383a),
                danger_fill_foreground: white(),
                chart_primary: color(0x0a84ff).alpha(0.82),
                chart_primary_hover: color(0x0a84ff),
                chart_muted: white().alpha(0.22),
                brush_fill: color(0x0a84ff).alpha(0.18),
                chart_grid: white().alpha(0.08),
                bar_track: white().alpha(0.1),
            },
        }
    }

    /// A separate set of role values. White control ink and the switch thumb are
    /// shared with Default. Components use the same roles in either mode.
    pub fn high_contrast_for(appearance: ThemeAppearance) -> Self {
        let pair = |light, dark| match appearance {
            ThemeAppearance::Light => light,
            ThemeAppearance::Dark => dark,
        };
        Self {
            canvas: pair(white(), color(0x101012)),
            stripe: pair(black().alpha(0.06), white().alpha(0.055)),
            surface: pair(color(0xf4f4f6), color(0x19191c)),
            elevated: pair(white(), color(0x222226)),
            backdrop: pair(black().alpha(0.32), black().alpha(0.60)),
            border: pair(black().alpha(0.56), white().alpha(0.55)),
            grid: pair(black().alpha(0.33), white().alpha(0.34)),
            elevated_border: pair(black().alpha(0.68), white().alpha(0.68)),
            foreground: pair(color(0x111113), white()),
            muted_foreground: pair(color(0x38383c), color(0xdfdfe4)),
            segment_track: pair(black().alpha(0.14), black().alpha(0.60)),
            segment_thumb: pair(white(), white().alpha(0.27)),
            switch_track_off: pair(black().alpha(0.58), white().alpha(0.38)),
            button_secondary: pair(black().alpha(0.11), white().alpha(0.16)),
            button_secondary_hover: pair(black().alpha(0.18), white().alpha(0.22)),
            accent: pair(color(0x005bbc), color(0x0070d6)),
            accent_hover: pair(color(0x004e9f), color(0x0066c5)),
            selection_active: pair(color(0x005bbc).alpha(0.23), color(0x3185ff).alpha(0.28)),
            selection_inactive: pair(black().alpha(0.14), white().alpha(0.14)),
            focus_ring: pair(color(0x00468c), color(0x8ac6ff)),
            find_span: pair(color(0xda9d19).alpha(0.33), color(0xe6ab3d).alpha(0.27)),
            find_span_current: pair(color(0xda9d19).alpha(0.53), color(0xe6ab3d).alpha(0.41)),
            find_current_border: pair(color(0x735000), color(0xffdd83)),
            danger_text: pair(color(0x9d161c), color(0xffaaa5)),
            danger_fill: color(0xcd3133),
            chart_primary: pair(color(0x005bbb), color(0x80bcff)),
            chart_primary_hover: pair(color(0x003d81), color(0xbaddff)),
            chart_muted: pair(black().alpha(0.34), white().alpha(0.37)),
            brush_fill: pair(color(0x005bbc).alpha(0.13), color(0x3893ff).alpha(0.16)),
            chart_grid: pair(black().alpha(0.27), white().alpha(0.28)),
            bar_track: pair(black().alpha(0.16), white().alpha(0.18)),
            ..Self::default_for(appearance)
        }
    }

    pub fn get(cx: &App) -> Self {
        cx.try_global::<Self>()
            .copied()
            .unwrap_or_else(|| Self::default_for(base::Theme::global(cx).appearance))
    }

    pub fn secondary_pressed(self) -> Hsla {
        self.button_secondary_hover
            .alpha((self.button_secondary_hover.a + 0.04).min(1.))
    }

    // The toolkit already paints hover on the parent menu row. An opaque fill keeps
    // the pressed state identical across keyboard and pointer highlights.
    pub fn menu_pressed(self) -> Hsla {
        self.elevated.blend(self.secondary_pressed())
    }

    pub fn input_hover(self) -> Hsla {
        self.border.alpha(
            (self.border.a
                + if self.appearance == ThemeAppearance::Light {
                    0.09
                } else {
                    0.10
                })
            .min(1.),
        )
    }

    pub fn accent_pressed(self) -> Hsla {
        self.accent.blend(black().alpha(0.12))
    }

    pub fn danger_hover(self) -> Hsla {
        color(0xc32e30)
    }

    pub fn danger_pressed(self) -> Hsla {
        color(0xad2628)
    }

    pub fn focus_shadows(self) -> Vec<BoxShadow> {
        [(3., self.focus_ring), (1., self.canvas)]
            .map(|(spread, color)| BoxShadow::new(px(0.), px(0.), color).spread_radius(px(spread)))
            .into()
    }

    pub fn elevated_shadows(self) -> Vec<BoxShadow> {
        let shadow = |y, blur, alpha| {
            BoxShadow::new(px(0.), px(y), black().alpha(alpha)).blur_radius(px(blur))
        };
        match self.appearance {
            ThemeAppearance::Light => vec![shadow(4., 16., 0.12), shadow(1., 3., 0.06)],
            ThemeAppearance::Dark => vec![shadow(6., 20., 0.36)],
        }
    }

    fn base_colors(self) -> ColorTokens {
        let mut c = ColorTokens::default();
        c.background = self.canvas;
        c.surface = self.surface;
        c.surface_foreground = self.foreground;
        c.foreground = self.foreground;
        c.muted = self.segment_track;
        c.muted_foreground = self.muted_foreground;
        c.accent = self.button_secondary_hover;
        c.accent_foreground = self.foreground;
        c.primary = self.accent;
        c.primary_foreground = self.accent_foreground;
        c.secondary = self.button_secondary;
        c.secondary_foreground = self.foreground;
        c.border = self.border;
        c.input = self.border;
        c.destructive = self.danger_fill;
        c.destructive_foreground = self.danger_fill_foreground;
        c.ring = self.focus_ring;
        c.selection = self.selection_active;
        c
    }

    pub fn install(self, cx: &mut App) {
        use gpui_kit::component::{Theme, ThemeMode};
        let mode = match self.appearance {
            ThemeAppearance::Light => ThemeMode::Light,
            ThemeAppearance::Dark => ThemeMode::Dark,
        };
        if Theme::global(cx).mode != mode {
            Theme::change(mode, None, cx);
        }
        let base_colors = self.base_colors();
        if cx.try_global::<Self>() == Some(&self)
            && base::Theme::global(cx).tokens.colors == base_colors
        {
            return;
        }
        Theme::update(cx, |c| {
            c.background = self.canvas;
            c.foreground = self.foreground;
            c.muted = self.segment_track;
            c.muted_foreground = self.muted_foreground;
            c.popover = self.elevated;
            c.popover_foreground = self.foreground;
            c.colors.list = self.elevated;
            c.list_hover = self.button_secondary_hover;
            c.list_active = self.button_secondary_hover;
            c.accent = self.button_secondary_hover;
            c.accent_foreground = self.foreground;
            c.primary = self.accent;
            c.primary_foreground = self.accent_foreground;
            c.primary_hover = self.accent_hover;
            c.primary_active = self.accent_pressed();
            c.secondary = self.button_secondary;
            c.secondary_foreground = self.foreground;
            c.secondary_hover = self.button_secondary_hover;
            c.border = self.border;
            c.input = self.border;
            c.ring = self.focus_ring;
            c.selection = self.selection_active;
            c.danger = self.danger_fill;
            c.danger_foreground = self.danger_fill_foreground;
            c.button_primary = self.accent;
            c.button_primary_hover = self.accent_hover;
            c.button_primary_active = self.accent_pressed();
            c.button_primary_foreground = self.accent_foreground;
            c.button_secondary = self.button_secondary;
            c.button_secondary_hover = self.button_secondary_hover;
            c.button_secondary_foreground = self.foreground;
            c.button_secondary_active = self.secondary_pressed();
            c.button_danger = self.danger_fill;
            c.button_danger_hover = self.danger_hover();
            c.button_danger_active = self.danger_pressed();
            c.button_danger_foreground = self.danger_fill_foreground;
            c.radius = px(6.);
            c.radius_lg = px(10.);
        });
        let base = base::Theme::global_mut(cx);
        base.appearance = self.appearance;
        base.tokens.colors = base_colors;
        base.tokens.radius.lg = px(10.);
        base.tokens.shadow.md = self.elevated_shadows();
        cx.set_global(self);
        cx.refresh_windows();
    }
}

#[cfg(test)]
#[path = "theme_tests.rs"]
mod tests;
