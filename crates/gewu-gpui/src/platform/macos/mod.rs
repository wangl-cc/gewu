//! Typed operations for native macOS methods whose bindings accept arbitrary Objective-C objects.
use objc2::MainThreadMarker;
use objc2_app_kit::NSDocumentController;

mod display;
mod picker;
pub(crate) use display::{DisplayObservation, accessibility_display_options};
pub(crate) use picker::open_files;

/// Clear this application's system recent documents, as in AppKit's Clear Menu action.
#[expect(unsafe_code, reason = "AppKit declares the sender nullable")]
pub(crate) fn clear_recent_documents(main: MainThreadMarker) {
    let controller = NSDocumentController::sharedDocumentController(main);
    // SAFETY: This AppKit action is on the main thread and accepts nil as its sender.
    unsafe { controller.clearRecentDocuments(None) }
}
