//! Internal integration points for the standalone `gewu-ui-capture` tool crate.

use std::{path::PathBuf, sync::Arc};

use gpui_kit::{
    Action, AnyWindowHandle, App, AssetSource, Bounds, Entity, Pixels, Point, Size, Window,
    base::ThemeAppearance,
};

use crate::{actions, platform, preferences, shell, theme::GewuPalette};

/// Keeps a headless document session's open-URL channel alive for its lifetime.
pub struct DocumentSession {
    pub window: AnyWindowHandle,
    document: Entity<shell::document::Document>,
    _open_urls: async_channel::Sender<Vec<PathBuf>>,
}

/// Returns Gewu's bundled asset source for headless screenshot rendering.
pub fn assets() -> Arc<dyn AssetSource> {
    Arc::new(shell::assets::AppAssets)
}

/// Initializes Gewu globals/actions in an isolated session and opens `path` in a window.
pub fn open_document(
    path: PathBuf,
    window_size: Option<Size<Pixels>>,
    cx: &mut App,
) -> Option<DocumentSession> {
    gpui_kit::init(cx);
    actions::init(cx);
    cx.set_global(preferences::Preferences::default());
    let (send, recv) = async_channel::unbounded();
    shell::install_file_actions(platform::RecentDocuments::session(), recv, cx);
    let (window, document) = shell::open_document_with_size(path, window_size, cx)?;
    Some(DocumentSession {
        window,
        document,
        _open_urls: send,
    })
}

/// Opens (or focuses) the Settings window.
pub fn open_settings(cx: &mut App) {
    preferences::open(cx);
}

/// Installs the palette for `appearance` and current contrast preference and refreshes windows.
pub fn set_appearance(appearance: ThemeAppearance, cx: &mut App) {
    platform::install_theme(appearance, cx);
    cx.refresh_windows();
}

/// Reports whether the currently installed palette matches `"standard"` or `"high-contrast"`.
pub fn palette_mode(cx: &App) -> Option<&'static str> {
    let appearance = gpui_kit::base::Theme::global(cx).appearance;
    let installed = GewuPalette::get(cx);
    if installed == GewuPalette::default_for(appearance) {
        Some("standard")
    } else if installed == GewuPalette::high_contrast_for(appearance) {
        Some("high-contrast")
    } else {
        None
    }
}

/// Returns the clickable logical point inside cell `(column, row)` of the active table.
pub fn cell_point(
    session: &DocumentSession,
    column: &str,
    row: u64,
    cx: &App,
) -> Option<Point<Pixels>> {
    session
        .document
        .read(cx)
        .table()?
        .read(cx)
        .capture_cell_point(column, row)
}

/// Returns the logical bounds of the header for `column`.
pub fn header_bounds(session: &DocumentSession, column: &str, cx: &App) -> Option<Bounds<Pixels>> {
    session
        .document
        .read(cx)
        .table()?
        .read(cx)
        .capture_header_bounds(column)
}

/// Returns `(row_count, column_count)` of the loaded table, if open.
pub fn table_dimensions(session: &DocumentSession, cx: &App) -> Option<(u64, usize)> {
    Some(
        session
            .document
            .read(cx)
            .table()?
            .read(cx)
            .capture_dimensions(),
    )
}

/// Dispatches one of the named document actions supported by automated capture scenes.
pub fn dispatch_action(name: &str, window: &mut Window, cx: &mut App) -> bool {
    let action: Box<dyn Action> = match name {
        "columns" => Box::new(actions::ManageColumns),
        "sort" => Box::new(actions::EditSort),
        "filter" => Box::new(actions::EditFilter),
        "chart" => Box::new(actions::ShowChartInspector),
        "analyze" => Box::new(actions::AnalyzeSelectedColumn),
        _ => return false,
    };
    window.dispatch_action(action, cx);
    true
}
