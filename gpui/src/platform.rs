//! Native window defaults and the system document history.
mod recent;
pub(crate) mod update;

use gpui_kit::{
    base::{Theme, ThemeAppearance},
    *,
};
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
    #[cfg(target_os = "macos")]
    let accent = {
        use objc2_app_kit::{NSColor, NSColorSpace};
        NSColor::controlAccentColor()
            .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
            .map(|color| {
                Rgba {
                    r: color.redComponent() as f32,
                    g: color.greenComponent() as f32,
                    b: color.blueComponent() as f32,
                    a: 1.,
                }
                .into()
            })
    };
    #[cfg(not(target_os = "macos"))]
    let accent = None;
    let component_mode = match appearance {
        ThemeAppearance::Light => gpui_kit::component::ThemeMode::Light,
        ThemeAppearance::Dark => gpui_kit::component::ThemeMode::Dark,
    };
    if gpui_kit::component::Theme::global(cx).mode != component_mode {
        gpui_kit::component::Theme::change(component_mode, None, cx);
    }
    let colors = crate::theme::document_colors(appearance, accent);
    let components = gpui_kit::component::Theme::global(cx);
    if (
        components.primary,
        components.primary_foreground,
        components.ring,
    ) != (colors.primary, colors.primary_foreground, colors.ring)
    {
        gpui_kit::component::Theme::update(cx, |theme| {
            theme.primary = colors.primary;
            theme.primary_foreground = colors.primary_foreground;
            theme.ring = colors.ring;
        });
    }
    let theme = Theme::global_mut(cx);
    if theme.appearance != appearance || theme.tokens.colors != colors {
        theme.appearance = appearance;
        theme.tokens.colors = colors;
        cx.refresh_windows();
    }
}
