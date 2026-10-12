//! RFC 4648 Base32, with the lenient decoding real-world OTP secrets need.

use crate::{Error, Result};

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Encodes `data` as padded, upper-case Base32 (the representation andOTP stores in backups).
pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
    for chunk in data.chunks(5) {
        let mut buf = [0u8; 5];
        buf[..chunk.len()].copy_from_slice(chunk);
        let bits = u64::from_be_bytes([0, 0, 0, buf[0], buf[1], buf[2], buf[3], buf[4]]);
        let symbols = match chunk.len() {
            1 => 2,
            2 => 4,
            3 => 5,
            4 => 7,
            _ => 8,
        };
        for i in 0..8 {
            if i < symbols {
                let idx = (bits >> (35 - i * 5)) & 0x1f;
                out.push(ALPHABET[idx as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Encodes `data` as unpadded Base32 (the representation used in `otpauth://` URIs).
pub fn encode_unpadded(data: &[u8]) -> String {
    encode(data).trim_end_matches('=').to_owned()
}

/// Decodes Base32, ignoring case, padding, whitespace and `-` separators.
///
/// Trailing bits that do not form a full byte are discarded, matching the behaviour of the
/// Apache Commons codec used by the Android app.
pub fn decode(input: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 5 / 8);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for c in input.chars() {
        let value = match c.to_ascii_uppercase() {
            c @ 'A'..='Z' => c as u32 - 'A' as u32,
            c @ '2'..='7' => c as u32 - '2' as u32 + 26,
            '=' | ' ' | '-' | '\t' | '\n' | '\r' => continue,
            _ => return Err(Error::InvalidSecret),
        };
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vectors() {
        let cases = [
            ("", ""),
            ("f", "MY======"),
            ("fo", "MZXQ===="),
            ("foo", "MZXW6==="),
            ("foob", "MZXW6YQ="),
            ("fooba", "MZXW6YTB"),
            ("foobar", "MZXW6YTBOI======"),
        ];
        for (plain, encoded) in cases {
            assert_eq!(encode(plain.as_bytes()), encoded);
            assert_eq!(decode(encoded).unwrap(), plain.as_bytes());
        }
    }

    #[test]
    fn lenient_decoding() {
        assert_eq!(decode("mzxw 6ytb-oi").unwrap(), b"foobar");
        assert!(decode("MZXW1").is_err());
    }
}
