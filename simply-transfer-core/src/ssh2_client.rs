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
        session.set_timeout(15000);
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
        let pub_path = priv_path.with_extension("pub");

        // Dynamically ensure the public key file exists and matches the private key perfectly
        if let Ok(priv_pem) = std::fs::read_to_string(priv_path) {
            if let Ok(private_key) = ssh_key::PrivateKey::from_openssh(&priv_pem) {
                let public_key = private_key.public_key();
                if let Ok(public_key_pem) = public_key.to_openssh() {
                    // Write it in OpenSSH format: "ssh-ed25519 <base64> simply-transfer"
                    let _ = std::fs::write(
                        &pub_path,
                        format!("{} simply-transfer", public_key_pem.trim()),
                    );
                }
            }
        }

        sess.userauth_pubkey_file(username, Some(&pub_path), priv_path, passphrase)
            .map_err(|e| SshError::AuthenticationFailed(e.to_string()))?;

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

        // Create remote parent directory
        if let Some(parent) = Path::new(remote_path).parent()
            && let Some(parent_str) = parent.to_str()
            && !parent_str.is_empty()
        {
            let is_windows = self
                .execute_command("cmd.exe /c echo Windows")
                .map(|out| out.trim() == "Windows")
                .unwrap_or(false);

            let mkdir_cmd = if is_windows {
                format!(
                    "powershell -NoProfile -Command \"New-Item -ItemType Directory -Force -Path '{}'\"",
                    parent_str
                )
            } else {
                format!("mkdir -p \"{}\"", parent_str.replace("\"", "\\\""))
            };

            let _ = self.execute_command(&mkdir_cmd);
        }

        let mut local_file =
            File::open(local_path).map_err(|e| SshError::SftpError(e.to_string()))?;
        let metadata = local_file
            .metadata()
            .map_err(|e| SshError::SftpError(e.to_string()))?;

        if metadata.len() == 0 {
            tracing::info!("File is empty, skipping SCP transfer.");
            return Ok(());
        }

        tracing::info!("Setting session to blocking and 30s timeout.");
        session.set_blocking(true);
        session.set_timeout(30000); // 30 seconds

        let mut scp_remote_path = remote_path.to_string();
        scp_remote_path = scp_remote_path.replace("\\", "/");
        if let Some((drive, rest)) = scp_remote_path.split_once(':') {
            if drive.len() == 1 {
                scp_remote_path = format!("/{}:{}", drive.to_uppercase(), rest);
            }
        }

        tracing::info!(
            "Calling scp_send for {} (formatted: {}) with size: {} and mode: {:#o}",
            remote_path,
            scp_remote_path,
            metadata.len(),
            0o644
        );
        let mut remote_file =
            match session.scp_send(Path::new(&scp_remote_path), 0o644, metadata.len(), None) {
                Ok(rf) => rf,
                Err(e) => {
                    tracing::error!("scp_send failed immediately with error: {}", e);
                    return Err(SshError::SftpError(e.to_string()));
                }
            };
        tracing::info!("scp_send channel opened successfully. Beginning chunk transfer...");

        // Use smaller buffer chunks to prevent stalling on large transfers
        let mut buffer = [0u8; 16384]; // Reduced from 65536 to 16KB
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
