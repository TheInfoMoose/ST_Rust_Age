use crate::ssh::{SshClient, SshError};
use russh::{client::Config, client::Handle};
use russh_sftp::client::SftpSession;
use std::path::Path;
use tokio::fs;
use std::sync::Arc;
use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub struct RusshClient {
    handle: Option<Handle<ClientHandler>>,
    sftp_client: Option<SftpSession>,
}

impl Default for RusshClient {
    fn default() -> Self {
        Self {
            handle: None,
            sftp_client: None,
        }
    }
}

struct ClientHandler;

#[async_trait]
impl russh::client::Handler for ClientHandler {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        _server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true) // Accept all keys for now
    }
}

impl RusshClient {
    pub fn new() -> Self {
        Self::default()
    }

    async fn establish_connection(&mut self, host: &str, port: u16) -> Result<(), SshError> {
        let config = Config {
            inactivity_timeout: Some(std::time::Duration::from_secs(30)),
            ..Default::default()
        };
        let config = Arc::new(config);
        
        let client = russh::client::connect(config, (host, port), ClientHandler)
            .await
            .map_err(|e| SshError::ConnectionFailed(e.to_string()))?;
        
        self.handle = Some(client);
        Ok(())
    }
}

#[async_trait]
impl SshClient for RusshClient {
    async fn connect(&mut self, host: &str, port: u16) -> Result<(), SshError> {
        if self.handle.is_some() {
            return Ok(());
        }
        
        self.establish_connection(host, port).await
    }

    async fn authenticate_publickey(
        &mut self,
        username: &str,
        private_key_pem: &str,
        _passphrase: Option<&str>,
    ) -> Result<(), SshError> {
        let handle = self.handle.as_mut().ok_or(SshError::ConnectionFailed("Not connected".to_string()))?;
        
        let key_pair = russh_keys::decode_secret_key(private_key_pem, None)
            .map_err(|e| SshError::AuthenticationFailed(e.to_string()))?;

        let auth_res = handle.authenticate_publickey(username, Arc::new(key_pair))
            .await
            .map_err(|e| SshError::AuthenticationFailed(e.to_string()))?;

        if !auth_res {
            return Err(SshError::AuthenticationFailed("Key rejected by server".to_string()));
        }

        let mut channel = handle.channel_open_session().await
            .map_err(|e| SshError::SftpError(e.to_string()))?;
        channel.request_subsystem(true, "sftp").await
            .map_err(|e| SshError::SftpError(e.to_string()))?;
            
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| SshError::SftpError(e.to_string()))?;
        
        self.sftp_client = Some(sftp);
        Ok(())
    }

    async fn upload_file(
        &self,
        local_path: &Path,
        remote_path: &str,
        progress_callback: Option<Box<dyn Fn(u64) + Send>>,
    ) -> Result<(), SshError> {
        let sftp = self.sftp_client.as_ref().ok_or(SshError::SftpError("Not connected to SFTP".to_string()))?;
        
        let mut local_file = fs::File::open(local_path)
            .await
            .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;

        let metadata = local_file.metadata().await
            .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;
        let _total_size = metadata.len();

        let mut sftp_file = sftp.create(remote_path).await
            .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;

        const BUFFER_SIZE: usize = 64 * 1024; // 64KB buffer
        let mut buffer = vec![0u8; BUFFER_SIZE];
        let mut uploaded_bytes = 0u64;

        loop {
            let bytes_read = local_file.read(&mut buffer).await
                .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;
                
            if bytes_read == 0 {
                break;
            }

            sftp_file.write_all(&buffer[..bytes_read]).await
                .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;

            uploaded_bytes += bytes_read as u64;
            
            if let Some(callback) = &progress_callback {
                callback(uploaded_bytes);
            }
        }

        Ok(())
    }

    async fn execute_command(&self, command: &str) -> Result<String, SshError> {
        let handle = self.handle.as_ref().ok_or(SshError::ConnectionFailed("Not connected".to_string()))?;
        
        let mut channel = handle.channel_open_session()
            .await
            .map_err(|e| SshError::CommandExecutionFailed(e.to_string()))?;

        channel.exec(true, command)
            .await
            .map_err(|e| SshError::CommandExecutionFailed(e.to_string()))?;

        let mut output = String::new();
        while let Some(msg) = channel.wait().await {
            match msg {
                russh::ChannelMsg::Data { data } => {
                    output.push_str(&String::from_utf8_lossy(&data));
                }
                _ => {}
            }
        }

        Ok(output)
    }
}
