//! The built-in palettes.
use gpui_kit::base::ThemeAppearance;

use super::Theme;

pub(crate) mod catppuccin;
pub(crate) mod default;
pub(crate) mod high_contrast;

pub(crate) const LIGHT: &[&Theme] = &[&default::LIGHT, &catppuccin::LATTE];
pub(crate) const DARK: &[&Theme] = &[
    &default::DARK,
    &catppuccin::FRAPPE,
    &catppuccin::MACCHIATO,
    &catppuccin::MOCHA,
];

pub(crate) const fn default_for(appearance: ThemeAppearance) -> &'static Theme {
    match appearance {
        ThemeAppearance::Light => &default::LIGHT,
        ThemeAppearance::Dark => &default::DARK,
    }
}

pub(crate) const fn high_contrast_for(appearance: ThemeAppearance) -> &'static Theme {
    match appearance {
        ThemeAppearance::Light => &high_contrast::LIGHT,
        ThemeAppearance::Dark => &high_contrast::DARK,
    }
}
