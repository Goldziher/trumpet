//! Snapshot encryption using AES-256-GCM.
//!
//! A 256-bit key is stored at `~/.trumpet/state/key`. If it does not exist,
//! one is generated on first use. Each snapshot is encrypted with a fresh
//! random nonce prepended to the ciphertext.

use std::path::Path;

use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, KeyInit, Nonce};

use crate::error::Error;

const NONCE_LEN: usize = 12;

/// Load the encryption key from `key_path`, or generate and persist a new one.
pub async fn load_or_create_key(key_path: &Path) -> Result<Key<Aes256Gcm>, Error> {
    if key_path.exists() {
        let bytes = tokio::fs::read(key_path)
            .await
            .map_err(|e| Error::StateSnapshotFailed {
                reason: format!("reading encryption key: {e}"),
            })?;
        if bytes.len() != 32 {
            return Err(Error::StateSnapshotFailed {
                reason: format!("encryption key must be 32 bytes, got {}", bytes.len()),
            });
        }
        Ok(*Key::<Aes256Gcm>::from_slice(&bytes))
    } else {
        let key = Aes256Gcm::generate_key(OsRng);
        if let Some(parent) = key_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::StateSnapshotFailed {
                    reason: format!("creating key directory: {e}"),
                })?;
        }
        tokio::fs::write(key_path, key.as_slice())
            .await
            .map_err(|e| Error::StateSnapshotFailed {
                reason: format!("writing encryption key: {e}"),
            })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(key_path, std::fs::Permissions::from_mode(0o600))
                .await
                .map_err(|e| Error::StateSnapshotFailed {
                    reason: format!("setting key file permissions: {e}"),
                })?;
        }

        Ok(key)
    }
}

/// Encrypt `plaintext` with AES-256-GCM. Returns `nonce || ciphertext`.
pub fn encrypt(key: &Key<Aes256Gcm>, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
    let cipher = Aes256Gcm::new(key);
    let nonce = Aes256Gcm::generate_nonce(OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| Error::StateSnapshotFailed {
            reason: "encryption failed".to_owned(),
        })?;
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt `data` (expected as `nonce || ciphertext`) with AES-256-GCM.
pub fn decrypt(key: &Key<Aes256Gcm>, data: &[u8]) -> Result<Vec<u8>, Error> {
    if data.len() < NONCE_LEN {
        return Err(Error::StateRestoreFailed {
            reason: "encrypted snapshot too short to contain nonce".to_owned(),
        });
    }
    let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_bytes);
    let cipher = Aes256Gcm::new(key);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| Error::StateRestoreFailed {
            reason: "decryption failed — key mismatch or corrupt snapshot".to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn load_or_create_key_generates_and_persists() {
        let dir = TempDir::new().unwrap();
        let key_path = dir.path().join("key");

        let key1 = load_or_create_key(&key_path).await.unwrap();
        assert!(key_path.exists(), "key file must be created");

        let key2 = load_or_create_key(&key_path).await.unwrap();
        assert_eq!(
            key1, key2,
            "loading the same key must return identical bytes"
        );
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let key = Aes256Gcm::generate_key(OsRng);
        let plaintext = b"hello trumpet state";

        let encrypted = encrypt(&key, plaintext).unwrap();
        assert_ne!(
            encrypted, plaintext,
            "ciphertext must differ from plaintext"
        );

        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext, "decrypted must match original");
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let key1 = Aes256Gcm::generate_key(OsRng);
        let key2 = Aes256Gcm::generate_key(OsRng);

        let encrypted = encrypt(&key1, b"secret").unwrap();
        let result = decrypt(&key2, &encrypted);
        assert!(result.is_err(), "decryption with wrong key must fail");
    }

    #[test]
    fn decrypt_truncated_data_fails() {
        let key = Aes256Gcm::generate_key(OsRng);
        let result = decrypt(&key, &[0u8; 5]);
        assert!(result.is_err(), "data shorter than nonce must fail");
    }
}
