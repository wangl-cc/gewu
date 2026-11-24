//! Native window defaults and the system document history.
mod recent;
pub(crate) mod update;

use gpui_kit::{base::ThemeAppearance, *};
pub(crate) use recent::RecentDocuments;

pub(crate) fn window_options(cx: &mut App) -> WindowOptions {
    let options = sized_window(1280., 800., cx);
    #[cfg(target_os = "macos")]
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.), px(19.))),
        }),
        ..options
    };
    options
}

pub(crate) fn sized_window(width: f32, height: f32, cx: &mut App) -> WindowOptions {
    let size = size(px(width), px(height));
    #[cfg(feature = "gpui-fast")]
    let bounds = WindowBounds::centered(None, size, cx);
    #[cfg(not(feature = "gpui-fast"))]
    let bounds = WindowBounds::centered(size, cx);
    WindowOptions {
        window_bounds: Some(bounds),
        focus: true,
        ..WindowOptions::default()
    }
}

/// Register on the window so appearance continues to update when only an auxiliary window is open.
pub(crate) fn follow_appearance(window: &Window, cx: &mut App) {
    apply_appearance(window, cx);
    window
        .observe_window_appearance(|w, cx| apply_appearance(w, cx))
        .detach();
}

fn apply_appearance(window: &Window, cx: &mut App) {
    let appearance = match window.appearance() {
        WindowAppearance::Light | WindowAppearance::VibrantLight => ThemeAppearance::Light,
        WindowAppearance::Dark | WindowAppearance::VibrantDark => ThemeAppearance::Dark,
    };
    install_theme(appearance, cx);
}

pub(crate) fn refresh_contrast(cx: &mut App) {
    install_theme(base::Theme::global(cx).appearance, cx);
}

fn install_theme(appearance: ThemeAppearance, cx: &mut App) {
    use crate::{
        preferences::Preferences,
        theme::{GewuPalette, VisualCues},
    };
    #[cfg(target_os = "macos")]
    let (system_contrast, differentiate) = gewu_macos::accessibility_display_options();
    #[cfg(not(target_os = "macos"))]
    let (system_contrast, differentiate) = (false, false);
    let contrast = Preferences::get(cx).high_contrast.enabled(system_contrast);
    let cues = VisualCues::for_enhancement(contrast || differentiate);
    if cx.try_global::<VisualCues>() != Some(&cues) {
        cx.set_global(cues);
        cx.refresh_windows();
    }
    let palette = if contrast {
        GewuPalette::high_contrast_for(appearance)
    } else {
        GewuPalette::default_for(appearance)
    };
    palette.install(cx);
}

#[cfg(target_os = "macos")]
pub(crate) fn observe_display_options(cx: &mut App) {
    struct DisplayUpdates {
        _observation: gewu_macos::DisplayObservation,
        _updates: Task<()>,
    }
    impl Global for DisplayUpdates {}
    // Each wake reads the latest system settings, so a pending wake covers further notifications.
    let (send, recv) = async_channel::bounded(1);
    let observation = gewu_macos::DisplayObservation::new(move || {
        let _ = send.try_send(());
    });
    let updates = cx.spawn(async move |cx| {
        while recv.recv().await.is_ok() {
            cx.update(refresh_contrast);
        }
    });
    cx.set_global(DisplayUpdates {
        _observation: observation,
        _updates: updates,
    });
}
