//! Every Objective-C call the app makes: typed operations over AppKit and Foundation for the
//! open and export panels, the pasteboard, recent documents and display options.
use std::path::Path;

use objc2::MainThreadMarker;
use objc2_app_kit::NSDocumentController;
use objc2_foundation::NSURL;

mod display;
pub(crate) mod pasteboard;
mod picker;
pub(crate) mod save;
pub(crate) use display::{DisplayObservation, accessibility_display_options};
pub(crate) use picker::{OpenPanel, open_files};

/// Add `path` to this application's system recent documents.
pub(crate) fn note_recent_document(main: MainThreadMarker, path: &Path) {
    if let Some(url) = NSURL::from_file_path(path) {
        NSDocumentController::sharedDocumentController(main).noteNewRecentDocumentURL(&url);
    }
}

/// Clear this application's system recent documents, as in AppKit's Clear Menu action.
#[expect(unsafe_code, reason = "AppKit declares the sender nullable")]
pub(crate) fn clear_recent_documents(main: MainThreadMarker) {
    let controller = NSDocumentController::sharedDocumentController(main);
    // SAFETY: This AppKit action is on the main thread and accepts nil as its sender.
    unsafe { controller.clearRecentDocuments(None) }
}
