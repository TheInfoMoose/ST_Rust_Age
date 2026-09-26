use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SshError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    #[error("SFTP error: {0}")]
    SftpError(String),
    #[error("Command execution failed: {0}")]
    CommandFailed(String),
}

/// Abstraction for an SSH/SFTP client to allow swapping out implementations
/// and ease of testing.
pub trait SshClient: Send + Sync {
    /// Connect to the remote host.
    fn connect(&mut self, host: &str, port: u16) -> Result<(), SshError>;

    /// Authenticate using an Ed25519/RSA private key string.
    fn authenticate_publickey(
        &mut self,
        username: &str,
        private_key_pem: &str,
        passphrase: Option<&str>,
    ) -> Result<(), SshError>;

    /// Open an SFTP session to upload a file.
    fn upload_file(&self, local_path: &Path, remote_path: &Path) -> Result<(), SshError>;

    /// Execute a remote command and return stdout.
    fn execute_command(&self, command: &str) -> Result<String, SshError>;
}

/// A mocked SSH client for use when testing the phased engine without a live server.
pub struct MockSshClient {
    pub is_connected: bool,
    pub is_authenticated: bool,
}

impl Default for MockSshClient {
    fn default() -> Self {
        Self::new()
    }
}

impl MockSshClient {
    pub fn new() -> Self {
        Self {
            is_connected: false,
            is_authenticated: false,
        }
    }
}

impl SshClient for MockSshClient {
    fn connect(&mut self, _host: &str, _port: u16) -> Result<(), SshError> {
        self.is_connected = true;
        Ok(())
    }

    fn authenticate_publickey(
        &mut self,
        _username: &str,
        _private_key_pem: &str,
        _passphrase: Option<&str>,
    ) -> Result<(), SshError> {
        if self.is_connected {
            self.is_authenticated = true;
            Ok(())
        } else {
            Err(SshError::ConnectionFailed("Not connected".to_string()))
        }
    }

    fn upload_file(&self, _local_path: &Path, _remote_path: &Path) -> Result<(), SshError> {
        Ok(())
    }

    fn execute_command(&self, command: &str) -> Result<String, SshError> {
        // Mock responding to a sha256sum validation
        if command.contains("sha256sum") {
            Ok("mock_hash  filename".to_string())
        } else {
            Ok("".to_string())
        }
    }
}
