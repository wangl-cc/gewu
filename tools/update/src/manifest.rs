use std::{error::Error, io::Read};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::{digest, signature};
use semver::Version;
use serde::Deserialize;

pub(crate) type Result<T> = std::result::Result<T, Box<dyn Error>>;
const ARCHIVE_LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Deserialize)]
struct Envelope {
    payload: String,
    signature: String,
}

#[derive(Deserialize)]
pub(crate) struct Release {
    pub version: Version,
    pub build: u64,
    pub arch: String,
    pub minimum_macos: Version,
    pub length: u64,
    sha256: String,
}

impl Release {
    pub(crate) fn verify(body: &[u8], trusted_key: &str, arch: &str) -> Result<Self> {
        let env: Envelope = serde_json::from_slice(body)?;
        let bytes = STANDARD.decode(env.payload)?;
        let key =
            signature::UnparsedPublicKey::new(&signature::ED25519, STANDARD.decode(trusted_key)?);
        key.verify(&bytes, &STANDARD.decode(env.signature)?)
            .map_err(|_| "The update manifest signature is invalid.")?;
        let release: Self = serde_json::from_slice(&bytes)?;
        if release.arch != arch
            || release.build == 0
            || !(1..=ARCHIVE_LIMIT).contains(&release.length)
            || !release.version.pre.is_empty()
            || !release.version.build.is_empty()
            || release.sha256.len() != 64
            || !release.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("Unexpected architecture or invalid stable release metadata.".into());
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
        if expected.is_some_and(|version| *version != self.version) {
            return Err("The available release changed. Check for updates again.".into());
        }
        Ok(true)
    }

    pub(crate) fn verify_archive(&self, mut input: impl Read) -> Result<()> {
        let mut digest = digest::Context::new(&digest::SHA256);
        let mut count = 0u64;
        let mut buf = [0; 64 * 1024];
        loop {
            let n = input.read(&mut buf)?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > self.length {
                return Err("The downloaded archive exceeds its signed length.".into());
            }
            digest.update(&buf[..n]);
        }
        let actual: String = digest
            .finish()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if count != self.length || !actual.eq_ignore_ascii_case(&self.sha256) {
            return Err("The downloaded archive differs from the signed release.".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
