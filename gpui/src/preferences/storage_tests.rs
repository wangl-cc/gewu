use std::process::Command;

use objc2::{AnyThread, encode::Encoding, rc::Retained};
use objc2_foundation::{NSNumber, NSString, NSUserDefaults};

use super::{FileType, ParquetCompression, Preferences, read_from, write_changes};

struct Suite {
    name: String,
    defaults: Retained<NSUserDefaults>,
}

impl Suite {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let name = format!(
            "io.gewu.settings-test.{}",
            directory.path().file_name().unwrap().to_string_lossy()
        );
        let defaults = NSUserDefaults::initWithSuiteName(
            NSUserDefaults::alloc(),
            Some(&NSString::from_str(&name)),
        )
        .unwrap();
        Self { name, defaults }
    }

    fn set_string(&self, key: &str, value: &str) {
        assert!(
            Command::new("/usr/bin/defaults")
                .args(["write", &self.name, key, "-string", value])
                .status()
                .unwrap()
                .success()
        );
    }

    fn expect_string(&self, key: &str, value: &str) {
        let object = self
            .defaults
            .objectForKey(&NSString::from_str(key))
            .unwrap();
        assert_eq!(
            object.downcast_ref::<NSString>().unwrap().to_string(),
            value
        );
        let external = Command::new("/usr/bin/defaults")
            .args(["read", &self.name, key])
            .output()
            .unwrap();
        assert!(external.status.success());
        assert_eq!(
            String::from_utf8(external.stdout).unwrap().trim_end(),
            value
        );
    }

    fn expect_double(&self, key: &str, value: f64) {
        let object = self
            .defaults
            .objectForKey(&NSString::from_str(key))
            .unwrap();
        let number = object.downcast_ref::<NSNumber>().unwrap();
        assert_eq!(number.encoding(), Encoding::Double);
        assert_eq!(number.as_f64(), value);
    }
}

impl Drop for Suite {
    fn drop(&mut self) {
        self.defaults
            .removePersistentDomainForName(&NSString::from_str(&self.name));
    }
}

#[test]
fn swift_sizes_and_format_names_round_trip_in_their_native_types() {
    let suite = Suite::new();
    let defaults = &suite.defaults;
    defaults.setDouble_forKey(12.75, &NSString::from_str("TableFontSize"));
    defaults.setDouble_forKey(27.25, &NSString::from_str("TableRowHeight"));
    defaults.setBool_forKey(false, &NSString::from_str("CopyIncludesHeaders"));
    suite.set_string("LastExportFormat", "tsv");
    suite.set_string("ParquetCompression", "snappy");

    let original = read_from(defaults);
    assert_eq!((original.font_size, original.row_height), (12.75, 27.25));
    assert_eq!(original.last_export_format, FileType::Tsv);
    assert_eq!(original.parquet_compression, ParquetCompression::Snappy);
    assert!(!original.copy_includes_headers);

    let changed = Preferences {
        font_size: 12.5,
        row_height: 27.75,
        last_export_format: FileType::Xlsx,
        parquet_compression: ParquetCompression::Lz4,
        ..original
    };
    write_changes(defaults, &original, &changed).unwrap();
    suite.expect_double("TableFontSize", 12.5);
    suite.expect_double("TableRowHeight", 27.75);
    suite.expect_string("LastExportFormat", "xlsx");
    suite.expect_string("ParquetCompression", "lz4");
    assert!(read_from(defaults) == changed);
}

#[test]
fn early_gpui_integer_choices_remain_readable_and_new_writes_use_names() {
    let suite = Suite::new();
    let defaults = &suite.defaults;
    defaults.setInteger_forKey(1, &NSString::from_str("LastExportFormat"));
    defaults.setInteger_forKey(5, &NSString::from_str("ParquetCompression"));
    let original = read_from(defaults);
    assert_eq!(original.last_export_format, FileType::Csv);
    assert_eq!(
        original.parquet_compression,
        ParquetCompression::Uncompressed
    );

    let changed = Preferences {
        last_export_format: FileType::Tsv,
        parquet_compression: ParquetCompression::Gzip,
        ..original
    };
    write_changes(defaults, &original, &changed).unwrap();
    suite.expect_string("LastExportFormat", "tsv");
    suite.expect_string("ParquetCompression", "gzip");
}
