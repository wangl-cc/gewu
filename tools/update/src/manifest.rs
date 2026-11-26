use std::{error::Error, io::Read};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::{
    digest,
    rand::{SecureRandom as _, SystemRandom},
    signature::{self, Ed25519KeyPair, KeyPair as _},
};
use semver::Version;
use serde::{Deserialize, Serialize};

pub(crate) type Result<T> = std::result::Result<T, Box<dyn Error>>;
const ARCHIVE_LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
struct Envelope {
    payload: String,
    signature: String,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct Release {
    pub version: Version,
    pub build: u64,
    pub arch: String,
    pub minimum_macos: Version,
    pub length: u64,
    sha256: String,
}

pub(crate) fn generate_keypair() -> Result<(String, String)> {
    let mut seed = [0u8; 32];
    SystemRandom::new()
        .fill(&mut seed)
        .map_err(|_| "Failed to read system randomness for Ed25519 key generation.")?;
    let pair =
        Ed25519KeyPair::from_seed_unchecked(&seed).map_err(|_| "Failed to derive Ed25519 key.")?;
    Ok((
        STANDARD.encode(seed),
        STANDARD.encode(pair.public_key().as_ref()),
    ))
}

fn hash_archive(mut input: impl Read, limit: u64) -> Result<(u64, String)> {
    let mut digest = digest::Context::new(&digest::SHA256);
    let mut count = 0u64;
    let mut buf = [0; 64 * 1024];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > limit {
            return Err("The archive exceeds the permitted length.".into());
        }
        digest.update(&buf[..n]);
    }
    let sha256 = digest
        .finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((count, sha256))
}

impl Release {
    fn validate(&self) -> Result<()> {
        if !matches!(self.arch.as_str(), "arm64" | "x86_64")
            || self.build == 0
            || !(1..=ARCHIVE_LIMIT).contains(&self.length)
            || !self.version.pre.is_empty()
            || !self.version.build.is_empty()
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("Unexpected architecture or invalid stable release metadata.".into());
        }
        Ok(())
    }

    pub(crate) fn from_archive(
        version: Version,
        build: u64,
        arch: &str,
        minimum_macos: Version,
        input: impl Read,
    ) -> Result<Self> {
        let (length, sha256) = hash_archive(input, ARCHIVE_LIMIT)?;
        let release = Self {
            version,
            build,
            arch: arch.to_owned(),
            minimum_macos,
            length,
            sha256,
        };
        release.validate()?;
        Ok(release)
    }

    pub(crate) fn sign(
        &self,
        private_key_base64: &str,
        public_key_base64: &str,
    ) -> Result<Vec<u8>> {
        self.validate()?;
        let seed = STANDARD.decode(private_key_base64.trim())?;
        let public = STANDARD.decode(public_key_base64.trim())?;
        if seed.len() != 32 || public.len() != 32 {
            return Err("Expected 32-byte base64 Ed25519 keys.".into());
        }
        let pair = Ed25519KeyPair::from_seed_and_public_key(&seed, &public)
            .map_err(|_| "The signing private key does not match the bundle public key.")?;
        let payload = serde_json::to_vec(self)?;
        let envelope = Envelope {
            payload: STANDARD.encode(&payload),
            signature: STANDARD.encode(pair.sign(&payload).as_ref()),
        };
        Ok(serde_json::to_vec(&envelope)?)
    }

    pub(crate) fn verify(body: &[u8], trusted_key: &str, arch: &str) -> Result<Self> {
        let env: Envelope = serde_json::from_slice(body)?;
        let bytes = STANDARD.decode(env.payload)?;
        let key =
            signature::UnparsedPublicKey::new(&signature::ED25519, STANDARD.decode(trusted_key)?);
        key.verify(&bytes, &STANDARD.decode(env.signature)?)
            .map_err(|_| "The update manifest signature is invalid.")?;
        let release: Self = serde_json::from_slice(&bytes)?;
        release.validate()?;
        if release.arch != arch {
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

    pub(crate) fn verify_archive(&self, input: impl Read) -> Result<()> {
        let (count, actual) = hash_archive(input, self.length)?;
        if count != self.length || !actual.eq_ignore_ascii_case(&self.sha256) {
            return Err("The downloaded archive differs from the signed release.".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
