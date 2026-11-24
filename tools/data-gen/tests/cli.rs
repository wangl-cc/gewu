use std::{fs, process::Command};

#[test]
fn zero_rows_preserves_the_destination() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sample.parquet");
    fs::write(&path, b"existing data").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_gewu-data-gen"))
        .arg(&path)
        .arg("0")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(fs::read(path).unwrap(), b"existing data");
}
