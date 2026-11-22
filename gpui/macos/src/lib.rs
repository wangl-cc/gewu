//! Typed operations for native macOS methods whose bindings accept arbitrary Objective-C objects.
#![cfg(target_os = "macos")]
#![deny(unsafe_code, clippy::undocumented_unsafe_blocks)]

use objc2::MainThreadMarker;
use objc2_app_kit::NSDocumentController;
use objc2_foundation::{NSString, NSUserDefaults};

/// Clear this application's system recent documents, as in AppKit's Clear Menu action.
#[expect(unsafe_code, reason = "AppKit declares the sender nullable")]
pub fn clear_recent_documents(main: MainThreadMarker) {
    let controller = NSDocumentController::sharedDocumentController(main);
    // SAFETY: This AppKit action is on the main thread and accepts nil as its sender.
    unsafe { controller.clearRecentDocuments(None) }
}

/// Write a property-list string to the supplied defaults instance and its associated suite.
#[expect(unsafe_code, reason = "the supplied value is always an NSString")]
pub fn set_string_default(defaults: &NSUserDefaults, key: &str, value: &str) {
    let key = NSString::from_str(key);
    let value = NSString::from_str(value);
    // SAFETY: NSString is a supported property-list type. Both strings live through the call.
    unsafe { defaults.setObject_forKey(Some(&value), &key) }
}
