pub mod hash;
pub mod token;

use keyring::Entry;
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
/// backed by the native OS keyring.
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

    /// Store a sensitive string securely.
    pub fn set_secret(&self, account: &str, mut secret: String) -> Result<(), CryptoError> {
        let entry = self.get_entry(account)?;
        entry.set_password(&secret)?;
        secret.zeroize();
        Ok(())
    }

    /// Retrieve a sensitive string. The caller is responsible for zeroizing it after use.
    pub fn get_secret(&self, account: &str) -> Result<String, CryptoError> {
        let entry = self.get_entry(account)?;
        match entry.get_password() {
            Ok(pwd) => Ok(pwd),
            Err(keyring::Error::NoEntry) => Err(CryptoError::NotFound),
            Err(e) => Err(CryptoError::KeyringError(e)),
        }
    }

    /// Delete a secret securely.
    pub fn delete_secret(&self, account: &str) -> Result<(), CryptoError> {
        let entry = self.get_entry(account)?;
        match entry.delete_credential() {
            Ok(_) => Ok(()),
            Err(keyring::Error::NoEntry) => Err(CryptoError::NotFound),
            Err(e) => Err(CryptoError::KeyringError(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Keyring tests might fail in headless CI environments without a dbus/secret-service session.
    #[test]
    #[ignore]
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
