//! Core of andOTP, rewritten in Rust.
//!
//! This crate contains everything that is not user interface:
//!
//! * [`otp`] – TOTP (RFC 6238), HOTP (RFC 4226), Steam Guard and mOTP token generation.
//! * [`entry`] – the account model, including `otpauth://` URI parsing/serialisation and
//!   the JSON representation used by andOTP backups.
//! * [`backup`] – reading and writing andOTP backups (plain JSON, password protected and the
//!   legacy password-protected format).
//! * [`vault`] – the encrypted on-disk database used by the desktop front-ends.
//! * [`list`] – sorting, searching and tag filtering of entries.
//!
//! All file formats are byte-for-byte compatible with the Android app, so backups can be moved
//! freely between andOTP for Android and this implementation.

pub mod backup;
pub mod base32;
pub mod crypto;
pub mod entry;
mod error;
pub mod list;
pub mod otp;
pub mod thumbnail;
pub mod vault;

pub use entry::{Entry, OtpType, Token};
pub use error::{Error, Result};
pub use otp::HashAlgorithm;
pub use vault::Vault;
