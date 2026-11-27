//! Invoke the signed bundle's update helper. A development build has no helper, so it never
//! contacts the update service.
use std::{
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

use semver::Version;

/// The running application bundle and its update helper.
#[derive(Clone)]
pub(crate) struct Release {
    bundle: PathBuf,
    helper: PathBuf,
}

/// Why the helper did not report a release.
#[derive(Debug)]
pub(crate) enum HelperError {
    Launch(std::io::Error),
    /// The helper exited unsuccessfully, with what it wrote to standard error.
    Failed {
        status: ExitStatus,
        message: String,
    },
    Output(std::str::Utf8Error),
    Version(semver::Error),
    /// A prerelease, or not the version an installation was asked for.
    Unexpected(Version),
}

impl Release {
    /// The release of the running executable, if it is a packaged bundle with a helper.
    pub(crate) fn current() -> Option<Self> {
        Self::from_executable(&std::env::current_exe().ok()?)
    }

    /// The release containing `executable`, if it sits in a bundle with a helper.
    pub(crate) fn from_executable(executable: &Path) -> Option<Self> {
        let macos = executable.parent()?;
        let contents = macos.parent()?;
        let bundle = contents.parent()?;
        if macos.file_name()? != "MacOS"
            || contents.file_name()? != "Contents"
            || bundle.extension()? != "app"
        {
            return None;
        }
        let helper = contents.join("Helpers/gewu-update");
        helper.is_file().then(|| Self {
            bundle: bundle.to_owned(),
            helper,
        })
    }

    /// Check for a release, or with `expected`, install that release. Blocks until the
    /// helper exits. `None` means no newer release.
    pub(crate) fn run(&self, expected: Option<&Version>) -> Result<Option<Version>, HelperError> {
        let mut process = Command::new(&self.helper);
        process
            .arg(if expected.is_some() {
                "install"
            } else {
                "check"
            })
            .arg(&self.bundle);
        if let Some(version) = expected {
            process.arg(version.to_string());
        }
        let output = process.output().map_err(HelperError::Launch)?;
        if !output.status.success() {
            return Err(HelperError::Failed {
                status: output.status,
                message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        let output = std::str::from_utf8(&output.stdout)
            .map_err(HelperError::Output)?
            .trim();
        if output.is_empty() {
            return Ok(None);
        }
        let version = Version::parse(output).map_err(HelperError::Version)?;
        if !version.pre.is_empty()
            || !version.build.is_empty()
            || expected.is_some_and(|wanted| *wanted != version)
        {
            return Err(HelperError::Unexpected(version));
        }
        Ok(Some(version))
    }
}

#[cfg(all(test, unix))]
#[path = "update_tests.rs"]
mod tests;
