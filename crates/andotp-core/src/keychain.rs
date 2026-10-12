//! Remembering vault passwords in the operating system's credential store
//! (macOS Keychain, Windows Credential Manager, Secret Service on Linux/BSD).
//!
//! This is the desktop counterpart of the Android app's KeyStore mode: the vault can be
//! unlocked without typing the password while the user's OS session is unlocked.

use std::path::Path;

use zeroize::Zeroizing;

use crate::{Error, Result};

const SERVICE: &str = "org.shadowice.andOTP";

fn entry(vault: &Path) -> Result<keyring::Entry> {
    // Key the credential by the absolute vault path so several vaults can coexist.
    let account = std::path::absolute(vault).unwrap_or_else(|_| vault.to_path_buf());
    keyring::Entry::new(SERVICE, &account.to_string_lossy()).map_err(|e| Error::Keychain(e.to_string()))
}

/// Stores the password of the vault at `vault`.
pub fn store(vault: &Path, password: &str) -> Result<()> {
    entry(vault)?.set_password(password).map_err(|e| Error::Keychain(e.to_string()))
}

/// Returns the stored password, or `None` if there is none.
pub fn load(vault: &Path) -> Result<Option<Zeroizing<String>>> {
    match entry(vault)?.get_password() {
        Ok(password) => Ok(Some(Zeroizing::new(password))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(Error::Keychain(e.to_string())),
    }
}

/// Removes the stored password (succeeds if there is none).
pub fn delete(vault: &Path) -> Result<()> {
    match entry(vault)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(Error::Keychain(e.to_string())),
    }
}
