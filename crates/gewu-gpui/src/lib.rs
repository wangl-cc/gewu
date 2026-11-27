#![deny(unsafe_code, clippy::undocumented_unsafe_blocks)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod actions;
mod panels;
mod platform;
mod preferences;
mod shell;
mod table;
mod theme;
mod ui;

#[cfg(feature = "ui-capture")]
#[doc(hidden)]
pub mod capture;
#[doc(hidden)]
pub mod examples;

pub(crate) use ui::messages::counted;

/// Launch Gewu and open each path in its own document window.
pub fn run(paths: impl Iterator<Item = std::path::PathBuf>) {
    shell::run(paths)
}
