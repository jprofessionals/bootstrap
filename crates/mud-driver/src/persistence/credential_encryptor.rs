use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{bail, Context, Result};
use base64::Engine;
/// Encrypts and decrypts credentials using AES-256-GCM.
///
/// Encrypted format: base64(12-byte nonce + ciphertext + 16-byte tag)
pub struct CredentialEncryptor {
    cipher: Aes256Gcm,
}

impl CredentialEncryptor {
    /// Create a new encryptor. Key must be exactly 32 bytes.
    pub fn new(key: &[u8]) -> Result<Self> {
        if key.len() != 32 {
            bail!(
                "encryption key must be exactly 32 bytes, got {}",
                key.len()
            );
        }
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|e| anyhow::anyhow!("invalid encryption key: {}", e))?;
        Ok(Self { cipher })
    }

    /// Encrypt a plaintext string. Returns base64-encoded nonce+ciphertext.
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        let mut nonce_bytes = [0u8; 12];
        rand::fill(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| anyhow::anyhow!("encryption failed: {}", e))?;

        // Prepend nonce to ciphertext
        let mut combined = Vec::with_capacity(12 + ciphertext.len());
        combined.extend_from_slice(&nonce_bytes);
        combined.extend_from_slice(&ciphertext);

        Ok(base64::engine::general_purpose::STANDARD.encode(&combined))
    }

    /// Decrypt a base64-encoded nonce+ciphertext string.
    pub fn decrypt(&self, encrypted: &str) -> Result<String> {
        let combined = base64::engine::general_purpose::STANDARD
            .decode(encrypted)
            .context("invalid base64 in encrypted credential")?;

        if combined.len() < 12 {
            bail!("encrypted data too short (need at least 12 bytes for nonce)");
        }

        let (nonce_bytes, ciphertext) = combined.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = self
            .cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("decryption failed: {}", e))?;

        String::from_utf8(plaintext).context("decrypted credential is not valid UTF-8")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        [0x42; 32]
    }

    #[test]
    fn round_trip() {
        let enc = CredentialEncryptor::new(&test_key()).unwrap();
        let plaintext = "super_secret_password_123";
        let encrypted = enc.encrypt(plaintext).unwrap();
        let decrypted = enc.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn different_ciphertexts_for_same_plaintext() {
        let enc = CredentialEncryptor::new(&test_key()).unwrap();
        let a = enc.encrypt("hello").unwrap();
        let b = enc.encrypt("hello").unwrap();
        assert_ne!(a, b); // random nonce ensures different output
    }

    #[test]
    fn wrong_key_fails() {
        let enc1 = CredentialEncryptor::new(&[0x42; 32]).unwrap();
        let enc2 = CredentialEncryptor::new(&[0x43; 32]).unwrap();
        let encrypted = enc1.encrypt("secret").unwrap();
        assert!(enc2.decrypt(&encrypted).is_err());
    }

    #[test]
    fn reject_short_key() {
        assert!(CredentialEncryptor::new(&[0; 31]).is_err());
    }

    #[test]
    fn reject_long_key() {
        assert!(CredentialEncryptor::new(&[0; 33]).is_err());
    }

    #[test]
    fn empty_plaintext() {
        let enc = CredentialEncryptor::new(&test_key()).unwrap();
        let encrypted = enc.encrypt("").unwrap();
        let decrypted = enc.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn unicode_plaintext() {
        let enc = CredentialEncryptor::new(&test_key()).unwrap();
        let encrypted = enc.encrypt("pässwörd_日本語").unwrap();
        let decrypted = enc.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, "pässwörd_日本語");
    }
}
