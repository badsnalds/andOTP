//! Core of andOTP, rewritten in Rust.
//!
//! This crate contains everything that is not user interface:
//!
//! * [`autotype`] – typing tokens into other applications (feature `autotype`).
//! * [`otp`] – TOTP (RFC 6238), HOTP (RFC 4226), Steam Guard and mOTP token generation.
//! * [`entry`] – the account model, including `otpauth://` URI parsing/serialisation and
//!   the JSON representation used by andOTP backups.
//! * [`backup`] – reading and writing andOTP backups (plain JSON, password protected and the
//!   legacy password-protected format).
//! * [`vault`] – the encrypted on-disk database used by the desktop front-ends.
//! * [`keychain`] – optional storage of the vault password in the OS credential store
//!   (feature `keychain`).
//! * [`list`] – sorting, searching and tag filtering of entries.
//!
//! All file formats are byte-for-byte compatible with the Android app, so backups can be moved
//! freely between andOTP for Android and this implementation.

#[cfg(feature = "autotype")]
pub mod autotype;
pub mod backup;
pub mod base32;
pub mod crypto;
pub mod entry;
mod error;
#[cfg(feature = "keychain")]
pub mod keychain;
pub mod list;
pub mod otp;
pub mod thumbnail;
pub mod vault;

pub use entry::{Entry, OtpType, Token};
pub use error::{Error, Result};
pub use otp::HashAlgorithm;
pub use vault::Vault;
