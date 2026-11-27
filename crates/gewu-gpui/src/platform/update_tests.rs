use std::{fs, os::unix::fs::PermissionsExt};

use super::{HelperError, Release};

#[test]
fn only_a_packaged_helper_can_offer_an_update() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("Gewu Beta.app");
    let executable = bundle.join("Contents/MacOS/Gewu");
    assert!(Release::from_executable(&executable).is_none());
    let helper = bundle.join("Contents/Helpers/gewu-update");
    fs::create_dir_all(helper.parent().unwrap()).unwrap();
    fs::write(&helper, "#!/bin/sh\necho '2.1.0'\n").unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    let release = Release::from_executable(&executable).unwrap();
    assert_eq!(release.run(None).unwrap().unwrap().to_string(), "2.1.0");
}

#[test]
fn withdrawn_release_and_unexpected_helper_version_cannot_report_an_installation() {
    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join("updater");
    let release = Release {
        bundle: dir.path().to_owned(),
        helper: helper.clone(),
    };
    let expected = semver::Version::new(1, 2, 3);
    fs::write(&helper, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(release.run(Some(&expected)).unwrap(), None);
    for invalid in ["bad", "1.2.4", "1.2.3-beta.2"] {
        fs::write(&helper, format!("#!/bin/sh\necho '{invalid}'\n")).unwrap();
        assert!(release.run(Some(&expected)).is_err());
    }
    fs::write(&helper, "#!/bin/sh\necho 'no network' >&2\nexit 3\n").unwrap();
    assert!(matches!(
        release.run(None),
        Err(HelperError::Failed { message, .. }) if message == "no network"
    ));
}
