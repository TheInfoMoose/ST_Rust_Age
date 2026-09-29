use crate::ssh::SshClient;
use crate::ssh::SshError;
use ssh2::Session;
use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;

pub struct Ssh2Client {
    session: Option<Session>,
}

impl Default for Ssh2Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Ssh2Client {
    pub fn new() -> Self {
        Self { session: None }
    }
}

impl SshClient for Ssh2Client {
    fn connect(&mut self, host: &str, port: u16) -> Result<(), SshError> {
        let tcp = TcpStream::connect(format!("{}:{}", host, port))
            .map_err(|e| SshError::ConnectionFailed(e.to_string()))?;

        let mut session = Session::new().map_err(|e| SshError::ConnectionFailed(e.to_string()))?;
        session.set_tcp_stream(tcp);
        session
            .handshake()
            .map_err(|e| SshError::ConnectionFailed(e.to_string()))?;

        self.session = Some(session);
        Ok(())
    }

    fn authenticate_publickey(
        &mut self,
        username: &str,
        private_key_pem: &str,
        passphrase: Option<&str>,
    ) -> Result<(), SshError> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| SshError::ConnectionFailed("Not connected".to_string()))?;

        // ssh2 currently requires a file path for the private key or extracting the key.
        // For demonstration, we assume `private_key_pem` is the path.
        // A complete implementation would write the PEM to a secure temp file or use memory.
        session
            .userauth_pubkey_file(username, None, Path::new(private_key_pem), passphrase)
            .map_err(|e| SshError::AuthenticationFailed(e.to_string()))?;

        if !session.authenticated() {
            return Err(SshError::AuthenticationFailed(
                "Authentication failed".to_string(),
            ));
        }

        Ok(())
    }

    fn upload_file(&self, local_path: &Path, remote_path: &str) -> Result<(), SshError> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| SshError::ConnectionFailed("Not connected".to_string()))?;

        // Create remote parent directory
        if let Some(parent) = Path::new(remote_path).parent() {
            if let Some(parent_str) = parent.to_str() {
                if !parent_str.is_empty() {
                    let mut channel = session
                        .channel_session()
                        .map_err(|e| SshError::SftpError(e.to_string()))?;
                    let mkdir_cmd = format!("mkdir -p '{}'", parent_str.replace("'", "'\\''"));
                    let _ = channel.exec(&mkdir_cmd);
                    let _ = channel.wait_close();
                }
            }
        }

        let mut local_file =
            File::open(local_path).map_err(|e| SshError::SftpError(e.to_string()))?;
        let metadata = local_file
            .metadata()
            .map_err(|e| SshError::SftpError(e.to_string()))?;

        let mut remote_file = session
            .scp_send(Path::new(remote_path), 0o644, metadata.len(), None)
            .map_err(|e| SshError::SftpError(e.to_string()))?;

        let mut buffer = [0u8; 32768];
        loop {
            let bytes_read = local_file
                .read(&mut buffer)
                .map_err(|e| SshError::SftpError(e.to_string()))?;
            if bytes_read == 0 {
                break;
            }
            remote_file
                .write_all(&buffer[..bytes_read])
                .map_err(|e| SshError::SftpError(e.to_string()))?;
        }

        // Close the channel
        remote_file
            .send_eof()
            .map_err(|e| SshError::SftpError(e.to_string()))?;
        remote_file
            .wait_eof()
            .map_err(|e| SshError::SftpError(e.to_string()))?;
        remote_file
            .close()
            .map_err(|e| SshError::SftpError(e.to_string()))?;
        remote_file
            .wait_close()
            .map_err(|e| SshError::SftpError(e.to_string()))?;

        Ok(())
    }

    fn execute_command(&self, command: &str) -> Result<String, SshError> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| SshError::ConnectionFailed("Not connected".to_string()))?;
        let mut channel = session
            .channel_session()
            .map_err(|e| SshError::CommandFailed(e.to_string()))?;

        channel
            .exec(command)
            .map_err(|e| SshError::CommandFailed(e.to_string()))?;

        let mut s = String::new();
        channel
            .read_to_string(&mut s)
            .map_err(|e| SshError::CommandFailed(e.to_string()))?;

        channel
            .wait_close()
            .map_err(|e| SshError::CommandFailed(e.to_string()))?;

        Ok(s)
    }
}
