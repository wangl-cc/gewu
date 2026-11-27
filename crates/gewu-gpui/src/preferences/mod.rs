//! Application-wide display, export, and update preferences.
use gewu_core::{FileType, ParquetCompression};
use gpui_kit::{base::ThemeAppearance, *};

use crate::theme::Theme;

mod file;
mod window;
pub(crate) use window::open;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum HighContrast {
    #[default]
    System,
    Off,
    On,
}

impl HighContrast {
    pub fn enabled(self, system: bool) -> bool {
        match self {
            Self::System => system,
            Self::Off => false,
            Self::On => true,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Preferences {
    pub light_theme: &'static Theme,
    pub dark_theme: &'static Theme,
    pub high_contrast: HighContrast,
    pub font_size: f32,
    pub row_height: f32,
    pub show_row_numbers: bool,
    pub row_number_base: u64,
    pub copy_includes_headers: bool,
    pub statistics_digits: usize,
    pub remember_export_format: bool,
    pub last_export_format: FileType,
    pub parquet_compression: ParquetCompression,
    /// Whether a newly opened document shows its Inspector. Each document keeps its own
    /// visibility afterwards; toggling it there also changes this default.
    pub show_inspector: bool,
    pub skip_hide_warning: bool,
    pub automatic_update_checks: bool,
    pub automatic_update_install: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            light_theme: Theme::default_for(ThemeAppearance::Light),
            dark_theme: Theme::default_for(ThemeAppearance::Dark),
            high_contrast: HighContrast::System,
            font_size: 13.,
            row_height: 28.,
            show_row_numbers: true,
            row_number_base: 1,
            copy_includes_headers: true,
            statistics_digits: 5,
            remember_export_format: true,
            last_export_format: FileType::Parquet,
            parquet_compression: ParquetCompression::Zstd,
            show_inspector: true,
            skip_hide_warning: false,
            automatic_update_checks: false,
            automatic_update_install: false,
        }
    }
}

impl Global for Preferences {}

struct SaveChanges;
impl Global for SaveChanges {}

pub(crate) fn init(cx: &mut App) {
    let prefs = file::load();
    cx.set_global(prefs);
    cx.set_global(SaveChanges);
}

impl Preferences {
    pub fn get(cx: &App) -> Self {
        cx.try_global::<Self>().copied().unwrap_or_default()
    }

    pub fn update(cx: &mut App, f: impl FnOnce(&mut Self)) {
        let before = Self::get(cx);
        let mut after = before;
        f(&mut after);
        if before == after {
            return;
        }
        if cx.has_global::<SaveChanges>()
            && let Err(error) = file::save(&after)
        {
            after = file::load();
            cx.show_system_notification(SystemNotification {
                tag: "preferences".into(),
                title: "Could not save settings".into(),
                body: error.into(),
                actions: Vec::new(),
            });
        }
        cx.set_global(after);
        if before.high_contrast != after.high_contrast
            || before.light_theme != after.light_theme
            || before.dark_theme != after.dark_theme
        {
            crate::platform::refresh_theme(cx);
        }
        cx.refresh_windows();
    }
}
