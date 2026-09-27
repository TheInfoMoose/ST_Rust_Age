use crate::CryptoError;
use crate::SecureStorage;
use rand_core::OsRng;
use ssh_key::PrivateKey;

pub struct KeyPairManager {
    storage: SecureStorage,
}

impl KeyPairManager {
    pub fn new(service_name: &str) -> Self {
        Self {
            storage: SecureStorage::new(service_name),
        }
    }

    /// Generates a new Ed25519 keypair, securely stores the OpenSSH private key PEM keyed by the public key,
    /// and returns the public key as a hex string (or OpenSSH string).
    pub fn generate_and_store(&self) -> Result<String, CryptoError> {
        let mut rng = OsRng;
        let priv_key = PrivateKey::random(&mut rng, ssh_key::Algorithm::Ed25519)
            .map_err(|e| CryptoError::Other(e.to_string()))?;

        let pub_key = priv_key
            .public_key()
            .to_openssh()
            .map_err(|e| CryptoError::Other(e.to_string()))?;
        let priv_pem = priv_key
            .to_openssh(ssh_key::LineEnding::LF)
            .map_err(|e| CryptoError::Other(e.to_string()))?;

        // Use the base64 part of the public key as the identifier to store the private key
        let parts: Vec<&str> = pub_key.split_whitespace().collect();
        let pub_id = if parts.len() >= 2 { parts[1] } else { &pub_key };

        self.storage.set_secret(pub_id, priv_pem.to_string())?;

        Ok(pub_id.to_string())
    }

    /// Retrieves the OpenSSH private key PEM string for a given connection_id (public key base64)
    pub fn get_private_key_pem(&self, connection_id: &str) -> Result<String, CryptoError> {
        self.storage.get_secret(connection_id)
    }

    /// Removes a keypair for a given connection_id
    pub fn delete_key(&self, connection_id: &str) -> Result<(), CryptoError> {
        self.storage.delete_secret(connection_id)
    }
}
