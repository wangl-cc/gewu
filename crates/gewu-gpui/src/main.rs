#![forbid(unsafe_code)]

fn main() {
    gewu_gpui::run(std::env::args_os().skip(1).map(std::path::PathBuf::from));
}
