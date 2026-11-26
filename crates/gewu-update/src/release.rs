use std::error::Error;

use semver::Version;
use serde::Deserialize;

pub(crate) type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Deserialize)]
pub(crate) struct Release {
    pub version: Version,
    pub build: u64,
    pub minimum_macos: Version,
}

pub(crate) fn parse_macos_version(value: &str) -> Result<Version> {
    let mut padded = value.trim().to_owned();
    while padded.matches('.').count() < 2 {
        padded.push_str(".0");
    }
    Ok(Version::parse(&padded)?)
}

impl Release {
    pub(crate) fn parse(body: &[u8]) -> Result<Self> {
        let release: Self = serde_json::from_slice(body)?;
        if release.build == 0
            || !release.version.pre.is_empty()
            || !release.version.build.is_empty()
        {
            return Err("Invalid stable release metadata.".into());
        }
        Ok(release)
    }

    pub(crate) fn available_for(
        &self,
        version: &Version,
        build: u64,
        os: &Version,
        expected: Option<&Version>,
    ) -> Result<bool> {
        if &self.version <= version || self.build <= build || &self.minimum_macos > os {
            return Ok(false);
        }
        if expected.is_some_and(|wanted| *wanted != self.version) {
            return Err("The available release changed. Check for updates again.".into());
        }
        Ok(true)
    }
}

#[cfg(test)]
#[path = "release_tests.rs"]
mod tests;
