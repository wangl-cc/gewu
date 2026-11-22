use std::path::{Path, PathBuf};

use gpui_kit::App;
#[cfg(target_os = "macos")]
use {
    objc2::MainThreadMarker,
    objc2_app_kit::NSDocumentController,
    objc2_foundation::{NSString, NSURL, NSUserDefaults},
};

pub(crate) enum RecentDocuments {
    #[cfg(target_os = "macos")]
    System(MainThreadMarker),
    Session(Vec<PathBuf>),
}

// Early previews used a visible prefix of the system list after Clear Menu. Respect
// that choice until the user clears the system list, when the marker can be retired.
#[cfg(target_os = "macos")]
const LEGACY_LIMIT: &str = "GewuRecentMenuCount";

impl RecentDocuments {
    pub(crate) fn system() -> Self {
        #[cfg(target_os = "macos")]
        if let Some(main) = MainThreadMarker::new() {
            let mut history = Self::System(main);
            let defaults = NSUserDefaults::standardUserDefaults();
            let key = NSString::from_str(LEGACY_LIMIT);
            // A previous Clear Menu with no subsequent opens has no visible files to preserve.
            if defaults.objectForKey(&key).is_some() && defaults.integerForKey(&key) <= 0 {
                history.clear();
            }
            return history;
        }
        Self::Session(Vec::new())
    }

    pub(crate) fn paths(&self) -> Vec<PathBuf> {
        match self {
            #[cfg(target_os = "macos")]
            Self::System(main) => {
                let controller = NSDocumentController::sharedDocumentController(*main);
                let defaults = NSUserDefaults::standardUserDefaults();
                let key = NSString::from_str(LEGACY_LIMIT);
                let maximum = controller.maximumRecentDocumentCount();
                let limit = if defaults.objectForKey(&key).is_some() {
                    maximum.min(defaults.integerForKey(&key).max(0) as usize)
                } else {
                    maximum
                };
                controller
                    .recentDocumentURLs()
                    .iter()
                    .filter_map(|url| url.to_file_path())
                    .take(limit)
                    .collect()
            }
            Self::Session(paths) => paths.clone(),
        }
    }

    pub(crate) fn record(&mut self, path: &Path, cx: &App) {
        match self {
            #[cfg(target_os = "macos")]
            Self::System(main) => {
                let controller = NSDocumentController::sharedDocumentController(*main);
                let Some(url) = NSURL::from_file_path(path) else {
                    return;
                };
                let defaults = NSUserDefaults::standardUserDefaults();
                let key = NSString::from_str(LEGACY_LIMIT);
                let prefix = defaults.objectForKey(&key).map(|_| {
                    let shown = self.paths();
                    shown.len() + usize::from(!shown.iter().any(|p| p == path))
                });
                controller.noteNewRecentDocumentURL(&url);
                if let Some(count) = prefix {
                    defaults.setInteger_forKey(
                        count.min(controller.maximumRecentDocumentCount()) as isize,
                        &key,
                    );
                }
            }
            Self::Session(paths) => {
                cx.add_recent_document(path);
                paths.retain(|entry| entry != path);
                paths.insert(0, path.to_owned());
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        match self {
            #[cfg(target_os = "macos")]
            Self::System(main) => {
                gewu_macos::clear_recent_documents(*main);
                NSUserDefaults::standardUserDefaults()
                    .removeObjectForKey(&NSString::from_str(LEGACY_LIMIT));
            }
            Self::Session(paths) => paths.clear(),
        }
    }
}
