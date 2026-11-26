use std::{
    ffi::OsString,
    fs::{self, File},
    path::Path,
    process::Command,
    ptr,
};

use objc2::runtime::AnyObject;
use objc2_foundation::{
    NSData, NSDictionary, NSProcessInfo, NSPropertyListMutabilityOptions,
    NSPropertyListSerialization, NSString,
};
use rustix::fs::{CWD, RenameFlags, renameat_with};
use semver::Version;

use crate::{
    http, macho,
    release::{Release, Result, parse_macos_version},
    security,
};

const REPOSITORY: &str = "https://github.com/wangl-cc/gewu/releases";
const IDENTIFIER: &str = "cc.loongw.Gewu";
const ARCHIVE_LIMIT: u64 = 512 * 1024 * 1024;
#[cfg(target_arch = "aarch64")]
const ARCH: &str = "arm64";
#[cfg(target_arch = "x86_64")]
const ARCH: &str = "x86_64";

pub(crate) struct BundleInfo {
    pub identifier: String,
    pub version: Version,
    pub build: u64,
    pub minimum_macos: Version,
}

#[expect(
    unsafe_code,
    reason = "parses Info.plist via Foundation NSPropertyListSerialization"
)]
fn parse_info_plist(bytes: &[u8]) -> Result<BundleInfo> {
    let data = NSData::with_bytes(bytes);
    // SAFETY: `data` is a valid immutable `NSData`, and passing null for format out-param is
    // documented.
    let object = unsafe {
        NSPropertyListSerialization::propertyListWithData_options_format_error(
            &data,
            NSPropertyListMutabilityOptions::Immutable,
            ptr::null_mut(),
        )
    }
    .map_err(|err| err.localizedDescription().to_string())?;
    let dict = object
        .downcast_ref::<NSDictionary>()
        .ok_or("Info.plist root is not a dictionary.")?;
    let string = |key: &str| -> Result<String> {
        let key: &AnyObject = &NSString::from_str(key);
        let value = dict
            .objectForKey(key)
            .ok_or("Missing required Info.plist key.")?;
        value
            .downcast_ref::<NSString>()
            .map(|s| s.to_string())
            .ok_or_else(|| "Info.plist value is not a string.".into())
    };
    Ok(BundleInfo {
        identifier: string("CFBundleIdentifier")?,
        version: Version::parse(&string("CFBundleShortVersionString")?)?,
        build: string("CFBundleVersion")?.parse()?,
        minimum_macos: parse_macos_version(&string("LSMinimumSystemVersion")?)?,
    })
}

fn read_info_plist(app: &Path) -> Result<BundleInfo> {
    let bytes = fs::read(app.join("Contents/Info.plist"))?;
    parse_info_plist(&bytes)
}

fn os_version() -> Result<Version> {
    let version = NSProcessInfo::processInfo().operatingSystemVersion();
    Ok(Version::new(
        u64::try_from(version.majorVersion)?,
        u64::try_from(version.minorVersion)?,
        u64::try_from(version.patchVersion)?,
    ))
}

pub(super) fn run(mut args: impl Iterator<Item = OsString>) -> Result<()> {
    let action = args.next();
    let path = args
        .next()
        .ok_or("Usage: gewu-update check APP or gewu-update install APP EXPECTED_VERSION")?;
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
    let app = fs::canonicalize(path)?;
    let info = read_info_plist(&app)?;
    let helper = app.join("Contents/Helpers/gewu-update");
    if fs::canonicalize(helper)? != fs::canonicalize(std::env::current_exe()?)?
        || info.identifier != IDENTIFIER
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

    let endpoint = format!("{REPOSITORY}/latest/download/update.json");
    let release = Release::parse(&http::get_bytes(&endpoint, 16 * 1024)?)?;
    let os = os_version()?;
    if release.available_for(&info.version, info.build, &os, expected.as_ref())? {
        if expected.is_some() {
            replace(&app, &release, &os)?;
        }
        println!("{}", release.version);
    }
    Ok(())
}

fn replace(app: &Path, release: &Release, os: &Version) -> Result<()> {
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
    http::download_to_file(&url, &archive, ARCHIVE_LIMIT)?;
    let expanded = staging.path().join("expanded");
    let ditto = Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(&archive)
        .arg(&expanded)
        .output()?;
    if !ditto.status.success() {
        let details = String::from_utf8_lossy(&ditto.stderr);
        return Err(format!(
            "/usr/bin/ditto failed ({}): {}",
            ditto.status,
            details.trim()
        )
        .into());
    }
    let candidate = expanded.join("Gewu.app");
    check_candidate(app, &candidate, release, os)?;
    // Atomic exchange preserves a complete installed bundle even if the helper is interrupted.
    renameat_with(CWD, candidate, CWD, app, RenameFlags::EXCHANGE)?;
    Ok(())
}

fn check_candidate(
    installed: &Path,
    candidate: &Path,
    release: &Release,
    os: &Version,
) -> Result<()> {
    let info = read_info_plist(candidate)?;
    if info.identifier != IDENTIFIER
        || info.version != release.version
        || info.build != release.build
        || &info.minimum_macos > os
        || !candidate.join("Contents/Helpers/gewu-update").is_file()
    {
        return Err("The staged application metadata does not match the expected release.".into());
    }
    for executable in ["Contents/MacOS/Gewu", "Contents/Helpers/gewu-update"] {
        macho::verify_arch(File::open(candidate.join(executable))?, ARCH)?;
    }
    security::verify_candidate(installed, candidate)
}

#[cfg(test)]
#[path = "macos_tests.rs"]
mod tests;
