use std::path::{Path, PathBuf};

use gpui_kit::App;
#[cfg(target_os = "macos")]
use {objc2::MainThreadMarker, objc2_app_kit::NSDocumentController, objc2_foundation::NSURL};

use super::storage;

const MAX_RECENT: usize = 10;

pub(crate) enum RecentDocuments {
    Persisted {
        paths: Vec<PathBuf>,
        state_path: Option<PathBuf>,
        #[cfg(target_os = "macos")]
        macos: Option<MainThreadMarker>,
    },
    #[cfg(test)]
    Session(Vec<PathBuf>),
}

fn push_recent(paths: &mut Vec<PathBuf>, path: &Path, limit: Option<usize>) {
    paths.retain(|entry| entry != path);
    paths.insert(0, path.to_owned());
    if let Some(max) = limit {
        paths.truncate(max);
    }
}

impl RecentDocuments {
    pub(crate) fn system() -> Self {
        let state_path = storage::state_path();
        let mut paths = state_path
            .as_deref()
            .map(storage::AppState::load_from)
            .map_or_else(Vec::new, |state| state.recent_files);
        paths.truncate(MAX_RECENT);
        Self::Persisted {
            paths,
            state_path,
            #[cfg(target_os = "macos")]
            macos: MainThreadMarker::new(),
        }
    }

    pub(crate) fn paths(&self) -> Vec<PathBuf> {
        match self {
            Self::Persisted { paths, .. } => paths.clone(),
            #[cfg(test)]
            Self::Session(paths) => paths.clone(),
        }
    }

    pub(crate) fn record(&mut self, path: &Path, cx: &App) {
        match self {
            Self::Persisted {
                paths,
                state_path,
                #[cfg(target_os = "macos")]
                macos,
            } => {
                push_recent(paths, path, Some(MAX_RECENT));
                if let Some(state_path) = state_path.as_deref() {
                    let _ = storage::update_state_at(state_path, |state| {
                        state.recent_files = paths.clone();
                    });
                }
                #[cfg(target_os = "macos")]
                if let Some(main) = macos
                    && let Some(url) = NSURL::from_file_path(path)
                {
                    NSDocumentController::sharedDocumentController(*main)
                        .noteNewRecentDocumentURL(&url);
                }
                #[cfg(not(target_os = "macos"))]
                cx.add_recent_document(path);
                #[cfg(target_os = "macos")]
                let _ = cx;
            }
            #[cfg(test)]
            Self::Session(paths) => {
                cx.add_recent_document(path);
                push_recent(paths, path, None);
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        match self {
            Self::Persisted {
                paths,
                state_path,
                #[cfg(target_os = "macos")]
                macos,
            } => {
                paths.clear();
                if let Some(state_path) = state_path.as_deref() {
                    let _ = storage::update_state_at(state_path, |state| {
                        state.recent_files.clear();
                    });
                }
                #[cfg(target_os = "macos")]
                if let Some(main) = macos {
                    gewu_macos::clear_recent_documents(*main);
                }
            }
            #[cfg(test)]
            Self::Session(paths) => paths.clear(),
        }
    }
}

#[cfg(test)]
#[path = "recent_tests.rs"]
mod tests;
