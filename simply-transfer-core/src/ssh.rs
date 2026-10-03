use std::path::Path;
use std::sync::Mutex;
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
    #[error("File transfer failed: {0}")]
    FileTransferFailed(String),
    #[error("Command execution failed: {0}")]
    CommandExecutionFailed(String),
}

/// Abstraction for an SSH/SFTP client to allow swapping out implementations
/// and ease of testing.
#[async_trait::async_trait]
pub trait SshClient: Send + Sync {
    /// Connect to the remote host.
    async fn connect(&mut self, host: &str, port: u16) -> Result<(), SshError>;

    /// Authenticate using an Ed25519/RSA private key string.
    async fn authenticate_publickey(
        &mut self,
        username: &str,
        private_key_pem: &str,
        passphrase: Option<&str>,
    ) -> Result<(), SshError>;

    /// Open an SFTP session to upload a file, optionally reporting progress in bytes.
    async fn upload_file(
        &self,
        local_path: &Path,
        remote_path: &str,
        progress_callback: Option<Box<dyn Fn(u64) + Send>>,
    ) -> Result<(), SshError>;

    /// Execute a remote command and return stdout.
    async fn execute_command(&self, command: &str) -> Result<String, SshError>;
}

/// A mocked SSH client for use when testing the phased engine without a live server.
pub struct MockSshClient {
    pub is_connected: bool,
    pub is_authenticated: bool,
    pub uploaded_paths: Mutex<Vec<String>>,
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
            uploaded_paths: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl SshClient for MockSshClient {
    async fn connect(&mut self, _host: &str, _port: u16) -> Result<(), SshError> {
        self.is_connected = true;
        Ok(())
    }

    async fn authenticate_publickey(
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

    async fn upload_file(
        &self,
        local_path: &Path,
        remote_path: &str,
        _progress_callback: Option<Box<dyn Fn(u64) + Send>>,
    ) -> Result<(), SshError> {
        let dest = Path::new(remote_path);
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::copy(local_path, dest) {
            return Err(SshError::SftpError(format!("Failed to copy file: {}", e)));
        }
        if let Ok(mut paths) = self.uploaded_paths.lock() {
            paths.push(remote_path.to_string());
        }
        Ok(())
    }

    async fn execute_command(&self, command: &str) -> Result<String, SshError> {
        // Mock responding to a sha256sum validation
        if command.contains("sha256sum") || command.contains("Get-FileHash") {
            Ok("mock_hash  filename".to_string())
        } else if command.contains("df -B1") {
            Ok("Filesystem     1B-blocks      Used Available Use% Mounted on\n/dev/sda1      100000000 100000000 999999999   1% /".to_string())
        } else if command.contains("Get-WmiObject")
            || command.contains("powershell.exe -NoProfile -Command")
        {
            Ok("999999999999".to_string())
        } else {
            Ok("".to_string())
        }
    }
}
