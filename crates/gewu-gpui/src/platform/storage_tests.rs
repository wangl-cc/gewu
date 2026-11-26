use super::*;

#[test]
fn atomic_json_writes_cleanly_and_state_updates_preserve_sibling_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/state.json");

    assert_eq!(AppState::load_from(&path), AppState::default());

    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"{not valid json").unwrap();
    assert_eq!(AppState::load_from(&path), AppState::default());

    update_state_at(&path, |state| {
        state.last_update_check = 1234.5;
    })
    .unwrap();
    update_state_at(&path, |state| {
        state.recent_files = vec![PathBuf::from("/tmp/a.parquet")];
    })
    .unwrap();

    let loaded = AppState::load_from(&path);
    assert_eq!(loaded.last_update_check, 1234.5);
    assert_eq!(loaded.recent_files, vec![PathBuf::from("/tmp/a.parquet")]);

    let entries: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec!["state.json"]);

    fs::write(
        &path,
        br#"{"recent_files":["/tmp/b.csv"],"last_update_check":-5.0,"unknown":true}"#,
    )
    .unwrap();
    let sanitized = AppState::load_from(&path);
    assert_eq!(sanitized.recent_files, vec![PathBuf::from("/tmp/b.csv")]);
    assert_eq!(sanitized.last_update_check, 0.);
}
