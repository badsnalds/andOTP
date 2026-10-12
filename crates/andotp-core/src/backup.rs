//! andOTP backup files.
//!
//! * **Plain text** (`otp_accounts.json`): a JSON array of entries.
//! * **Password protected** (`otp_accounts.json.aes`):
//!   `iterations (u32, big endian) || salt (12 bytes) || nonce (12 bytes) || AES-256-GCM(json)`
//!   with the key derived by PBKDF2-HMAC-SHA1(password, salt, iterations).
//! * **Legacy password protected** (pre 0.6): `nonce || AES-256-GCM(json)` keyed with SHA-256(password).
//!
//! OpenPGP backups are plain-text backups encrypted with OpenPGP; decrypt them with `gpg` first.

use rand::Rng;
use sha2::{Digest, Sha256};

use crate::crypto::{self, IV_LENGTH};
use crate::{Entry, Error, Result};

/// Range from which the PBKDF2 iteration count of new encrypted files is drawn.
///
/// The Android app used 140 000–160 000; current OWASP guidance for PBKDF2-HMAC-SHA1 is far
/// higher. The count is stored in the file header, so andOTP for Android can read these files.
pub const ITERATIONS: std::ops::RangeInclusive<u32> = 600_000..=660_000;
/// Upper bound accepted when reading a file header, so corrupt files cannot stall the program.
pub const MAX_ITERATIONS: u32 = 10_000_000;
const INT_LENGTH: usize = 4;
const HEADER_LENGTH: usize = INT_LENGTH + IV_LENGTH;

/// The kind of backup file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupFormat {
    Plain,
    Encrypted,
    EncryptedLegacy,
}

impl BackupFormat {
    /// The default file name the Android app uses (`timestamp` like `2024-01-31_12-00-00`).
    pub fn file_name(self, timestamp: Option<&str>) -> String {
        let ext = match self {
            Self::Plain => "json",
            Self::Encrypted | Self::EncryptedLegacy => "json.aes",
        };
        match timestamp {
            Some(ts) => format!("otp_accounts_{ts}.{ext}"),
            None => format!("otp_accounts.{ext}"),
        }
    }
}

/// Serialises entries to the plain-text backup format.
pub fn to_json(entries: &[Entry]) -> Result<String> {
    Ok(serde_json::to_string(entries)?)
}

/// Serialises entries as indented JSON (still a valid plain-text backup).
pub fn to_json_pretty(entries: &[Entry]) -> Result<String> {
    Ok(serde_json::to_string_pretty(entries)?)
}

/// Parses a plain-text backup.
pub fn from_json(json: &str) -> Result<Vec<Entry>> {
    Ok(serde_json::from_str(json.trim_start_matches('\u{feff}'))?)
}

/// Header and derived key of a password protected file.
pub(crate) struct KeyParams {
    pub iterations: u32,
    pub salt: [u8; IV_LENGTH],
}

impl KeyParams {
    pub fn generate() -> Self {
        let mut salt = [0u8; IV_LENGTH];
        salt.copy_from_slice(&crypto::random_bytes(IV_LENGTH));
        Self { iterations: rand::thread_rng().gen_range(ITERATIONS), salt }
    }

    pub fn parse(data: &[u8]) -> Result<(Self, &[u8])> {
        if data.len() < HEADER_LENGTH + IV_LENGTH + crypto::TAG_LENGTH {
            return Err(Error::Malformed);
        }
        let iterations = u32::from_be_bytes(data[..INT_LENGTH].try_into().expect("length checked"));
        if iterations == 0 || iterations > MAX_ITERATIONS {
            return Err(Error::Malformed);
        }
        let mut salt = [0u8; IV_LENGTH];
        salt.copy_from_slice(&data[INT_LENGTH..HEADER_LENGTH]);
        Ok((Self { iterations, salt }, &data[HEADER_LENGTH..]))
    }

    pub fn seal(&self, key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
        let encrypted = crypto::encrypt(key, plaintext)?;
        let mut out = Vec::with_capacity(HEADER_LENGTH + encrypted.len());
        out.extend_from_slice(&self.iterations.to_be_bytes());
        out.extend_from_slice(&self.salt);
        out.extend_from_slice(&encrypted);
        Ok(out)
    }
}

/// Encrypts arbitrary data in the password protected backup format.
pub fn encrypt(password: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
    let params = KeyParams::generate();
    let key = crypto::derive_key(password, &params.salt, params.iterations);
    params.seal(key.as_ref(), plaintext)
}

/// Decrypts a password protected backup.
pub fn decrypt(password: &str, data: &[u8]) -> Result<Vec<u8>> {
    let (params, body) = KeyParams::parse(data)?;
    let key = crypto::derive_key(password, &params.salt, params.iterations);
    crypto::decrypt(key.as_ref(), body)
}

/// Decrypts a backup in the legacy (SHA-256 keyed) format.
pub fn decrypt_legacy(password: &str, data: &[u8]) -> Result<Vec<u8>> {
    let key = Sha256::digest(password.as_bytes());
    crypto::decrypt(&key, data)
}

/// Encrypts entries into a password protected backup.
pub fn export_encrypted(entries: &[Entry], password: &str) -> Result<Vec<u8>> {
    encrypt(password, to_json(entries)?.as_bytes())
}

/// Reads any andOTP backup. Plain-text files are recognised automatically; encrypted files need
/// `password`. The legacy format is tried first: it is cheap to check (the GCM tag authenticates
/// the result), whereas misreading a legacy file as the current format would run PBKDF2 with a
/// random iteration count.
pub fn import(data: &[u8], password: Option<&str>) -> Result<(Vec<Entry>, BackupFormat)> {
    let trimmed = data.iter().position(|b| !b.is_ascii_whitespace()).map_or(&[][..], |i| &data[i..]);
    if trimmed.first() == Some(&b'[') || trimmed.starts_with("\u{feff}[".as_bytes()) {
        let json = std::str::from_utf8(data).map_err(|_| Error::Malformed)?;
        return Ok((from_json(json)?, BackupFormat::Plain));
    }
    let password = password.ok_or(Error::Decryption)?;
    let (plain, format) = match decrypt_legacy(password, data) {
        Ok(plain) => (plain, BackupFormat::EncryptedLegacy),
        Err(_) => (decrypt(password, data)?, BackupFormat::Encrypted),
    };
    let json = String::from_utf8(plain).map_err(|_| Error::Malformed)?;
    Ok((from_json(&json)?, format))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OtpType;

    fn sample() -> Vec<Entry> {
        vec![
            Entry::new(OtpType::Totp, "JBSWY3DPEHPK3PXP", "GitHub", "octocat").unwrap(),
            Entry::new(OtpType::Hotp, "MZXW6YTBOI", "", "counter").unwrap(),
        ]
    }

    #[test]
    fn plain_round_trip() {
        let json = to_json(&sample()).unwrap();
        let (entries, format) = import(json.as_bytes(), None).unwrap();
        assert_eq!(format, BackupFormat::Plain);
        assert_eq!(entries, sample());
    }

    #[test]
    fn encrypted_round_trip() {
        // Use a small iteration count to keep the test fast; the header carries it.
        let params = KeyParams { iterations: 1000, salt: [7; IV_LENGTH] };
        let key = crypto::derive_key("hunter2", &params.salt, params.iterations);
        let data = params.seal(key.as_ref(), to_json(&sample()).unwrap().as_bytes()).unwrap();

        let (entries, format) = import(&data, Some("hunter2")).unwrap();
        assert_eq!(format, BackupFormat::Encrypted);
        assert_eq!(entries, sample());
        assert!(import(&data, Some("wrong")).is_err());
        assert!(import(&data, None).is_err());
    }

    #[test]
    fn legacy_round_trip() {
        let key = Sha256::digest(b"old password");
        let data = crypto::encrypt(&key, to_json(&sample()).unwrap().as_bytes()).unwrap();
        let (entries, format) = import(&data, Some("old password")).unwrap();
        assert_eq!(format, BackupFormat::EncryptedLegacy);
        assert_eq!(entries, sample());
    }

    #[test]
    fn file_names() {
        assert_eq!(BackupFormat::Plain.file_name(None), "otp_accounts.json");
        assert_eq!(
            BackupFormat::Encrypted.file_name(Some("2024-01-31_12-00-00")),
            "otp_accounts_2024-01-31_12-00-00.json.aes"
        );
    }
}
