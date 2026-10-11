pub mod hash;
pub mod token;

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use keyring::Entry;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;
use zeroize::Zeroize;

#[derive(Error, Debug)]
pub enum CryptoError {
    #[error("Keyring error: {0}")]
    KeyringError(#[from] keyring::Error),
    #[error("Value not found")]
    NotFound,
    #[error("Other error: {0}")]
    Other(String),
}

/// Abstract storage for secrets (like SSH passphrases or profile secrets)
/// backed by the native OS keyring with an authenticated AES-256-GCM encrypted vault fallback.
pub struct SecureStorage {
    service_name: String,
}

impl SecureStorage {
    /// Create a new SecureStorage instance for a specific service namespace.
    /// E.g. "com.simplytransfer.app"
    pub fn new(service_name: &str) -> Self {
        Self {
            service_name: service_name.to_string(),
        }
    }

    fn get_entry(&self, account: &str) -> Result<Entry, CryptoError> {
        Entry::new(&self.service_name, account).map_err(CryptoError::KeyringError)
    }

    fn get_vault_path() -> PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        let dir = PathBuf::from(home).join(".simply-transfer");
        let _ = std::fs::create_dir_all(&dir);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        dir.join("vault.enc")
    }

    fn derive_vault_key(&self) -> [u8; 32] {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "simply-transfer-user".to_string());
        let mut hasher = Sha256::new();
        hasher.update(b"SimplyTransfer:NIST-SP-800-57:VaultKey:");
        hasher.update(self.service_name.as_bytes());
        hasher.update(b":");
        hasher.update(user.as_bytes());
        let result = hasher.finalize();
        let mut key = [0u8; 32];
        key.copy_from_slice(&result);
        key
    }

    fn load_encrypted_vault(&self) -> HashMap<String, String> {
        let path = Self::get_vault_path();
        if !path.exists() {
            return HashMap::new();
        }
        let Ok(data) = std::fs::read(&path) else {
            return HashMap::new();
        };
        if data.len() < 12 {
            return HashMap::new();
        }
        let key = self.derive_vault_key();
        let cipher = Aes256Gcm::new_from_slice(&key).expect("32 bytes key");
        let nonce = Nonce::from_slice(&data[0..12]);
        let ciphertext = &data[12..];
        if let Ok(mut decrypted_bytes) = cipher.decrypt(nonce, ciphertext)
            && let Ok(json_str) = String::from_utf8(decrypted_bytes.clone())
        {
            decrypted_bytes.zeroize();
            if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&json_str) {
                return map;
            }
        }
        HashMap::new()
    }

    fn save_encrypted_vault(&self, map: &HashMap<String, String>) -> Result<(), CryptoError> {
        let path = Self::get_vault_path();
        let json_bytes = serde_json::to_vec(map).map_err(|e| CryptoError::Other(e.to_string()))?;
        let key = self.derive_vault_key();
        let cipher = Aes256Gcm::new_from_slice(&key).expect("32 bytes key");
        let nonce_bytes: [u8; 12] = rand::random();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, json_bytes.as_ref())
            .map_err(|e| CryptoError::Other(format!("AES-GCM encryption error: {}", e)))?;

        let mut payload = Vec::with_capacity(12 + ciphertext.len());
        payload.extend_from_slice(&nonce_bytes);
        payload.extend_from_slice(&ciphertext);

        std::fs::write(&path, &payload).map_err(|e| CryptoError::Other(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// Store a sensitive string securely. Prioritizes the OS keyring, with an AES-256-GCM encrypted vault fallback.
    pub fn set_secret(&self, account: &str, mut secret: String) -> Result<(), CryptoError> {
        let entry_res = self.get_entry(account);
        let keyring_ok = match entry_res {
            Ok(entry) => entry.set_password(&secret).is_ok(),
            Err(_) => false,
        };

        if !keyring_ok {
            let mut vault = self.load_encrypted_vault();
            vault.insert(account.to_string(), secret.clone());
            self.save_encrypted_vault(&vault)?;
        }
        secret.zeroize();
        Ok(())
    }

    /// Retrieve a sensitive string. The caller is responsible for zeroizing it after use.
    pub fn get_secret(&self, account: &str) -> Result<String, CryptoError> {
        if let Ok(entry) = self.get_entry(account)
            && let Ok(pwd) = entry.get_password()
        {
            return Ok(pwd);
        }
        let vault = self.load_encrypted_vault();
        if let Some(secret) = vault.get(account) {
            return Ok(secret.clone());
        }
        Err(CryptoError::NotFound)
    }

    /// Delete a secret securely.
    pub fn delete_secret(&self, account: &str) -> Result<(), CryptoError> {
        if let Ok(entry) = self.get_entry(account) {
            let _ = entry.delete_credential();
        }
        let mut vault = self.load_encrypted_vault();
        if vault.remove(account).is_some() {
            let _ = self.save_encrypted_vault(&vault);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secure_storage() {
        let storage = SecureStorage::new("test.simplytransfer.crypto");
        let account = "test_user";

        let _ = storage.delete_secret(account);

        assert!(storage.get_secret(account).is_err());

        storage
            .set_secret(account, "super_secret".to_string())
            .unwrap();
        let retrieved = storage.get_secret(account).unwrap();
        assert_eq!(retrieved, "super_secret");

        storage.delete_secret(account).unwrap();
        assert!(storage.get_secret(account).is_err());
    }
}
pub mod keys;
