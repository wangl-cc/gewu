#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod actions;
mod clipboard;
mod columns;
mod data;
mod errors;
mod find;
mod messages;
mod panels;
mod platform;
mod preferences;
mod scroll;
mod selection;
mod shell;
mod table;
mod theme;

pub(crate) use messages::counted;
pub use table::TableView;

/// Launch Gewu and open each path in its own document window.
pub fn run(paths: impl Iterator<Item = std::path::PathBuf>) {
    shell::run(paths)
}
