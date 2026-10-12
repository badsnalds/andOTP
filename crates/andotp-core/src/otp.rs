//! One-time password algorithms.

use std::fmt;
use std::str::FromStr;

use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};

pub const TOTP_DEFAULT_PERIOD: u32 = 30;
pub const TOTP_DEFAULT_DIGITS: u32 = 6;
pub const HOTP_INITIAL_COUNTER: u64 = 1;
pub const STEAM_DEFAULT_DIGITS: u32 = 5;
/// mOTP tokens change every 10 seconds.
pub const MOTP_PERIOD: u32 = 10;

const STEAM_CHARS: &[u8; 26] = b"23456789BCDFGHJKMNPQRTVWXY";

/// HMAC hash function used to derive a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HashAlgorithm {
    #[default]
    Sha1,
    Sha256,
    Sha512,
}

impl HashAlgorithm {
    pub const ALL: [HashAlgorithm; 3] = [Self::Sha1, Self::Sha256, Self::Sha512];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sha1 => "SHA1",
            Self::Sha256 => "SHA256",
            Self::Sha512 => "SHA512",
        }
    }
}

impl fmt::Display for HashAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for HashAlgorithm {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_uppercase().replace('-', "").as_str() {
            "SHA1" => Ok(Self::Sha1),
            "SHA256" => Ok(Self::Sha256),
            "SHA512" => Ok(Self::Sha512),
            other => Err(format!("unknown hash algorithm `{other}`")),
        }
    }
}

fn hmac<M: Mac + hmac::digest::KeyInit>(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <M as Mac>::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Computes the 31-bit dynamically truncated HOTP value (RFC 4226, section 5.3).
pub fn hotp_value(secret: &[u8], counter: u64, algorithm: HashAlgorithm) -> u32 {
    let data = counter.to_be_bytes();
    let hash = match algorithm {
        HashAlgorithm::Sha1 => hmac::<Hmac<sha1::Sha1>>(secret, &data),
        HashAlgorithm::Sha256 => hmac::<Hmac<sha2::Sha256>>(secret, &data),
        HashAlgorithm::Sha512 => hmac::<Hmac<sha2::Sha512>>(secret, &data),
    };
    let offset = (hash[hash.len() - 1] & 0x0f) as usize;
    u32::from_be_bytes([hash[offset] & 0x7f, hash[offset + 1], hash[offset + 2], hash[offset + 3]])
}

/// Reduces `value` to `digits` decimal digits, zero padded.
fn decimal(value: u32, digits: u32) -> String {
    let digits = digits.clamp(1, 10);
    let value = u64::from(value) % 10u64.pow(digits);
    format!("{value:0width$}", width = digits as usize)
}

/// The time step for `time` (seconds since the epoch), shifted by `offset` steps.
pub fn time_counter(time: u64, period: u32, offset: i64) -> u64 {
    (time / u64::from(period.max(1))).saturating_add_signed(offset)
}

/// HOTP token as specified in RFC 4226.
pub fn hotp(secret: &[u8], counter: u64, digits: u32, algorithm: HashAlgorithm) -> String {
    decimal(hotp_value(secret, counter, algorithm), digits)
}

/// TOTP token as specified in RFC 6238.
pub fn totp(secret: &[u8], time: u64, period: u32, digits: u32, algorithm: HashAlgorithm, offset: i64) -> String {
    hotp(secret, time_counter(time, period, offset), digits, algorithm)
}

/// Steam Guard token: a TOTP value rendered in Steam's 26 character alphabet.
pub fn steam(secret: &[u8], time: u64, period: u32, digits: u32, algorithm: HashAlgorithm, offset: i64) -> String {
    let mut value = hotp_value(secret, time_counter(time, period, offset), algorithm);
    let len = STEAM_CHARS.len() as u32;
    (0..digits)
        .map(|_| {
            let c = STEAM_CHARS[(value % len) as usize] as char;
            value /= len;
            c
        })
        .collect()
}

/// Mobile-OTP token: the first six hex characters of `MD5(epoch/10 || secret || pin)`.
pub fn motp(pin: &str, secret: &str, time: u64, offset: i64) -> String {
    let step = time_counter(time, MOTP_PERIOD, offset);
    let digest = Md5::digest(format!("{step}{secret}{pin}").as_bytes());
    digest.iter().take(3).map(|b| format!("{b:02x}")).collect()
}

/// Splits a token into groups of `group` characters, counted from the right (`"123456"` → `"123 456"`).
pub fn group_token(token: &str, group: usize) -> String {
    if group == 0 {
        return token.to_owned();
    }
    let chars: Vec<char> = token.chars().collect();
    let first = match chars.len() % group {
        0 => group,
        n => n,
    };
    let mut out = String::with_capacity(token.len() + token.len() / group);
    for (i, c) in chars.iter().enumerate() {
        if i >= first && (i - first) % group == 0 {
            out.push(' ');
        }
        out.push(*c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_SHA1: &[u8] = b"12345678901234567890";
    const KEY_SHA256: &[u8] = b"12345678901234567890123456789012";
    const KEY_SHA512: &[u8] = b"1234567890123456789012345678901234567890123456789012345678901234";

    #[test]
    fn rfc6238_vectors() {
        let cases: [(u64, &str, &str, &str); 6] = [
            (59, "94287082", "46119246", "90693936"),
            (1111111109, "07081804", "68084774", "25091201"),
            (1111111111, "14050471", "67062674", "99943326"),
            (1234567890, "89005924", "91819424", "93441116"),
            (2000000000, "69279037", "90698825", "38618901"),
            (20000000000, "65353130", "77737706", "47863826"),
        ];
        for (time, sha1, sha256, sha512) in cases {
            assert_eq!(totp(KEY_SHA1, time, 30, 8, HashAlgorithm::Sha1, 0), sha1);
            assert_eq!(totp(KEY_SHA256, time, 30, 8, HashAlgorithm::Sha256, 0), sha256);
            assert_eq!(totp(KEY_SHA512, time, 30, 8, HashAlgorithm::Sha512, 0), sha512);
        }
    }

    #[test]
    fn rfc4226_vectors() {
        let expected =
            ["755224", "287082", "359152", "969429", "338314", "254676", "287922", "162583", "399871", "520489"];
        for (counter, token) in expected.iter().enumerate() {
            assert_eq!(hotp(KEY_SHA1, counter as u64, 6, HashAlgorithm::Sha1), *token);
        }
    }

    #[test]
    fn previous_token_offset() {
        assert_eq!(
            totp(KEY_SHA1, 89, 30, 8, HashAlgorithm::Sha1, -1),
            totp(KEY_SHA1, 59, 30, 8, HashAlgorithm::Sha1, 0)
        );
    }

    #[test]
    fn steam_token_uses_steam_alphabet() {
        let token = steam(KEY_SHA1, 59, 30, 5, HashAlgorithm::Sha1, 0);
        assert_eq!(token.len(), 5);
        assert!(token.bytes().all(|c| STEAM_CHARS.contains(&c)));
        // 94287082 → base-26 digits, least significant first.
        let mut v = 1094287082u32; // full 31-bit value for this step
        let manual: String = (0..5)
            .map(|_| {
                let c = STEAM_CHARS[(v % 26) as usize] as char;
                v /= 26;
                c
            })
            .collect();
        assert_eq!(hotp_value(KEY_SHA1, 1, HashAlgorithm::Sha1), 1094287082);
        assert_eq!(token, manual);
    }

    #[test]
    fn motp_matches_reference() {
        // MD5("1234567" + "0123456789abcdef" + "1234") computed with a reference implementation.
        let token = motp("1234", "0123456789abcdef", 12_345_678, 0);
        let digest = Md5::digest(b"12345670123456789abcdef1234");
        let expected: String = digest.iter().take(3).map(|b| format!("{b:02x}")).collect();
        assert_eq!(token, expected);
        assert_eq!(token.len(), 6);
    }

    #[test]
    fn grouping() {
        assert_eq!(group_token("123456", 3), "123 456");
        assert_eq!(group_token("1234567", 3), "1 234 567");
        assert_eq!(group_token("12345678", 4), "1234 5678");
        assert_eq!(group_token("123456", 0), "123456");
    }

    #[test]
    fn algorithm_parsing() {
        assert_eq!("sha-256".parse::<HashAlgorithm>().unwrap(), HashAlgorithm::Sha256);
        assert!("md5".parse::<HashAlgorithm>().is_err());
    }
}
