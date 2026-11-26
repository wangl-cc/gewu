//! Update a packaged Gewu application using native macOS security verification.
#![deny(unsafe_code, clippy::undocumented_unsafe_blocks)]

#[cfg(target_os = "macos")]
mod http;
#[cfg(any(test, target_os = "macos"))]
mod macho;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(test, target_os = "macos"))]
mod release;
#[cfg(target_os = "macos")]
mod security;

#[cfg(target_os = "macos")]
fn main() -> std::process::ExitCode {
    if let Err(error) = macos::run(std::env::args_os().skip(1)) {
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr().lock(), "{error}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}

#[cfg(not(target_os = "macos"))]
fn main() -> std::process::ExitCode {
    std::process::ExitCode::FAILURE
}
