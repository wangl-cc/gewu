use std::path::Path;

use serde::{Deserialize, Serialize};

use super::*;
use crate::platform::storage::{read_json, settings_path, write_json};

const FORMATS: &[(&str, FileType)] = &[
    ("parquet", FileType::Parquet),
    ("csv", FileType::Csv),
    ("tsv", FileType::Tsv),
    ("xlsx", FileType::Xlsx),
];
const CONTRAST: &[(&str, HighContrast)] = &[
    ("system", HighContrast::System),
    ("off", HighContrast::Off),
    ("on", HighContrast::On),
];
const CODECS: &[(&str, ParquetCompression)] = &[
    ("zstd", ParquetCompression::Zstd),
    ("snappy", ParquetCompression::Snappy),
    ("gzip", ParquetCompression::Gzip),
    ("brotli", ParquetCompression::Brotli),
    ("lz4", ParquetCompression::Lz4),
    ("uncompressed", ParquetCompression::Uncompressed),
];

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct SettingsFile {
    high_contrast: Option<String>,
    font_size: Option<f32>,
    row_height: Option<f32>,
    show_row_numbers: Option<bool>,
    row_number_base: Option<u64>,
    copy_includes_headers: Option<bool>,
    statistics_digits: Option<usize>,
    remember_export_format: Option<bool>,
    last_export_format: Option<String>,
    parquet_compression: Option<String>,
    show_inspector: Option<bool>,
    skip_hide_warning: Option<bool>,
    automatic_update_checks: Option<bool>,
    automatic_update_install: Option<bool>,
}

fn decode_choice<T: Copy>(value: Option<&str>, items: &[(&str, T)], fallback: T) -> T {
    value
        .and_then(|name| items.iter().find(|(label, _)| *label == name))
        .map_or(fallback, |(_, item)| *item)
}

fn encode_choice<T: PartialEq>(value: T, items: &[(&str, T)]) -> Option<String> {
    items
        .iter()
        .find(|(_, item)| *item == value)
        .map(|(label, _)| (*label).to_owned())
}

pub(super) fn load() -> Preferences {
    settings_path()
        .as_deref()
        .map(load_from)
        .unwrap_or_default()
}

pub(super) fn load_from(path: &Path) -> Preferences {
    let file: SettingsFile = read_json(path).unwrap_or_default();
    let d = Preferences::default();
    Preferences {
        high_contrast: decode_choice(file.high_contrast.as_deref(), CONTRAST, d.high_contrast),
        font_size: file
            .font_size
            .filter(|v| v.is_finite())
            .map_or(d.font_size, |v| v.clamp(10., 20.)),
        row_height: file
            .row_height
            .filter(|v| v.is_finite())
            .map_or(d.row_height, |v| v.clamp(24., 36.)),
        show_row_numbers: file.show_row_numbers.unwrap_or(d.show_row_numbers),
        row_number_base: file.row_number_base.map_or(d.row_number_base, |v| v.min(1)),
        copy_includes_headers: file
            .copy_includes_headers
            .unwrap_or(d.copy_includes_headers),
        statistics_digits: file
            .statistics_digits
            .map_or(d.statistics_digits, |v| v.clamp(3, 15)),
        remember_export_format: file
            .remember_export_format
            .unwrap_or(d.remember_export_format),
        last_export_format: decode_choice(
            file.last_export_format.as_deref(),
            FORMATS,
            d.last_export_format,
        ),
        parquet_compression: decode_choice(
            file.parquet_compression.as_deref(),
            CODECS,
            d.parquet_compression,
        ),
        show_inspector: file.show_inspector.unwrap_or(d.show_inspector),
        skip_hide_warning: file.skip_hide_warning.unwrap_or(d.skip_hide_warning),
        automatic_update_checks: file
            .automatic_update_checks
            .unwrap_or(d.automatic_update_checks),
        automatic_update_install: file
            .automatic_update_install
            .unwrap_or(d.automatic_update_install),
    }
}

pub(super) fn save(prefs: &Preferences) -> Result<(), String> {
    let Some(path) = settings_path() else {
        return Ok(());
    };
    save_to(&path, prefs)
}

pub(super) fn save_to(path: &Path, prefs: &Preferences) -> Result<(), String> {
    let file = SettingsFile {
        high_contrast: encode_choice(prefs.high_contrast, CONTRAST),
        font_size: Some(prefs.font_size),
        row_height: Some(prefs.row_height),
        show_row_numbers: Some(prefs.show_row_numbers),
        row_number_base: Some(prefs.row_number_base),
        copy_includes_headers: Some(prefs.copy_includes_headers),
        statistics_digits: Some(prefs.statistics_digits),
        remember_export_format: Some(prefs.remember_export_format),
        last_export_format: encode_choice(prefs.last_export_format, FORMATS),
        parquet_compression: encode_choice(prefs.parquet_compression, CODECS),
        show_inspector: Some(prefs.show_inspector),
        skip_hide_warning: Some(prefs.skip_hide_warning),
        automatic_update_checks: Some(prefs.automatic_update_checks),
        automatic_update_install: Some(prefs.automatic_update_install),
    };
    write_json(path, &file)
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
