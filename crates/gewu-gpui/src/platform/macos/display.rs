//! Observe the same AppKit display options that are sampled on each appearance change.
use block2::RcBlock;
use objc2::{
    rc::Retained,
    runtime::{NSObjectProtocol, ProtocolObject},
};
use objc2_app_kit::{NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification};
use objc2_foundation::NSNotificationCenter;

pub fn accessibility_display_options() -> (bool, bool) {
    let workspace = NSWorkspace::sharedWorkspace();
    (
        workspace.accessibilityDisplayShouldIncreaseContrast(),
        workspace.accessibilityDisplayShouldDifferentiateWithoutColor(),
    )
}

pub struct DisplayObservation {
    center: Retained<NSNotificationCenter>,
    observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

impl DisplayObservation {
    #[expect(
        unsafe_code,
        reason = "the notification name is an AppKit constant; the block is thread-safe"
    )]
    pub fn new(changed: impl Fn() + Send + Sync + 'static) -> Self {
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        let block = RcBlock::new(move |_| changed());
        // SAFETY: Observe every sender on the posting thread. The callback only
        // captures Send + Sync state and never retains the notification.
        let observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
                None,
                None,
                &block,
            )
        };
        Self { center, observer }
    }
}

impl Drop for DisplayObservation {
    #[expect(
        unsafe_code,
        reason = "removes the registration from the same notification center"
    )]
    fn drop(&mut self) {
        // SAFETY: AppKit returned this observer token when registering on this center.
        unsafe { self.center.removeObserver((*self.observer).as_ref()) };
    }
}

#[cfg(test)]
#[path = "display_tests.rs"]
mod tests;
