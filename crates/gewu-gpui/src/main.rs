//! The Gewu application: opens each path given on the command line.
#![forbid(unsafe_code)]

fn main() {
    gewu_gpui::run(std::env::args_os().skip(1).map(std::path::PathBuf::from));
}
