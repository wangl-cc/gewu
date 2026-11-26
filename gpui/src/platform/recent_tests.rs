use gpui_kit::TestAppContext;

use super::*;

fn persisted_at(state_path: &Path) -> RecentDocuments {
    let mut paths = storage::AppState::load_from(state_path).recent_files;
    paths.truncate(MAX_RECENT);
    RecentDocuments::Persisted {
        paths,
        state_path: Some(state_path.to_owned()),
        #[cfg(target_os = "macos")]
        macos: None,
    }
}

#[test]
fn persisted_recent_documents_deduplicate_cap_reload_and_preserve_other_state() {
    let dir = tempfile::tempdir().unwrap();
    let state_path = dir.path().join("state.json");
    storage::update_state_at(&state_path, |state| state.last_update_check = 42.0).unwrap();

    let app = TestAppContext::single();
    let mut recent = persisted_at(&state_path);
    app.update(|cx| {
        for i in 0..12 {
            recent.record(&dir.path().join(format!("file-{i}.parquet")), cx);
        }
        recent.record(&dir.path().join("file-5.parquet"), cx);
    });

    let paths = recent.paths();
    assert_eq!(paths.len(), MAX_RECENT);
    assert_eq!(paths[0], dir.path().join("file-5.parquet"));
    assert_eq!(paths[1], dir.path().join("file-11.parquet"));
    assert!(!paths.contains(&dir.path().join("file-0.parquet")));
    assert!(!paths.contains(&dir.path().join("file-1.parquet")));

    let reloaded = persisted_at(&state_path);
    assert_eq!(reloaded.paths(), paths);
    assert_eq!(
        storage::AppState::load_from(&state_path).last_update_check,
        42.0
    );

    recent.clear();
    assert!(recent.paths().is_empty());
    let after_clear = storage::AppState::load_from(&state_path);
    assert!(after_clear.recent_files.is_empty());
    assert_eq!(after_clear.last_update_check, 42.0);
}
