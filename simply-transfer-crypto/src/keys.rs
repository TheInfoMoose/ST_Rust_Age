use ed25519_dalek::{SigningKey};
use crate::SecureStorage;
use crate::CryptoError;

pub struct KeyPairManager {
    storage: SecureStorage,
}

impl KeyPairManager {
    pub fn new(service_name: &str) -> Self {
        Self {
            storage: SecureStorage::new(service_name),
        }
    }

    /// Generates a new Ed25519 keypair, securely stores the private key keyed by the public key,
    /// and returns the public key as a hex string.
    pub fn generate_and_store(&self) -> Result<String, CryptoError> {
        let mut csprng = rand::rng();
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        
        let priv_hex = hex::encode(signing_key.to_bytes());
        let pub_hex = hex::encode(verifying_key.to_bytes());
        self.storage.set_secret(&pub_hex, priv_hex)?;

        Ok(pub_hex)
    }

    /// Retrieves the signing key for a given connection_id
    pub fn get_signing_key(&self, connection_id: &str) -> Result<SigningKey, CryptoError> {
        let priv_hex = self.storage.get_secret(connection_id)?;
        let bytes = hex::decode(&priv_hex)
            .map_err(|_| CryptoError::Other("Invalid hex in secure storage".into()))?;
        
        let bytes_array: [u8; 32] = bytes.try_into()
            .map_err(|_| CryptoError::Other("Invalid key length".into()))?;

        Ok(SigningKey::from_bytes(&bytes_array))
    }

    /// Removes a keypair for a given connection_id
    pub fn delete_key(&self, connection_id: &str) -> Result<(), CryptoError> {
        self.storage.delete_secret(connection_id)
    }
}
