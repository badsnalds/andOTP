//! The encrypted on-disk database.
//!
//! A vault file uses exactly the password protected andOTP backup format, so it can be
//! imported into andOTP for Android (or any other andOTP compatible tool) as is.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::backup::{self, KeyParams};
use crate::crypto::{self, KEY_LENGTH};
use crate::{Entry, Error, Result};

/// File name of the vault inside the data directory.
pub const FILE_NAME: &str = "secrets.json.aes";

/// Result of merging entries into a vault.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ImportSummary {
    pub added: usize,
    pub skipped: usize,
}

/// An unlocked vault.
pub struct Vault {
    path: PathBuf,
    params: KeyParams,
    key: Zeroizing<[u8; KEY_LENGTH]>,
    pub entries: Vec<Entry>,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault").field("path", &self.path).field("entries", &self.entries.len()).finish()
    }
}

impl Vault {
    /// The per-user default location (e.g. `~/.local/share/andotp/secrets.json.aes` on Linux).
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("org", "shadowice", "andOTP").map(|d| d.data_dir().join(FILE_NAME))
    }

    /// Creates a new, empty vault protected by `password`.
    pub fn create(path: impl Into<PathBuf>, password: &str) -> Result<Self> {
        Self::create_with_params(path.into(), password, KeyParams::generate())
    }

    /// Like [`Vault::create`] with an explicit PBKDF2 iteration count (mainly for tests).
    pub fn create_with_iterations(path: impl Into<PathBuf>, password: &str, iterations: u32) -> Result<Self> {
        let mut params = KeyParams::generate();
        params.iterations = iterations.max(1);
        Self::create_with_params(path.into(), password, params)
    }

    fn create_with_params(path: PathBuf, password: &str, params: KeyParams) -> Result<Self> {
        if path.exists() {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} already exists", path.display()),
            )));
        }
        let key = crypto::derive_key(password, &params.salt, params.iterations);
        let vault = Self { path, params, key, entries: Vec::new() };
        vault.save()?;
        Ok(vault)
    }

    /// Opens and decrypts an existing vault.
    pub fn open(path: impl Into<PathBuf>, password: &str) -> Result<Self> {
        let path = path.into();
        let data = fs::read(&path)?;
        let (params, body) = KeyParams::parse(&data)?;
        let key = crypto::derive_key(password, &params.salt, params.iterations);
        let plain = Zeroizing::new(crypto::decrypt(key.as_ref(), body)?);
        let json = std::str::from_utf8(&plain).map_err(|_| Error::Malformed)?;
        let entries = backup::from_json(json)?;
        Ok(Self { path, params, key, entries })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Encrypts and writes the vault atomically, keeping the previous version as `*.bck`.
    pub fn save(&self) -> Result<()> {
        let json = Zeroizing::new(backup::to_json(&self.entries)?);
        let data = self.params.seal(self.key.as_ref(), json.as_bytes())?;

        if let Some(dir) = self.path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let tmp = sibling(&self.path, "tmp");
        write_private(&tmp, &data)?;
        if self.path.exists() {
            fs::copy(&self.path, sibling(&self.path, "bck"))?;
        }
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Re-encrypts the vault with a new password (and fresh salt).
    pub fn change_password(&mut self, password: &str) -> Result<()> {
        let params = KeyParams::generate();
        self.key = crypto::derive_key(password, &params.salt, params.iterations);
        self.params = params;
        self.save()
    }

    /// Adds an entry unless the same account already exists. Returns whether it was added.
    pub fn add(&mut self, entry: Entry) -> bool {
        if self.entries.iter().any(|e| e.same_account(&entry)) {
            return false;
        }
        self.entries.push(entry);
        true
    }

    /// Adds all entries that are not already present.
    pub fn import(&mut self, entries: impl IntoIterator<Item = Entry>) -> ImportSummary {
        let mut summary = ImportSummary::default();
        for entry in entries {
            if self.add(entry) {
                summary.added += 1;
            } else {
                summary.skipped += 1;
            }
        }
        summary
    }

    /// Moves the entry at `from` to position `to` (manual ordering).
    pub fn move_entry(&mut self, from: usize, to: usize) {
        if from < self.entries.len() && to < self.entries.len() {
            let entry = self.entries.remove(from);
            self.entries.insert(to, entry);
        }
    }
}

fn sibling(path: &Path, ext: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(ext);
    path.with_file_name(name)
}

/// Writes `data` to `path`, readable only by the current user where the platform supports it.
pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut file = private_file(path)?;
    file.write_all(data)?;
    file.sync_all()
}

#[cfg(unix)]
fn private_file(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)
}

#[cfg(not(unix))]
fn private_file(path: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new().write(true).create(true).truncate(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OtpType;

    #[test]
    fn create_save_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(FILE_NAME);

        let mut vault = Vault::create_with_iterations(&path, "pw", 1000).unwrap();
        assert!(Vault::create_with_iterations(&path, "pw", 1000).is_err());

        let entry = Entry::new(OtpType::Totp, "JBSWY3DPEHPK3PXP", "GitHub", "me").unwrap();
        assert!(vault.add(entry.clone()));
        assert!(!vault.add(entry.clone()));
        vault.save().unwrap();
        assert!(sibling(&path, "bck").exists());

        let reopened = Vault::open(&path, "pw").unwrap();
        assert_eq!(reopened.entries, std::slice::from_ref(&entry));
        assert!(matches!(Vault::open(&path, "nope"), Err(Error::Decryption)));

        // The vault file is a regular andOTP encrypted backup.
        let (entries, _) = backup::import(&fs::read(&path).unwrap(), Some("pw")).unwrap();
        assert_eq!(entries, [entry]);
    }

    #[test]
    fn password_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut vault = Vault::create_with_iterations(&path, "old", 1000).unwrap();
        vault.params.iterations = 1000;
        vault.key = crypto::derive_key("new", &vault.params.salt, 1000);
        vault.save().unwrap();
        assert!(Vault::open(&path, "old").is_err());
        assert!(Vault::open(&path, "new").is_ok());
    }

    #[test]
    fn import_and_reorder() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = Vault::create_with_iterations(dir.path().join(FILE_NAME), "pw", 1000).unwrap();
        let a = Entry::new(OtpType::Totp, "MZXW6", "A", "a").unwrap();
        let b = Entry::new(OtpType::Totp, "MZXW6YQ", "B", "b").unwrap();
        let summary = vault.import([a.clone(), b.clone(), a.clone()]);
        assert_eq!(summary, ImportSummary { added: 2, skipped: 1 });
        vault.move_entry(1, 0);
        assert_eq!(vault.entries, [b, a]);
    }
}
