#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod actions;
mod clipboard;
mod columns;
mod data;
mod errors;
mod find;
mod panels;
mod platform;
mod preferences;
mod scroll;
mod selection;
mod shell;
mod table;
mod theme;

pub use table::TableView;

/// Format counts whose English noun takes a regular plural.
fn counted(n: u64, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// Launch Gewu and open each path in its own document window.
pub fn run(paths: impl Iterator<Item = std::path::PathBuf>) {
    shell::run(paths)
}
