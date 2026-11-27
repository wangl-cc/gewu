//! Compile-time color palettes and Light/Dark theme projection onto GPUI controls.
use gpui_kit::{
    base::{ColorTokens, ThemeAppearance},
    *,
};

mod color;
pub(crate) mod palettes;

pub(crate) const DISABLED_OPACITY: f32 = 0.45;

pub(crate) fn tabular_digits() -> FontFeatures {
    static DIGITS: std::sync::LazyLock<FontFeatures> =
        std::sync::LazyLock::new(|| FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])));
    DIGITS.clone()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ElevatedShadow {
    Soft,
    Deep,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GewuPalette {
    pub canvas: Hsla,
    pub stripe: Hsla,
    pub surface: Hsla,
    pub elevated: Hsla,
    pub backdrop: Hsla,
    pub border: Hsla,
    pub input_hover: Hsla,
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
    pub accent_pressed: Hsla,
    pub selection_active: Hsla,
    pub selection_inactive: Hsla,
    pub focus_ring: Hsla,
    pub find_span: Hsla,
    pub find_span_current: Hsla,
    pub find_current_border: Hsla,
    pub danger_text: Hsla,
    pub chart_primary: Hsla,
    pub chart_primary_hover: Hsla,
    pub chart_muted: Hsla,
    pub brush_fill: Hsla,
    pub chart_grid: Hsla,
    pub bar_track: Hsla,
    pub shadow: Hsla,
    pub elevated_shadow: ElevatedShadow,
}

impl Global for GewuPalette {}

/// Named color theme classified under [`ThemeAppearance::Light`] or [`ThemeAppearance::Dark`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub appearance: ThemeAppearance,
    pub palette: GewuPalette,
}

impl Theme {
    pub const DARK: &'static [&'static Self] = palettes::DARK;
    pub const LIGHT: &'static [&'static Self] = palettes::LIGHT;

    pub const fn default_for(appearance: ThemeAppearance) -> &'static Self {
        palettes::default_for(appearance)
    }

    pub const fn high_contrast_for(appearance: ThemeAppearance) -> &'static Self {
        palettes::high_contrast_for(appearance)
    }

    pub fn install(&self, cx: &mut App) {
        self.palette.install(self.appearance, cx);
    }
}

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

impl GewuPalette {
    pub fn get(cx: &App) -> Self {
        cx.try_global::<Self>()
            .copied()
            .unwrap_or_else(|| Theme::default_for(base::Theme::global(cx).appearance).palette)
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
        self.input_hover
    }

    pub fn accent_pressed(self) -> Hsla {
        self.accent_pressed
    }

    pub fn focus_shadows(self) -> Vec<BoxShadow> {
        [(3., self.focus_ring), (1., self.canvas)]
            .map(|(spread, color)| BoxShadow::new(px(0.), px(0.), color).spread_radius(px(spread)))
            .into()
    }

    pub fn elevated_shadows(self) -> Vec<BoxShadow> {
        let shadow = |y, blur, alpha| {
            BoxShadow::new(px(0.), px(y), self.shadow.alpha(alpha)).blur_radius(px(blur))
        };
        match self.elevated_shadow {
            ElevatedShadow::Soft => vec![shadow(4., 16., 0.12), shadow(1., 3., 0.06)],
            ElevatedShadow::Deep => vec![shadow(6., 20., 0.36)],
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
        c.ring = self.focus_ring;
        c.selection = self.selection_active;
        c
    }

    pub fn install(self, appearance: ThemeAppearance, cx: &mut App) {
        use gpui_kit::component::{Theme, ThemeMode};
        let mode = match appearance {
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
            c.button_primary = self.accent;
            c.button_primary_hover = self.accent_hover;
            c.button_primary_active = self.accent_pressed();
            c.button_primary_foreground = self.accent_foreground;
            c.button_secondary = self.button_secondary;
            c.button_secondary_hover = self.button_secondary_hover;
            c.button_secondary_foreground = self.foreground;
            c.button_secondary_active = self.secondary_pressed();
            c.radius = px(6.);
            c.radius_lg = px(10.);
        });
        let base = base::Theme::global_mut(cx);
        base.appearance = appearance;
        base.tokens.colors = base_colors;
        base.tokens.radius.lg = px(10.);
        base.tokens.shadow.md = self.elevated_shadows();
        cx.set_global(self);
        cx.refresh_windows();
    }
}

#[cfg(test)]
mod tests;
