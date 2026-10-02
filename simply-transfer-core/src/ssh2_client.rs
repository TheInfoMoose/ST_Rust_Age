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
        private_key_path: &str,
        passphrase: Option<&str>,
    ) -> Result<(), SshError> {
        let sess = self
            .session
            .as_mut()
            .ok_or_else(|| SshError::ConnectionFailed("Not connected".to_string()))?;

        let priv_path = std::path::Path::new(private_key_path);

        // Read the private key directly from disk
        let priv_pem = std::fs::read_to_string(priv_path)
            .map_err(|e| SshError::AuthenticationFailed(format!("Failed to read private key: {}", e)))?;
            
        // Derive the public key natively in memory
        let private_key = ssh_key::PrivateKey::from_openssh(&priv_pem)
            .map_err(|e| SshError::AuthenticationFailed(format!("Invalid private key format: {}", e)))?;
            
        let public_key = private_key.public_key();
        let public_key_pem = public_key.to_openssh()
            .map_err(|e| SshError::AuthenticationFailed(format!("Failed to derive public key: {}", e)))?;

        // Authenticate purely via memory, bypassing libssh2 file path issues on Windows
        sess.userauth_pubkey_memory(
            username,
            Some(&format!("{} simply-transfer", public_key_pem.trim())),
            &priv_pem,
            passphrase,
        ).map_err(|e| SshError::AuthenticationFailed(e.to_string()))?;

        Ok(())
    }

    fn upload_file(
        &self,
        local_path: &Path,
        remote_path: &str,
        progress_callback: Option<Box<dyn Fn(u64) + Send>>,
    ) -> Result<(), SshError> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| SshError::ConnectionFailed("Not connected".to_string()))?;

        let mut final_remote_path = remote_path.to_string();
        // Create remote parent directory
        if let Some(parent) = Path::new(remote_path).parent()
            && let Some(parent_str) = parent.to_str()
            && !parent_str.is_empty()
        {
            let is_windows = self.execute_command("cmd.exe /c echo Windows")
                .map(|out| out.trim() == "Windows")
                .unwrap_or(false);

            if is_windows {
                final_remote_path = final_remote_path.replace("/", "\\");
            }

            let mkdir_cmd = if is_windows {
                format!("powershell -NoProfile -Command \"New-Item -ItemType Directory -Force -Path '{}'\"", parent_str.replace("/", "\\"))
            } else {
                format!("mkdir -p '{}'", parent_str.replace("'", "'\\''"))
            };
            
            let _ = self.execute_command(&mkdir_cmd);
        }

        let mut local_file =
            File::open(local_path).map_err(|e| SshError::SftpError(e.to_string()))?;
        let metadata = local_file
            .metadata()
            .map_err(|e| SshError::SftpError(e.to_string()))?;

        // 0-byte files cause libssh2 scp_send to fail or hang on some OpenSSH versions.
        if metadata.len() == 0 {
            // Touch the file on the remote side instead of scp_send
            let touch_cmd = if final_remote_path.contains('\\') {
                format!("powershell -NoProfile -Command \"New-Item -ItemType File -Force -Path '{}'\"", final_remote_path)
            } else {
                format!("touch '{}'", final_remote_path.replace("'", "'\\''"))
            };
            let _ = self.execute_command(&touch_cmd);
            return Ok(());
        }

        let sftp = session
            .sftp()
            .map_err(|e| SshError::SftpError(format!("Failed to initialize SFTP: {}", e)))?;

        let mut remote_file = sftp
            .create(Path::new(&final_remote_path))
            .map_err(|e| SshError::SftpError(format!("Failed to open remote file via SFTP: {}", e)))?;

        let mut buffer = [0u8; 65536];
        let mut total_written: u64 = 0;
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

            total_written += bytes_read as u64;
            if let Some(ref cb) = progress_callback {
                cb(total_written);
            }
        }
        
        // Remote file is closed automatically when it goes out of scope

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

        // Drain stderr to prevent command from blocking if stderr fills up
        let mut err = String::new();
        channel.stderr().read_to_string(&mut err).ok();

        channel
            .wait_eof()
            .map_err(|e| SshError::CommandFailed(e.to_string()))?;

        channel
            .close()
            .map_err(|e| SshError::CommandFailed(e.to_string()))?;

        channel
            .wait_close()
            .map_err(|e| SshError::CommandFailed(e.to_string()))?;

        Ok(s)
    }
}
