#[cfg(target_os = "macos")]
mod demo;
#[cfg(target_os = "macos")]
mod scenes;
#[cfg(target_os = "macos")]
mod session;

#[cfg(not(target_os = "macos"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("This capture host needs macOS and Metal.")
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    use std::{
        env, fs,
        path::{Path, PathBuf},
        process::Command,
    };

    use anyhow::{Context as _, bail};

    use crate::{
        scenes::{GROUPS, run_group},
        session::Mode,
    };

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .context("locate workspace root")?;
    let target_dir = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root.join("target"));

    let mut sample: Option<PathBuf> = None;
    let mut output = target_dir.join("ui-capture");
    let mut modes = Vec::new();
    let mut groups = Vec::new();
    let mut demo_only = false;
    let mut demo_width: u32 = 1280;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--demo" => demo_only = true,
            "--width" => {
                let w = args.next().context("--width expects a pixel width")?;
                demo_width = w.parse().context("invalid --width")?;
                if demo_width == 0 {
                    bail!("--width must be positive");
                }
            }
            "--sample" => {
                let p = args.next().context("--sample expects a file path")?;
                sample = Some(fs::canonicalize(&p).unwrap_or_else(|_| PathBuf::from(p)));
            }
            "--output" => {
                let p = args.next().context("--output expects a directory path")?;
                let dir = PathBuf::from(p);
                output = if dir.is_absolute() {
                    dir
                } else {
                    env::current_dir()?.join(dir)
                };
            }
            "--mode" => match args.next().as_deref() {
                Some("standard") => modes = vec![Mode::Standard],
                Some("high-contrast") => modes = vec![Mode::HighContrast],
                Some(other) => {
                    bail!("unknown --mode {other:?} (expected standard or high-contrast)")
                }
                None => bail!("--mode expects standard or high-contrast"),
            },
            "--group" => {
                let g = args.next().context("--group expects a scene group name")?;
                if !GROUPS.contains(&g.as_str()) {
                    bail!("unknown --group {g:?} (valid: {})", GROUPS.join(", "));
                }
                groups.push(g);
            }
            "-h" | "--help" => {
                println!(
                    "Usage: cargo run --locked -p gewu-ui-capture -- [options]\n  \
                     --sample FILE    Use an existing gewu-data-gen 2M-row Parquet\n  \
                     --output DIR     Set the screenshot destination\n  \
                     --mode MODE      Capture one mode (standard or high-contrast)\n  \
                     --group GROUP    Repeat to select scene groups ({})\n  \
                     --demo           Record animated PNG (APNG) README demos\n  \
                     --width PX       Target width for --demo (default 1280)",
                    GROUPS.join(", ")
                );
                return Ok(());
            }
            other => bail!("unknown argument {other:?} (pass --help for usage)"),
        }
    }

    if modes.is_empty() {
        modes = vec![Mode::Standard, Mode::HighContrast];
    }
    if groups.is_empty() {
        groups = GROUPS.iter().map(|g| (*g).to_owned()).collect();
    }

    let work = target_dir.join("ui-capture-work");
    fs::create_dir_all(&work)?;
    let large_sample = if groups.iter().any(|g| g == "large") {
        let path = sample.unwrap_or_else(|| work.join("sample.parquet"));
        if !path.is_file() {
            let status = Command::new("cargo")
                .args(["run", "--locked", "-p", "gewu-data-gen", "--"])
                .arg(&path)
                .arg("2000000")
                .current_dir(repo_root)
                .status()?;
            if !status.success() {
                bail!("could not generate large fixture");
            }
        }
        Some(path)
    } else {
        None
    };

    let harbor = manifest_dir.join("fixtures/Harbor observations.csv");
    let corrupt = work.join("Damaged observations.parquet");
    fs::write(&corrupt, b"Incomplete example data")?;
    let text_system = gpui_kit::platform::current_platform(true).text_system();

    if demo_only {
        return demo::record_demo(&harbor, text_system, &output, demo_width);
    }

    for &mode in &modes {
        for group in &groups {
            let dataset = match group.as_str() {
                "corrupt" => corrupt.as_path(),
                "large" => large_sample.as_deref().context("missing large sample")?,
                _ => harbor.as_path(),
            };
            run_group(group, dataset, text_system.clone(), output.clone(), mode)?;
        }
    }
    Ok(())
}
