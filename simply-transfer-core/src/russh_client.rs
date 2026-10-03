use crate::ssh::{SshClient, SshError};
use async_trait::async_trait;
use russh::{client::Config, client::Handle};
use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncReadExt;

pub struct RusshClient {
    handle: Option<Handle<ClientHandler>>,
}

impl Default for RusshClient {
    fn default() -> Self {
        Self {
            handle: None,
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
        let mut config = Config::default();
        config.inactivity_timeout = Some(std::time::Duration::from_secs(30));
        config.window_size = 1024 * 1024 * 100; // 100MB window
        config.limits.rekey_time_limit = std::time::Duration::from_secs(86400);
        config.limits.rekey_read_limit = 1024 * 1024 * 1024 * 100; // 100 GB
        config.limits.rekey_write_limit = 1024 * 1024 * 1024 * 100; // 100 GB

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
        let handle = self
            .handle
            .as_mut()
            .ok_or(SshError::ConnectionFailed("Not connected".to_string()))?;

        let key_pair = russh_keys::decode_secret_key(private_key_pem, None)
            .map_err(|e| SshError::AuthenticationFailed(e.to_string()))?;

        let auth_res = handle
            .authenticate_publickey(username, Arc::new(key_pair))
            .await
            .map_err(|e| SshError::AuthenticationFailed(e.to_string()))?;

        if !auth_res {
            return Err(SshError::AuthenticationFailed(
                "Key rejected by server".to_string(),
            ));
        }

        Ok(())
    }

    async fn upload_file(
        &self,
        local_path: &Path,
        remote_path: &str,
        progress_callback: Option<Box<dyn Fn(u64) + Send>>,
    ) -> Result<(), SshError> {
        let handle = self.handle.as_ref().ok_or(SshError::ConnectionFailed("Not connected".into()))?;
        
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::CommandExecutionFailed(e.to_string()))?;

        let subsystem_name = format!("simply-transfer-data|{}", remote_path);
        channel.request_subsystem(true, &subsystem_name)
            .await
            .map_err(|e| SshError::CommandExecutionFailed(e.to_string()))?;

        let mut file = fs::File::open(local_path)
            .await
            .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;
        
        let mut buffer = vec![0u8; 1024 * 1024 * 4]; // 4MB chunks
        let mut progress = 0u64;

        loop {
            let bytes_read = file.read(&mut buffer).await
                .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;
            
            if bytes_read == 0 {
                break;
            }

            channel.data(&buffer[..bytes_read])
                .await
                .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;

            progress += bytes_read as u64;
            if let Some(cb) = &progress_callback {
                cb(progress);
            }
        }

        channel.eof().await.ok();
        channel.close().await.ok();
        
        Ok(())
    }

    async fn execute_command(&self, command: &str) -> Result<String, SshError> {
        let handle = self
            .handle
            .as_ref()
            .ok_or(SshError::ConnectionFailed("Not connected".to_string()))?;

        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::CommandExecutionFailed(e.to_string()))?;

        channel
            .exec(true, command)
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
