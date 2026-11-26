use super::*;

#[test]
fn release_metadata_validates_versions_builds_and_minimum_os() {
    let release =
        Release::parse(br#"{"version":"1.2.3","build":43,"minimum_macos":"15.0.0"}"#).unwrap();
    let current = Version::new(1, 0, 0);
    let system = Version::new(15, 0, 0);

    assert!(release.available_for(&current, 40, &system, None).unwrap());
    assert!(
        release
            .available_for(&current, 40, &system, Some(&Version::new(1, 2, 3)))
            .unwrap()
    );
    assert!(
        release
            .available_for(&current, 40, &system, Some(&Version::new(1, 2, 2)))
            .is_err()
    );
    assert!(!release.available_for(&current, 43, &system, None).unwrap());
    assert!(
        !release
            .available_for(
                &Version::new(2, 0, 0),
                40,
                &system,
                Some(&Version::new(2, 1, 0)),
            )
            .unwrap()
    );
    assert!(
        !release
            .available_for(&current, 40, &Version::new(14, 7, 0), None)
            .unwrap()
    );
    assert_eq!(parse_macos_version("15").unwrap(), Version::new(15, 0, 0));
    assert_eq!(parse_macos_version("15.2").unwrap(), Version::new(15, 2, 0));
    assert!(Release::parse(br#"{"version":"1.2.3","build":0,"minimum_macos":"15.0.0"}"#).is_err());
    assert!(
        Release::parse(br#"{"version":"1.2.3-rc.1","build":43,"minimum_macos":"15.0.0"}"#).is_err()
    );
    assert!(
        Release::parse(br#"{"version":"1.2.3+build","build":43,"minimum_macos":"15.0.0"}"#)
            .is_err()
    );
}
