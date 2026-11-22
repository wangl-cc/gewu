use std::{
    ffi::OsString,
    fs::{self, File},
    path::Path,
    process::Command,
};

use rustix::fs::{CWD, RenameFlags, renameat_with};
use semver::Version;

use crate::manifest::{Release, Result};

const REPOSITORY: &str = "https://github.com/wangl-cc/gewu/releases";
const IDENTIFIER: &str = "io.github.wangl-cc.gewu";
#[cfg(target_arch = "aarch64")]
const ARCH: &str = "arm64";
#[cfg(target_arch = "x86_64")]
const ARCH: &str = "x86_64";

fn output(command: &mut Command) -> Result<Vec<u8>> {
    let result = command.output()?;
    if !result.status.success() {
        let details = String::from_utf8_lossy(&result.stderr);
        return Err(format!(
            "{} failed ({}): {}",
            command.get_program().to_string_lossy(),
            result.status,
            details.trim()
        )
        .into());
    }
    Ok(result.stdout)
}

fn text(command: &mut Command) -> Result<String> {
    Ok(String::from_utf8(output(command)?)?.trim().to_owned())
}

fn plist(app: &Path, key: &str) -> Result<String> {
    text(
        Command::new("/usr/bin/plutil")
            .args(["-extract", key, "raw", "-o", "-", "--"])
            .arg(app.join("Contents/Info.plist")),
    )
}

fn download(url: &str, max_bytes: u64) -> Command {
    let mut cmd = Command::new("/usr/bin/curl");
    cmd.args([
        "--disable",
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "20",
        "--max-time",
        "300",
        "--max-filesize",
    ])
    .arg(max_bytes.to_string())
    .arg("--url")
    .arg(url);
    cmd
}

fn os_version() -> Result<Version> {
    let mut value = text(Command::new("/usr/bin/sw_vers").arg("-productVersion"))?;
    while value.matches('.').count() < 2 {
        value.push_str(".0");
    }
    Ok(Version::parse(&value)?)
}

pub(super) fn run(mut args: impl Iterator<Item = OsString>) -> Result<()> {
    let action = args.next();
    let path = args
        .next()
        .ok_or("Usage: gewu-update check APP or gewu-update install APP EXPECTED_VERSION")?;
    let app = fs::canonicalize(path)?;
    if action.as_deref().and_then(|s| s.to_str()) == Some("verify") {
        let manifest = args.next().ok_or("Expected the release manifest path.")?;
        let archive = args.next().ok_or("Expected the release archive path.")?;
        if args.next().is_some() {
            return Err("Unexpected verification arguments.".into());
        }
        let release = Release::verify(&fs::read(manifest)?, &plist(&app, "SUPublicEDKey")?, ARCH)?;
        if release.version != Version::parse(&plist(&app, "CFBundleShortVersionString")?)?
            || release.build != plist(&app, "CFBundleVersion")?.parse::<u64>()?
        {
            return Err("Signed metadata and bundle versions differ.".into());
        }
        return release.verify_archive(File::open(archive)?);
    }
    let expected = match action.as_deref().and_then(|s| s.to_str()) {
        Some("check") => None,
        Some("install") => {
            let version = args
                .next()
                .ok_or("Install requires the version offered to the user.")?;
            Some(Version::parse(
                version.to_str().ok_or("Invalid expected version.")?,
            )?)
        }
        _ => return Err("Expected check or install.".into()),
    };
    if args.next().is_some() {
        return Err("Unexpected update arguments.".into());
    }
    let helper = app.join("Contents/Helpers/gewu-update");
    if fs::canonicalize(helper)? != fs::canonicalize(std::env::current_exe()?)?
        || plist(&app, "CFBundleIdentifier")? != IDENTIFIER
    {
        return Err("Run the update helper from the Gewu bundle being updated.".into());
    }

    // Concurrent installers for this bundle cannot both compare against the same old build.
    let parent = app
        .parent()
        .ok_or("The application has no enclosing directory.")?;
    let directory = File::open(parent)?;
    if expected.is_some() {
        directory.try_lock().map_err(
            |_| "Another application update holds this directory. Try again after it completes.",
        )?;
    }

    let key = plist(&app, "SUPublicEDKey")?;
    let version = Version::parse(&plist(&app, "CFBundleShortVersionString")?)?;
    let build = plist(&app, "CFBundleVersion")?.parse()?;
    let endpoint = format!("{REPOSITORY}/latest/download/update-{ARCH}.json");
    let release = Release::verify(&output(&mut download(&endpoint, 16 * 1024))?, &key, ARCH)?;
    if release.available_for(&version, build, &os_version()?, expected.as_ref())? {
        if expected.is_some() {
            replace(&app, &release, &key)?;
        }
        println!("{}", release.version);
    }
    Ok(())
}

fn replace(app: &Path, release: &Release, key: &str) -> Result<()> {
    let parent = app
        .parent()
        .ok_or("The application has no enclosing directory.")?;
    let staging = tempfile::Builder::new().prefix(".gewu-update-").tempdir_in(parent)
        .map_err(|error| format!("Cannot stage an update beside {}. Move Gewu to a writable Applications folder and retry. {error}", app.display()))?;
    let archive = staging.path().join("update.zip");
    let url = format!(
        "{REPOSITORY}/download/v{0}/Gewu-v{0}-macos-{ARCH}.zip",
        release.version
    );
    output(download(&url, release.length).arg("--output").arg(&archive))?;
    release.verify_archive(File::open(&archive)?)?;
    let expanded = staging.path().join("expanded");
    output(
        Command::new("/usr/bin/ditto")
            .args(["-x", "-k"])
            .arg(&archive)
            .arg(&expanded),
    )?;
    let candidate = expanded.join("Gewu.app");
    check_candidate(app, &candidate, release, key)?;
    // Atomic exchange preserves a complete installed bundle even if the helper is interrupted.
    renameat_with(CWD, candidate, CWD, app, RenameFlags::EXCHANGE)?;
    Ok(())
}

fn check_candidate(installed: &Path, candidate: &Path, release: &Release, key: &str) -> Result<()> {
    if plist(candidate, "CFBundleIdentifier")? != IDENTIFIER
        || plist(candidate, "CFBundleShortVersionString")? != release.version.to_string()
        || plist(candidate, "CFBundleVersion")? != release.build.to_string()
        || plist(candidate, "SUPublicEDKey")? != key
        || !candidate.join("Contents/Helpers/gewu-update").is_file()
    {
        return Err("The signed application metadata does not match the expected release.".into());
    }
    let requirement = text(
        Command::new("/usr/bin/codesign")
            .args(["-d", "-r-"])
            .arg(installed),
    )?;
    let requirement = requirement
        .strip_prefix("designated => ")
        .ok_or("Cannot read the installed application's signing requirement.")?;
    output(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict", "--check-notarization"])
            .arg(format!("-R={requirement}"))
            .arg(candidate),
    )?;
    output(
        Command::new("/usr/sbin/spctl")
            .args(["--assess", "--type", "execute"])
            .arg(candidate),
    )?;
    for executable in ["Contents/MacOS/Gewu", "Contents/Helpers/gewu-update"] {
        output(
            Command::new("/usr/bin/lipo")
                .arg(candidate.join(executable))
                .args(["-verify_arch", ARCH]),
        )?;
    }
    Ok(())
}
