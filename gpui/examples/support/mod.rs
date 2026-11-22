//! Return benchmark failures to Cargo or the invoking shell.
use std::{error::Error, fmt::Display, sync::OnceLock};

use gpui_kit::App;

static FAILURE: OnceLock<String> = OnceLock::new();

pub fn fail(error: impl Display, cx: &mut App) {
    let _ = FAILURE.set(error.to_string());
    #[cfg(target_os = "macos")]
    if let Some(main) = objc2::MainThreadMarker::new() {
        use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType};
        let app = NSApplication::sharedApplication(main);
        // GPUI's quit() calls [NSApplication terminate:], which exits successfully before
        // main can return the error. Stop and wake the event loop so main reports the failure.
        app.stop(None);
        if let Some(wake) = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
            NSEventType::ApplicationDefined, Default::default(), NSEventModifierFlags::empty(),
            0., 0, None, 0, 0, 0,
        ) {
            app.postEvent_atStart(&wake, true);
        }
        return;
    }
    cx.quit();
}

pub fn result() -> Result<(), Box<dyn Error>> {
    FAILURE
        .get()
        .map_or(Ok(()), |error| Err(error.clone().into()))
}
