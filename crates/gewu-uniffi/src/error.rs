//! Structured errors crossing the language boundary.

use gewu_core as core;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum GewuError {
    #[error("{message}")]
    Io { message: String },
    #[error("{message}")]
    Unsupported { message: String },
    #[error("{message}")]
    InvalidArgument { message: String },
    #[error("cancelled")]
    Cancelled,
    #[error("stale view version {requested}, current is {current}")]
    StaleVersion { requested: u64, current: u64 },
    #[error("{message}")]
    Internal { message: String },
}

impl From<core::Error> for GewuError {
    fn from(e: core::Error) -> Self {
        match e {
            core::Error::Io(e) => GewuError::Io {
                message: e.to_string(),
            },
            core::Error::Unsupported(message) => GewuError::Unsupported { message },
            core::Error::InvalidArgument(message) => GewuError::InvalidArgument { message },
            core::Error::Cancelled => GewuError::Cancelled,
            core::Error::StaleVersion { requested, current } => {
                GewuError::StaleVersion { requested, current }
            }
            other => GewuError::Internal {
                message: other.to_string(),
            },
        }
    }
}
