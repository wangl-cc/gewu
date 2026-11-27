//! Entry points the crate's examples and benchmarks drive, outside the product API.
use std::path::PathBuf;

use gpui_kit::{AnyView, App, AppContext, Entity, Window};

use crate::{preferences::Preferences, shell};

/// Install Gewu's actions and globals for an example: default preferences with the Inspector
/// hidden, and recent documents kept in memory.
pub fn init(cx: &mut App) {
    shell::init_session(
        Preferences {
            show_inspector: false,
            ..Preferences::default()
        },
        cx,
    );
}

/// A document opened the way the app opens a file, to embed in an example's window.
pub struct Document(Entity<shell::document::Document>);

impl Document {
    /// Start loading `path`. Requires [`init`].
    pub fn open(path: PathBuf, window: &mut Window, cx: &mut App) -> Self {
        Self(cx.new(|cx| shell::document::Document::new(path, true, window, cx)))
    }

    pub fn view(&self) -> AnyView {
        self.0.clone().into()
    }

    pub fn is_loaded(&self, cx: &App) -> bool {
        self.0.read(cx).table().is_some()
    }

    /// Put display row `row` at the top of the loaded table.
    pub fn scroll_to_row(&self, row: u64, cx: &mut App) {
        if let Some(table) = self.0.read(cx).table() {
            table.update(cx, |table, cx| {
                table.scroll_to_row(row);
                cx.notify();
            });
        }
    }
}
