use std::io::Cursor;

use ring::signature::Ed25519KeyPair;
use serde_json::json;

use super::*;

#[test]
fn signed_manifest_binds_architecture_version_os_and_archive() {
    let key = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
    let payload = serde_json::to_vec(&json!({
        "arch": "arm64", "version": "1.2.3", "build": 43, "minimum_macos": "15.0.0", "length": 3,
        "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    }))
    .unwrap();
    let signature = STANDARD.encode(key.sign(&payload).as_ref());
    let envelope = |body: &[u8]| {
        serde_json::to_vec(&json!({ "payload": STANDARD.encode(body), "signature": signature }))
            .unwrap()
    };
    let public = STANDARD.encode(key.public_key().as_ref());
    let signed = envelope(&payload);
    let release = Release::verify(&signed, &public, "arm64").unwrap();
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
                Some(&Version::new(2, 1, 0))
            )
            .unwrap()
    );
    assert!(
        !release
            .available_for(&current, 40, &Version::new(14, 7, 0), None)
            .unwrap()
    );
    assert!(release.verify_archive(Cursor::new(b"abc")).is_ok());
    for tampered in [&b"a"[..], b"abd", b"abcd"] {
        assert!(release.verify_archive(Cursor::new(tampered)).is_err());
    }
    assert!(Release::verify(&signed, &public, "x86_64").is_err());
    let mut tampered = payload;
    tampered.push(b' ');
    assert!(Release::verify(&envelope(&tampered), &public, "arm64").is_err());
}

#[test]
fn generated_keypair_signs_and_verifies_archive_manifest_round_trip() {
    let (private, public) = generate_keypair().unwrap();
    let (other_private, _) = generate_keypair().unwrap();
    let archive = b"gewu-release-archive";
    let release = Release::from_archive(
        Version::new(1, 2, 3),
        43,
        "arm64",
        Version::new(15, 0, 0),
        Cursor::new(archive),
    )
    .unwrap();
    let manifest = release.sign(&private, &public).unwrap();
    let verified = Release::verify(&manifest, &public, "arm64").unwrap();
    assert_eq!(verified.version, Version::new(1, 2, 3));
    assert_eq!(verified.build, 43);
    assert!(verified.verify_archive(Cursor::new(archive)).is_ok());
    assert!(
        verified
            .verify_archive(Cursor::new(b"gewu-release-archivf"))
            .is_err()
    );
    assert!(release.sign(&other_private, &public).is_err());
    assert!(release.sign(&STANDARD.encode(b"short"), &public).is_err());
    assert!(
        Release::from_archive(
            Version::new(1, 2, 3),
            0,
            "arm64",
            Version::new(15, 0, 0),
            Cursor::new(archive),
        )
        .is_err()
    );
}
