use thiserror::Error;

/// Errors produced by `andotp-core`.
#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid base32 secret")]
    InvalidSecret,
    #[error("invalid otpauth URI: {0}")]
    InvalidUri(String),
    #[error("invalid entry: {0}")]
    InvalidEntry(String),
    #[error("decryption failed (wrong password or corrupted data)")]
    Decryption,
    #[error("encryption failed")]
    Encryption,
    #[error("data is too short or malformed")]
    Malformed,
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
