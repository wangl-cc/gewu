//! Update a packaged Gewu application using its embedded release verification key.
#![forbid(unsafe_code)]

#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(test, target_os = "macos"))]
mod manifest;

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
