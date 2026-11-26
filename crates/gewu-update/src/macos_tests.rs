use super::*;

#[test]
fn parses_bundle_info_plist_and_reports_running_os_version() {
    let plist = br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>cc.loongw.Gewu</string>
  <key>CFBundleShortVersionString</key><string>1.2.3</string>
  <key>CFBundleVersion</key><string>42</string>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
</dict></plist>"#;
    let info = parse_info_plist(plist).unwrap();
    assert_eq!(info.identifier, "cc.loongw.Gewu");
    assert_eq!(info.version, Version::new(1, 2, 3));
    assert_eq!(info.build, 42);
    assert_eq!(info.minimum_macos, Version::new(15, 0, 0));

    let os = os_version().unwrap();
    assert!(os >= Version::new(14, 0, 0));
}
