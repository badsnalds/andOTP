//! Symmetric encryption as used by andOTP: AES-GCM with a random 96-bit nonce prepended to the
//! ciphertext, and PBKDF2-HMAC-SHA1 for password based keys.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Aes256Gcm, Nonce};
use rand::RngCore;
use zeroize::Zeroizing;

use crate::{Error, Result};

/// Length of the AES-GCM nonce (and of the PBKDF2 salt in backups).
pub const IV_LENGTH: usize = 12;
/// Length of the AES-GCM authentication tag.
pub const TAG_LENGTH: usize = 16;
/// Length of keys derived with PBKDF2 (AES-256).
pub const KEY_LENGTH: usize = 32;

/// Fills a new vector with `len` cryptographically secure random bytes.
pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}

/// Derives an AES-256 key from a password with PBKDF2-HMAC-SHA1.
pub fn derive_key(password: &str, salt: &[u8], iterations: u32) -> Zeroizing<[u8; KEY_LENGTH]> {
    let mut key = Zeroizing::new([0u8; KEY_LENGTH]);
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password.as_bytes(), salt, iterations, key.as_mut());
    key
}

/// Encrypts with an explicit nonce; returns `ciphertext || tag`.
pub fn encrypt_with_nonce(key: &[u8], nonce: &[u8; IV_LENGTH], plaintext: &[u8]) -> Result<Vec<u8>> {
    let nonce = Nonce::from_slice(nonce);
    match key.len() {
        16 => Aes128Gcm::new_from_slice(key).map_err(|_| Error::Encryption)?.encrypt(nonce, plaintext),
        32 => Aes256Gcm::new_from_slice(key).map_err(|_| Error::Encryption)?.encrypt(nonce, plaintext),
        _ => return Err(Error::Encryption),
    }
    .map_err(|_| Error::Encryption)
}

/// Decrypts `ciphertext || tag` with an explicit nonce.
pub fn decrypt_with_nonce(key: &[u8], nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    if nonce.len() != IV_LENGTH {
        return Err(Error::Malformed);
    }
    let nonce = Nonce::from_slice(nonce);
    match key.len() {
        16 => Aes128Gcm::new_from_slice(key).map_err(|_| Error::Decryption)?.decrypt(nonce, ciphertext),
        32 => Aes256Gcm::new_from_slice(key).map_err(|_| Error::Decryption)?.decrypt(nonce, ciphertext),
        _ => return Err(Error::Decryption),
    }
    .map_err(|_| Error::Decryption)
}

/// Encrypts with a fresh random nonce; returns `nonce || ciphertext || tag`.
pub fn encrypt(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
    let mut nonce = [0u8; IV_LENGTH];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ciphertext = encrypt_with_nonce(key, &nonce, plaintext)?;
    let mut out = Vec::with_capacity(IV_LENGTH + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypts `nonce || ciphertext || tag`.
pub fn decrypt(key: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < IV_LENGTH + TAG_LENGTH {
        return Err(Error::Malformed);
    }
    let (nonce, ciphertext) = data.split_at(IV_LENGTH);
    decrypt_with_nonce(key, nonce, ciphertext)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// Test vectors from Go's crypto/cipher/gcm_test.go (also used by the Android test suite).
    #[test]
    fn gcm_vectors() {
        let cases = [
            ("11754cd72aec309bf52f7687212e8957", "3c819d9a9bed087615030b65", "", "250327c674aaf477aef2675748cf6971"),
            ("ca47248ac0b6f8372a97ac43508308ed", "ffd2b598feabc9019262d2be", "", "60d20404af527d248d893ae495707d1a"),
            (
                "7fddb57453c241d03efbed3ac44e371c",
                "ee283a3fc75575e33efd4887",
                "d5de42b461646c255c87bd2962d3b9a2",
                "2ccda4a5415cb91e135c2a0f78c9b2fdb36d1df9b9d5e596f83e8b7f52971cb3",
            ),
            (
                "ab72c77b97cb5fe9a382d9fe81ffdbed",
                "54cc7dc2c37ec006bcc6d1da",
                "007c5e5b3e59df24a7c355584fc1518d",
                "0e1bde206a07a9c2c1b65300f8c649972b4401346697138c7a4891ee59867d0c",
            ),
            (
                "feffe9928665731c6d6a8f9467308308",
                "cafebabefacedbaddecaf888",
                "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b391aafd255",
                "42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e091473f59854d5c2af327cd64a62cf35abd2ba6fab4",
            ),
        ];
        for (key, nonce, plain, cipher) in cases {
            let (key, nonce, plain, cipher) = (hex(key), hex(nonce), hex(plain), hex(cipher));
            let encrypted = encrypt_with_nonce(&key, nonce.as_slice().try_into().unwrap(), &plain).unwrap();
            assert_eq!(encrypted, cipher);
            assert_eq!(decrypt_with_nonce(&key, &nonce, &cipher).unwrap(), plain);
        }
    }

    #[test]
    fn round_trip_and_tamper_detection() {
        let key = random_bytes(32);
        let mut data = encrypt(&key, b"secret").unwrap();
        assert_eq!(decrypt(&key, &data).unwrap(), b"secret");
        let last = data.len() - 1;
        data[last] ^= 1;
        assert!(matches!(decrypt(&key, &data), Err(Error::Decryption)));
        assert!(matches!(decrypt(&key, &[0; 5]), Err(Error::Malformed)));
    }

    #[test]
    fn pbkdf2_vector() {
        // RFC 6070, test case 2 (first 20 bytes of the derived key).
        let key = derive_key("password", b"salt", 2);
        assert_eq!(&key[..20], hex("ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957").as_slice());
    }
}
