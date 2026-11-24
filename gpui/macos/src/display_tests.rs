use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use super::{
    DisplayObservation, NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
};

#[test]
#[expect(
    unsafe_code,
    reason = "the test posts an AppKit notification with a nil sender"
)]
fn workspace_delivers_changes_only_while_observation_is_alive() {
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let post = || {
        // SAFETY: The notification name belongs to this center. No sender object is required.
        unsafe {
            center.postNotificationName_object(
                NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
                None,
            )
        }
    };
    let count = Arc::new(AtomicUsize::new(0));
    let callback = count.clone();
    let observation = DisplayObservation::new(move || {
        callback.fetch_add(1, Ordering::Relaxed);
    });
    post();
    assert_eq!(count.load(Ordering::Relaxed), 1);
    drop(observation);
    post();
    assert_eq!(count.load(Ordering::Relaxed), 1);
}
