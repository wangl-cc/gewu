use std::{fs, os::unix::fs::PermissionsExt, process::Command};

use super::*;

#[test]
fn parses_valid_requirement_and_rejects_unsigned_and_adhoc_bundles() {
    assert!(
        requirement_from_str(
            r#"identifier "cc.loongw.Gewu" and anchor apple generic and certificate leaf[subject.OU] = "TEAMID1234""#
        )
        .is_ok()
    );
    assert!(requirement_from_str("not valid requirement syntax ((").is_err());

    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("Gewu.app");
    let exe = app.join("Contents/MacOS/Gewu");
    fs::create_dir_all(exe.parent().unwrap()).unwrap();
    fs::write(
        app.join("Contents/Info.plist"),
        br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>cc.loongw.Gewu</string>
  <key>CFBundleExecutable</key><string>Gewu</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
</dict></plist>"#,
    )
    .unwrap();
    fs::copy("/bin/echo", &exe).unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();

    // Unsigned bundle is rejected.
    assert!(verify_candidate(&app, &app).is_err());

    // Ad-hoc signed bundle is rejected as an update authority.
    let status = Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-"])
        .arg(&app)
        .status()
        .unwrap();
    assert!(status.success());
    let err = verify_candidate(&app, &app).unwrap_err().to_string();
    assert!(err.contains("ad-hoc"), "unexpected error: {err}");
}
