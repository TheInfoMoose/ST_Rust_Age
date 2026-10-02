import re

with open("simply-transfer-core/src/ssh2_client.rs", "r") as f:
    content = f.read()

replacement = """
    fn authenticate_publickey(
        &mut self,
        username: &str,
        private_key_path: &str,
        passphrase: Option<&str>,
    ) -> Result<(), SshError> {
        let sess = self
            .session
            .as_mut()
            .ok_or_else(|| SshError::ConnectionFailed("Not connected".to_string()))?;

        let priv_path = std::path::Path::new(private_key_path);
        let pub_path = priv_path.with_extension("pub");
        
        // Dynamically ensure the public key file exists and matches the private key perfectly
        if let Ok(priv_pem) = std::fs::read_to_string(priv_path) {
            if let Ok(private_key) = ssh_key::PrivateKey::from_openssh(&priv_pem) {
                let public_key = private_key.public_key();
                if let Ok(public_key_pem) = public_key.to_openssh() {
                    // Write it in OpenSSH format: "ssh-ed25519 <base64> simply-transfer"
                    let _ = std::fs::write(&pub_path, format!("{} simply-transfer", public_key_pem.trim()));
                }
            }
        }

        sess.userauth_pubkey_file(
            username,
            Some(&pub_path),
            priv_path,
            passphrase,
        )
        .map_err(|e| SshError::AuthenticationFailed(e.to_string()))?;

        Ok(())
    }
"""

# Find the old authenticate_publickey block
old_block_pattern = re.compile(
    r'fn authenticate_publickey\(.*?\) -> Result<\(\), SshError> \{.*?Ok\(\(\)\)\n\s*\}', 
    re.DOTALL
)

content = old_block_pattern.sub(replacement.strip(), content, count=1)

with open("simply-transfer-core/src/ssh2_client.rs", "w") as f:
    f.write(content)

