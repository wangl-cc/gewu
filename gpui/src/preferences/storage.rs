use objc2_foundation::{NSString, NSUserDefaults};

use super::*;

// Swift persisted the lowercase names. Early GPUI builds stored indices into these lists.
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

fn choice<T: Copy>(value: Option<String>, items: &[(&str, T)], fallback: T) -> T {
    let Some(value) = value else { return fallback };
    value
        .parse::<usize>()
        .ok()
        .and_then(|i| items.get(i))
        .or_else(|| items.iter().find(|(name, _)| *name == value))
        .map_or(fallback, |(_, v)| *v)
}

pub(super) fn load() -> Preferences {
    read_from(&NSUserDefaults::standardUserDefaults())
}

fn read_from(defaults: &NSUserDefaults) -> Preferences {
    let bool_value = |key: &str, fallback| {
        let key = NSString::from_str(key);
        defaults
            .objectForKey(&key)
            .map_or(fallback, |_| defaults.boolForKey(&key))
    };
    let integer = |key: &str, fallback| {
        let key = NSString::from_str(key);
        defaults
            .objectForKey(&key)
            .map_or(fallback, |_| defaults.integerForKey(&key))
    };
    let dimension = |key: &str, fallback: f32, min, max| {
        let key = NSString::from_str(key);
        defaults
            .objectForKey(&key)
            .map(|_| defaults.doubleForKey(&key))
            .filter(|v| v.is_finite())
            .map_or(fallback, |v| v.clamp(min, max) as f32)
    };
    let string = |key| {
        defaults
            .stringForKey(&NSString::from_str(key))
            .map(|s| s.to_string())
    };
    let p = Preferences::default();
    Preferences {
        high_contrast: choice(string("HighContrast"), CONTRAST, p.high_contrast),
        font_size: dimension("TableFontSize", p.font_size, 10., 20.),
        row_height: dimension("TableRowHeight", p.row_height, 24., 36.),
        show_row_numbers: bool_value("ShowRowNumbers", p.show_row_numbers),
        row_number_base: integer("RowNumberBase", p.row_number_base as isize).clamp(0, 1) as u64,
        copy_includes_headers: bool_value("CopyIncludesHeaders", p.copy_includes_headers),
        statistics_digits: integer("StatisticsSignificantDigits", p.statistics_digits as isize)
            .clamp(3, 15) as usize,
        remember_export_format: bool_value("RememberExportFormat", p.remember_export_format),
        last_export_format: choice(string("LastExportFormat"), FORMATS, p.last_export_format),
        parquet_compression: choice(string("ParquetCompression"), CODECS, p.parquet_compression),
        show_inspector: bool_value("ShowColumnInspector", p.show_inspector),
        skip_hide_warning: bool_value("SkipHideColumnWarning", p.skip_hide_warning),
    }
}

pub(super) fn save(before: &Preferences, after: &Preferences) -> Result<(), String> {
    write_changes(&NSUserDefaults::standardUserDefaults(), before, after)
}

fn write_changes(
    defaults: &NSUserDefaults,
    before: &Preferences,
    after: &Preferences,
) -> Result<(), String> {
    for (key, old, new) in [
        ("TableFontSize", before.font_size, after.font_size),
        ("TableRowHeight", before.row_height, after.row_height),
    ] {
        if old != new {
            defaults.setDouble_forKey(f64::from(new), &NSString::from_str(key));
        }
    }
    for (key, old, new) in [
        (
            "StatisticsSignificantDigits",
            before.statistics_digits as isize,
            after.statistics_digits as isize,
        ),
        (
            "RowNumberBase",
            before.row_number_base as isize,
            after.row_number_base as isize,
        ),
    ] {
        if old != new {
            defaults.setInteger_forKey(new, &NSString::from_str(key));
        }
    }
    for (key, old, new) in [
        (
            "CopyIncludesHeaders",
            before.copy_includes_headers,
            after.copy_includes_headers,
        ),
        (
            "ShowRowNumbers",
            before.show_row_numbers,
            after.show_row_numbers,
        ),
        (
            "RememberExportFormat",
            before.remember_export_format,
            after.remember_export_format,
        ),
        (
            "ShowColumnInspector",
            before.show_inspector,
            after.show_inspector,
        ),
        (
            "SkipHideColumnWarning",
            before.skip_hide_warning,
            after.skip_hide_warning,
        ),
    ] {
        if old != new {
            defaults.setBool_forKey(new, &NSString::from_str(key));
        }
    }
    write_choice(
        defaults,
        "HighContrast",
        CONTRAST,
        before.high_contrast,
        after.high_contrast,
    )?;
    write_choice(
        defaults,
        "LastExportFormat",
        FORMATS,
        before.last_export_format,
        after.last_export_format,
    )?;
    write_choice(
        defaults,
        "ParquetCompression",
        CODECS,
        before.parquet_compression,
        after.parquet_compression,
    )
}

fn write_choice<T: PartialEq>(
    defaults: &NSUserDefaults,
    key: &str,
    items: &[(&str, T)],
    old: T,
    new: T,
) -> Result<(), String> {
    if old == new {
        return Ok(());
    }
    let (name, _) = items
        .iter()
        .find(|(_, v)| *v == new)
        .ok_or_else(|| format!("Unknown value for {key}"))?;
    gewu_macos::set_string_default(defaults, key, name);
    Ok(())
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
