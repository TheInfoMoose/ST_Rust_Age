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
use std::sync::{Mutex, MutexGuard};
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

/// Process-wide lock serializing every read-modify-write cycle on the shared vault file.
static VAULT_LOCK: Mutex<()> = Mutex::new(());

fn lock_vault() -> MutexGuard<'static, ()> {
    VAULT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Abstract storage for secrets (like SSH passphrases or profile secrets)
/// backed by the native OS keyring with an authenticated AES-256-GCM encrypted vault fallback.
pub struct SecureStorage {
    service_name: String,
    vault_dir: Option<PathBuf>,
    use_keyring: bool,
}

impl SecureStorage {
    /// Create a new SecureStorage instance for a specific service namespace.
    /// E.g. "com.simplytransfer.app"
    pub fn new(service_name: &str) -> Self {
        Self {
            service_name: service_name.to_string(),
            vault_dir: None,
            use_keyring: true,
        }
    }

    /// No OS keyring; vault lives in `vault_dir`. For tests and headless use.
    pub fn isolated(service_name: &str, vault_dir: impl Into<PathBuf>) -> Self {
        Self {
            service_name: service_name.to_string(),
            vault_dir: Some(vault_dir.into()),
            use_keyring: false,
        }
    }

    fn get_entry(&self, account: &str) -> Result<Entry, CryptoError> {
        Entry::new(&self.service_name, account).map_err(CryptoError::KeyringError)
    }

    fn vault_dir(&self) -> Result<PathBuf, CryptoError> {
        let dir = if let Some(ref custom_dir) = self.vault_dir {
            std::fs::create_dir_all(custom_dir).map_err(|e| CryptoError::Other(e.to_string()))?;
            custom_dir.clone()
        } else {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .map_err(|_| {
                    CryptoError::Other("HOME or USERPROFILE environment variable not set".into())
                })?;
            let primary_dir = PathBuf::from(&home).join(".simply-transfer");
            std::fs::create_dir_all(&primary_dir).map_err(|e| CryptoError::Other(e.to_string()))?;
            primary_dir
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        Ok(dir)
    }

    fn file_stem(&self) -> String {
        if self.service_name == "com.simplytransfer.app" {
            "vault".to_string()
        } else {
            let sanitized: String = self
                .service_name
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect();
            format!("vault_{}", sanitized)
        }
    }

    fn get_vault_path(&self) -> Result<PathBuf, CryptoError> {
        let dir = self.vault_dir()?;
        let stem = self.file_stem();
        Ok(dir.join(format!("{}.enc", stem)))
    }

    fn key_path(&self) -> Result<PathBuf, CryptoError> {
        let dir = self.vault_dir()?;
        let stem = self.file_stem();
        Ok(dir.join(format!("{}.key", stem)))
    }

    /// v1.0 key; used only to migrate
    fn legacy_vault_key(&self) -> [u8; 32] {
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

    fn vault_key(&self) -> Result<[u8; 32], CryptoError> {
        let path = self.key_path()?;
        loop {
            if path.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let meta =
                        std::fs::metadata(&path).map_err(|e| CryptoError::Other(e.to_string()))?;
                    if meta.permissions().mode() & 0o077 != 0 {
                        return Err(CryptoError::Other(
                            "vault key file permissions too open".to_string(),
                        ));
                    }
                }
                let bytes = std::fs::read(&path).map_err(|e| CryptoError::Other(e.to_string()))?;
                if bytes.len() != 32 {
                    return Err(CryptoError::Other("vault key file corrupt".to_string()));
                }
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                if self.use_keyring
                    && let Ok(entry) = self.get_entry("__vault_key__")
                {
                    let hex_key = hex::encode(key);
                    let needs_set = match entry.get_password() {
                        Ok(existing) => existing != hex_key,
                        Err(_) => true,
                    };
                    if needs_set {
                        let _ = entry.set_password(&hex_key);
                    }
                }
                return Ok(key);
            }

            let mut from_keyring: Option<[u8; 32]> = None;
            if self.use_keyring
                && let Ok(entry) = self.get_entry("__vault_key__")
                && let Ok(hex_str) = entry.get_password()
                && hex_str.len() == 64
                && let Ok(decoded) = hex::decode(&hex_str)
                && decoded.len() == 32
            {
                let mut k = [0u8; 32];
                k.copy_from_slice(&decoded);
                from_keyring = Some(k);
            }

            let (key, came_from_keyring) = match from_keyring {
                Some(k) => (k, true),
                None => (rand::random::<[u8; 32]>(), false),
            };

            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            match opts.open(&path) {
                Ok(mut file) => {
                    use std::io::Write;
                    file.write_all(&key)
                        .map_err(|e| CryptoError::Other(e.to_string()))?;
                    if self.use_keyring
                        && !came_from_keyring
                        && let Ok(entry) = self.get_entry("__vault_key__")
                    {
                        let _ = entry.set_password(&hex::encode(key));
                    }
                    return Ok(key);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    continue;
                }
                Err(e) => return Err(CryptoError::Other(e.to_string())),
            }
        }
    }

    fn load_encrypted_vault(&self) -> Result<HashMap<String, String>, CryptoError> {
        let path = self.get_vault_path()?;
        if !path.exists() {
            return Ok(HashMap::new());
        }
        let data = std::fs::read(&path)
            .map_err(|e| CryptoError::Other(format!("vault unreadable: {}", e)))?;
        if data.len() < 12 {
            return Err(CryptoError::Other(
                "vault unreadable: ciphertext too short".to_string(),
            ));
        }
        let nonce = Nonce::from_slice(&data[0..12]);
        let ciphertext = &data[12..];

        let primary_key = self.vault_key()?;
        let primary_cipher = Aes256Gcm::new_from_slice(&primary_key).expect("32 bytes key");
        if let Ok(mut decrypted_bytes) = primary_cipher.decrypt(nonce, ciphertext) {
            let json_res = String::from_utf8(decrypted_bytes.clone());
            decrypted_bytes.zeroize();
            let json_str = json_res.map_err(|e| {
                CryptoError::Other(format!("vault unreadable: invalid utf-8: {}", e))
            })?;
            let map = serde_json::from_str::<HashMap<String, String>>(&json_str).map_err(|e| {
                CryptoError::Other(format!("vault unreadable: invalid json: {}", e))
            })?;
            return Ok(map);
        }

        let legacy_key = self.legacy_vault_key();
        let legacy_cipher = Aes256Gcm::new_from_slice(&legacy_key).expect("32 bytes key");
        if let Ok(mut decrypted_bytes) = legacy_cipher.decrypt(nonce, ciphertext) {
            let json_res = String::from_utf8(decrypted_bytes.clone());
            decrypted_bytes.zeroize();
            let json_str = json_res.map_err(|e| {
                CryptoError::Other(format!("vault unreadable: invalid utf-8: {}", e))
            })?;
            let map = serde_json::from_str::<HashMap<String, String>>(&json_str).map_err(|e| {
                CryptoError::Other(format!("vault unreadable: invalid json: {}", e))
            })?;
            self.save_encrypted_vault(&map)?;
            return Ok(map);
        }

        Err(CryptoError::Other(
            "vault unreadable: decryption failed".to_string(),
        ))
    }

    fn save_encrypted_vault(&self, map: &HashMap<String, String>) -> Result<(), CryptoError> {
        let path = self.get_vault_path()?;
        let json_bytes = serde_json::to_vec(map).map_err(|e| CryptoError::Other(e.to_string()))?;
        let key = self.vault_key()?;
        let cipher = Aes256Gcm::new_from_slice(&key).expect("32 bytes key");
        let nonce_bytes: [u8; 12] = rand::random();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, json_bytes.as_ref())
            .map_err(|e| CryptoError::Other(format!("AES-GCM encryption error: {}", e)))?;

        let mut payload = Vec::with_capacity(12 + ciphertext.len());
        payload.extend_from_slice(&nonce_bytes);
        payload.extend_from_slice(&ciphertext);

        // Write to a temp file in the same directory, then atomically rename into place
        // so readers never observe a partially written vault.
        let tmp_path = path.with_extension(format!("enc.tmp.{}", std::process::id()));
        std::fs::write(&tmp_path, &payload).map_err(|e| CryptoError::Other(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600));
        }
        if let Err(e) = std::fs::rename(&tmp_path, &path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(CryptoError::Other(e.to_string()));
        }
        Ok(())
    }

    /// Store a sensitive string securely. Synchronizes with both the OS keyring and the AES-256-GCM encrypted vault fallback
    /// so transient D-Bus Secret Service sessions (e.g. on Arch/CachyOS Linux) never lose keys across threads or restarts.
    pub fn set_secret(&self, account: &str, mut secret: String) -> Result<(), CryptoError> {
        let _guard = lock_vault();
        if self.use_keyring
            && let Ok(entry) = self.get_entry(account)
        {
            let _ = entry.set_password(&secret);
        }

        let mut vault = match self.load_encrypted_vault() {
            Ok(v) => v,
            Err(e) => {
                secret.zeroize();
                return Err(e);
            }
        };
        vault.insert(account.to_string(), secret.clone());
        let res = self.save_encrypted_vault(&vault);
        secret.zeroize();
        res
    }

    /// Retrieve a sensitive string. The caller is responsible for zeroizing it after use.
    pub fn get_secret(&self, account: &str) -> Result<String, CryptoError> {
        let _guard = lock_vault();
        if self.use_keyring
            && let Ok(entry) = self.get_entry(account)
            && let Ok(pwd) = entry.get_password()
            && !pwd.trim().is_empty()
        {
            return Ok(pwd);
        }
        let vault = self.load_encrypted_vault()?;
        if let Some(secret) = vault.get(account)
            && !secret.trim().is_empty()
        {
            return Ok(secret.clone());
        }
        Err(CryptoError::NotFound)
    }

    /// Delete a secret securely.
    pub fn delete_secret(&self, account: &str) -> Result<(), CryptoError> {
        let _guard = lock_vault();
        if self.use_keyring
            && let Ok(entry) = self.get_entry(account)
        {
            let _ = entry.delete_credential();
        }
        let mut vault = self.load_encrypted_vault()?;
        if vault.remove(account).is_some() {
            self.save_encrypted_vault(&vault)?;
        }
        Ok(())
    }

    /// Purge all stored secrets that are not referenced in `active_accounts`.
    /// Returns the number of orphaned keys purged.
    pub fn purge_orphaned_keys(&self, active_accounts: &[String]) -> Result<usize, CryptoError> {
        let _guard = lock_vault();
        let mut vault = self.load_encrypted_vault()?;
        let mut to_remove = Vec::new();
        for key in vault.keys() {
            let is_managed_key = key.starts_with("key_")
                || key.starts_with("AAAAC3NzaC1lZDI1NTE5")
                || key.starts_with("ssh-ed25519")
                || key.starts_with("wg_");
            if is_managed_key && !active_accounts.iter().any(|a| a == key) {
                to_remove.push(key.clone());
            }
        }
        let purged_count = to_remove.len();
        for key in &to_remove {
            if self.use_keyring
                && let Ok(entry) = self.get_entry(key)
            {
                let _ = entry.delete_credential();
            }
            vault.remove(key);
        }
        if purged_count > 0 {
            self.save_encrypted_vault(&vault)?;
        }
        Ok(purged_count)
    }

    /// Encrypts a sensitive value at rest using AES-256-GCM and returns a base64-encoded nonce+ciphertext.
    pub fn encrypt_value(&self, plaintext: &str) -> Result<String, CryptoError> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let key = self.vault_key()?;
        let cipher = Aes256Gcm::new_from_slice(&key).expect("32 bytes key");
        let nonce_bytes: [u8; 12] = rand::random();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| CryptoError::Other(format!("AES-GCM encryption error: {}", e)))?;

        let mut payload = Vec::with_capacity(12 + ciphertext.len());
        payload.extend_from_slice(&nonce_bytes);
        payload.extend_from_slice(&ciphertext);
        Ok(STANDARD.encode(&payload))
    }

    /// Decrypts a base64-encoded AES-256-GCM nonce+ciphertext produced by `encrypt_value`.
    pub fn decrypt_value(&self, encoded: &str) -> Result<String, CryptoError> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let data = STANDARD
            .decode(encoded)
            .map_err(|e| CryptoError::Other(format!("Base64 decode error: {}", e)))?;
        if data.len() < 12 {
            return Err(CryptoError::Other("Ciphertext too short".to_string()));
        }
        let nonce = Nonce::from_slice(&data[0..12]);
        let ciphertext = &data[12..];

        let primary_key = self.vault_key()?;
        let primary_cipher = Aes256Gcm::new_from_slice(&primary_key).expect("32 bytes key");
        if let Ok(decrypted) = primary_cipher.decrypt(nonce, ciphertext) {
            return String::from_utf8(decrypted).map_err(|e| CryptoError::Other(e.to_string()));
        }

        let legacy_key = self.legacy_vault_key();
        let legacy_cipher = Aes256Gcm::new_from_slice(&legacy_key).expect("32 bytes key");
        let decrypted = legacy_cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| CryptoError::Other(format!("AES-GCM decryption error: {}", e)))?;
        String::from_utf8(decrypted).map_err(|e| CryptoError::Other(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_encrypt_bytes(storage: &SecureStorage, plaintext: &[u8]) -> Vec<u8> {
        let key = storage.legacy_vault_key();
        let cipher = Aes256Gcm::new_from_slice(&key).expect("32 bytes key");
        let nonce_bytes: [u8; 12] = rand::random();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher.encrypt(nonce, plaintext).unwrap();
        let mut payload = Vec::with_capacity(12 + ciphertext.len());
        payload.extend_from_slice(&nonce_bytes);
        payload.extend_from_slice(&ciphertext);
        payload
    }

    fn write_legacy_vault(storage: &SecureStorage, map: &HashMap<String, String>) -> Vec<u8> {
        let json_bytes = serde_json::to_vec(map).unwrap();
        let payload = legacy_encrypt_bytes(storage, &json_bytes);
        let path = storage.get_vault_path().unwrap();
        std::fs::write(&path, &payload).unwrap();
        payload
    }

    #[test]
    fn test_secure_storage() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = SecureStorage::isolated("test.simplytransfer.crypto", tmp.path());
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

        // Test purge_orphaned_keys
        storage
            .set_secret("key_active_1", "secret_active".to_string())
            .unwrap();
        storage
            .set_secret("key_orphan_1", "secret_orphan".to_string())
            .unwrap();
        let purged = storage
            .purge_orphaned_keys(&["key_active_1".to_string()])
            .unwrap();
        assert!(purged >= 1);
        assert!(storage.get_secret("key_orphan_1").is_err());
        assert_eq!(storage.get_secret("key_active_1").unwrap(), "secret_active");
        storage.delete_secret("key_active_1").unwrap();
    }

    #[test]
    fn isolated_storage_writes_only_inside_vault_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = SecureStorage::isolated("test.simplytransfer.crypto", tmp.path());
        storage
            .set_secret("acct", "secret_value".to_string())
            .unwrap();

        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(entries.len(), 2);
        let mut exts: Vec<_> = entries
            .iter()
            .filter_map(|p| p.extension().and_then(|s| s.to_str()))
            .collect();
        exts.sort();
        assert_eq!(exts, vec!["enc", "key"]);
    }

    #[test]
    fn legacy_vault_migrates_on_first_read() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = SecureStorage::isolated("test.simplytransfer.crypto", tmp.path());
        let mut map = HashMap::new();
        map.insert("acct".to_string(), "legacy_secret".to_string());
        let old_bytes = write_legacy_vault(&storage, &map);

        let val = storage.get_secret("acct").unwrap();
        assert_eq!(val, "legacy_secret");

        let vault_path = storage.get_vault_path().unwrap();
        let new_bytes = std::fs::read(&vault_path).unwrap();
        assert_ne!(old_bytes, new_bytes);

        let legacy_key = storage.legacy_vault_key();
        let legacy_cipher = Aes256Gcm::new_from_slice(&legacy_key).unwrap();
        let nonce = Nonce::from_slice(&new_bytes[0..12]);
        assert!(legacy_cipher.decrypt(nonce, &new_bytes[12..]).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn key_file_has_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let storage = SecureStorage::isolated("test.simplytransfer.crypto", tmp.path());
        storage.set_secret("acct", "val".to_string()).unwrap();
        let key_path = storage.key_path().unwrap();
        let mode = std::fs::metadata(&key_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    #[cfg(unix)]
    fn open_key_file_permissions_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let storage = SecureStorage::isolated("test.simplytransfer.crypto", tmp.path());
        storage.set_secret("acct", "val".to_string()).unwrap();
        let key_path = storage.key_path().unwrap();
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(storage.get_secret("acct").is_err());
    }

    #[test]
    fn corrupt_vault_errors_without_overwriting() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = SecureStorage::isolated("test.simplytransfer.crypto", tmp.path());
        let vault_path = storage.get_vault_path().unwrap();
        let garbage: [u8; 40] = rand::random();
        std::fs::write(&vault_path, garbage).unwrap();

        match storage.get_secret("acct") {
            Err(CryptoError::Other(_)) => {}
            other => panic!("expected Err(CryptoError::Other(_)), got {:?}", other),
        }
        let after = std::fs::read(&vault_path).unwrap();
        assert_eq!(after.as_slice(), &garbage);
    }

    #[test]
    fn decrypt_value_accepts_legacy_key() {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let tmp = tempfile::tempdir().unwrap();
        let storage = SecureStorage::isolated("test.simplytransfer.crypto", tmp.path());
        let payload = legacy_encrypt_bytes(&storage, b"legacy_value_payload");
        let encoded = STANDARD.encode(&payload);
        let decrypted = storage.decrypt_value(&encoded).unwrap();
        assert_eq!(decrypted, "legacy_value_payload");
    }
}
pub mod keys;
pub use keys::{VpnConfig, WireGuardConf};
