use std::fs;

use super::{FileType, HighContrast, ParquetCompression, Preferences, load_from, save_to};
use crate::theme::palettes::{catppuccin, default};

#[test]
fn settings_json_round_trips_all_preferences_including_enums_and_fractions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");

    assert!(load_from(&path) == Preferences::default());

    let custom = Preferences {
        light_theme: &catppuccin::LATTE,
        dark_theme: &catppuccin::MOCHA,
        high_contrast: HighContrast::On,
        font_size: 15.75,
        row_height: 31.25,
        show_row_numbers: false,
        row_number_base: 0,
        copy_includes_headers: false,
        statistics_digits: 9,
        remember_export_format: false,
        last_export_format: FileType::Xlsx,
        parquet_compression: ParquetCompression::Lz4,
        show_inspector: false,
        skip_hide_warning: true,
        automatic_update_checks: true,
        automatic_update_install: true,
    };
    save_to(&path, &custom).unwrap();
    assert!(load_from(&path) == custom);

    for mode in [HighContrast::System, HighContrast::Off, HighContrast::On] {
        let updated = Preferences {
            high_contrast: mode,
            ..custom
        };
        save_to(&path, &updated).unwrap();
        assert_eq!(load_from(&path).high_contrast, mode);
    }
    assert!(!HighContrast::Off.enabled(true));
    assert!(HighContrast::On.enabled(false));
    assert!(!HighContrast::System.enabled(false));
    assert!(HighContrast::System.enabled(true));
}

#[test]
fn partial_corrupt_or_out_of_range_settings_json_falls_back_per_field() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");

    fs::write(&path, b"{invalid json").unwrap();
    assert!(load_from(&path) == Preferences::default());

    fs::write(
        &path,
        br#"{
            "light_theme": "catppuccin-mocha",
            "dark_theme": "unknown-theme",
            "font_size": 99.0,
            "row_height": 10.0,
            "row_number_base": 5,
            "statistics_digits": 1,
            "last_export_format": "unknown-format",
            "parquet_compression": "brotli",
            "automatic_update_checks": true,
            "extra_future_key": 123
        }"#,
    )
    .unwrap();
    let loaded = load_from(&path);
    assert_eq!(loaded.light_theme, &default::LIGHT);
    assert_eq!(loaded.dark_theme, &default::DARK);
    assert_eq!(loaded.font_size, 20.);
    assert_eq!(loaded.row_height, 24.);
    assert_eq!(loaded.row_number_base, 1);
    assert_eq!(loaded.statistics_digits, 3);
    assert_eq!(loaded.last_export_format, FileType::Parquet);
    assert_eq!(loaded.parquet_compression, ParquetCompression::Brotli);
    assert!(loaded.automatic_update_checks);
    assert_eq!(loaded.show_inspector, Preferences::default().show_inspector);
}
