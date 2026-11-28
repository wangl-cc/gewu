//! Every Objective-C call the app makes: typed operations over AppKit and Foundation for the
//! open and export panels, the pasteboard, recent documents and display options.
use std::path::Path;

use objc2::{MainThreadMarker, rc::Retained};
use objc2_app_kit::{NSDocumentController, NSView, NSWindow};
use objc2_foundation::NSURL;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

mod display;
pub(crate) mod panels;
pub(crate) mod pasteboard;
pub(crate) use display::{DisplayObservation, accessibility_display_options};

/// The AppKit window behind a gpui window, to attach sheets to.
#[expect(
    unsafe_code,
    reason = "gpui hands out its AppKit view as an untyped pointer"
)]
pub(crate) fn ns_window(window: &gpui_kit::Window) -> Option<Retained<NSWindow>> {
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    // SAFETY: An AppKit window handle points to the window's live NSView, which outlives this
    // borrow of the gpui window; the view is only read on the main thread that owns it.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    view.window()
}

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
