use std::path::{Path, PathBuf};

use gpui_kit::App;
#[cfg(target_os = "macos")]
use {objc2::MainThreadMarker, objc2_app_kit::NSDocumentController, objc2_foundation::NSURL};

use super::storage;

const MAX_RECENT: usize = 10;

pub(crate) struct RecentDocuments {
    paths: Vec<PathBuf>,
    state_path: Option<PathBuf>,
    #[cfg(target_os = "macos")]
    macos: Option<MainThreadMarker>,
}

impl RecentDocuments {
    pub(crate) fn system() -> Self {
        let state_path = storage::state_path();
        let mut paths = state_path
            .as_deref()
            .map(storage::AppState::load_from)
            .map_or_else(Vec::new, |state| state.recent_files);
        paths.truncate(MAX_RECENT);
        Self {
            paths,
            state_path,
            #[cfg(target_os = "macos")]
            macos: MainThreadMarker::new(),
        }
    }

    #[allow(dead_code)] // Used by shell tests and the visual-design capture host.
    pub(crate) fn session() -> Self {
        Self {
            paths: Vec::new(),
            state_path: None,
            #[cfg(target_os = "macos")]
            macos: None,
        }
    }

    pub(crate) fn paths(&self) -> Vec<PathBuf> {
        self.paths.clone()
    }

    pub(crate) fn record(&mut self, path: &Path, cx: &App) {
        self.paths.retain(|entry| entry != path);
        self.paths.insert(0, path.to_owned());
        self.paths.truncate(MAX_RECENT);

        if let Some(state_path) = self.state_path.as_deref() {
            let _ = storage::update_state_at(state_path, |state| {
                state.recent_files = self.paths.clone();
            });
        }
        #[cfg(target_os = "macos")]
        if let Some(main) = self.macos
            && let Some(url) = NSURL::from_file_path(path)
        {
            NSDocumentController::sharedDocumentController(main).noteNewRecentDocumentURL(&url);
        }
        #[cfg(not(target_os = "macos"))]
        cx.add_recent_document(path);
        #[cfg(target_os = "macos")]
        let _ = cx;
    }

    pub(crate) fn clear(&mut self) {
        self.paths.clear();
        if let Some(state_path) = self.state_path.as_deref() {
            let _ = storage::update_state_at(state_path, |state| {
                state.recent_files.clear();
            });
        }
        #[cfg(target_os = "macos")]
        if let Some(main) = self.macos {
            super::macos::clear_recent_documents(main);
        }
    }
}

#[cfg(test)]
#[path = "recent_tests.rs"]
mod tests;
