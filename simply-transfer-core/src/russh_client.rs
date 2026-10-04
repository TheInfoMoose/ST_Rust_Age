use crate::ssh::{SshClient, SshError};
use async_trait::async_trait;
use russh::{client::Config, client::Handle};
use std::path::Path;
use std::sync::Arc;
use tokio::fs;

#[derive(Default)]
pub struct RusshClient {
    handle: Option<Handle<ClientHandler>>,
    remote_host: Option<String>,
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

        self.remote_host = Some(host.to_string());
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

    async fn get_remote_file_size(
        &self,
        remote_path: &str,
        is_windows_dest: bool,
    ) -> Result<u64, SshError> {
        let exec_request = if is_windows_dest {
            // Check if file exists first to avoid error spam
            format!(
                "powershell -NoProfile -Command \"if (Test-Path '{}') {{ (Get-Item '{}').length }} else {{ 0 }}\"",
                remote_path.replace("\"", "\"\""),
                remote_path.replace("\"", "\"\"")
            )
        } else {
            format!(
                "if [ -f \"{}\" ]; then stat -c%s \"{}\"; else echo 0; fi",
                remote_path, remote_path
            )
        };

        let output = self.execute_command(&exec_request).await?;
        Ok(output.trim().parse::<u64>().unwrap_or(0))
    }

    async fn upload_file(
        &self,
        local_path: &Path,
        remote_path: &str,
        offset: u64,
        progress_callback: Option<Box<dyn Fn(u64) + Send>>,
        mut cancel_rx: tokio::sync::watch::Receiver<crate::engine::ControlSignal>,
    ) -> Result<(), SshError> {
        let handle = self
            .handle
            .as_ref()
            .ok_or(SshError::ConnectionFailed("Not connected".into()))?;

        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::CommandExecutionFailed(e.to_string()))?;

        let subsystem_name = format!("data-quic|{}|{}", offset, remote_path);
        channel
            .request_subsystem(true, &subsystem_name)
            .await
            .map_err(|e| SshError::CommandExecutionFailed(e.to_string()))?;

        let mut response_str = String::new();
        while let Some(msg) = channel.wait().await {
            if let russh::ChannelMsg::Data { ref data } = msg {
                response_str = String::from_utf8_lossy(data).to_string();
                break;
            }
        }

        if response_str.is_empty() {
            return Err(SshError::FileTransferFailed(
                "No QUIC port provided by server".into(),
            ));
        }

        let parts: Vec<&str> = response_str.split('|').collect();
        if parts.len() != 3 {
            return Err(SshError::FileTransferFailed(
                "Invalid QUIC connection response".into(),
            ));
        }
        let port: u16 = parts[0].parse().unwrap_or(0);
        let token: u64 = parts[1].parse().unwrap_or(0);

        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let cert_der = STANDARD
            .decode(parts[2])
            .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;

        let mut root_cert_store = rustls::RootCertStore::empty();
        root_cert_store
            .add(rustls::pki_types::CertificateDer::from(cert_der))
            .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;

        let _ = rustls::crypto::ring::default_provider().install_default();

        let mut client_crypto = rustls::ClientConfig::builder()
            .with_root_certificates(root_cert_store)
            .with_no_client_auth();
        client_crypto.alpn_protocols = vec![b"simply-transfer".to_vec()];

        let quic_client_config = quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)
            .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;

        let mut client_endpoint = quinn::Endpoint::client("0.0.0.0:0".parse().unwrap())
            .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;
        client_endpoint.set_default_client_config(quinn::ClientConfig::new(std::sync::Arc::new(
            quic_client_config,
        )));

        let remote_host = self.remote_host.as_deref().unwrap_or("127.0.0.1");
        let connect_addr: std::net::SocketAddr = format!("{}:{}", remote_host, port)
            .parse()
            .map_err(|e| SshError::FileTransferFailed(format!("Invalid socket address: {}", e)))?;

        let connection = client_endpoint
            .connect(connect_addr, "localhost")
            .map_err(|e| SshError::FileTransferFailed(e.to_string()))?
            .await
            .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;
        let mut bi = connection
            .open_bi()
            .await
            .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;

        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        bi.0.write_all(&token.to_le_bytes())
            .await
            .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;

        let mut file = fs::File::open(local_path)
            .await
            .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;

        if offset > 0 {
            use tokio::io::AsyncSeekExt;
            file.seek(tokio::io::SeekFrom::Start(offset))
                .await
                .map_err(|e| SshError::FileTransferFailed(e.to_string()))?;
        }

        let mut buffer = vec![0u8; 1024 * 1024 * 4]; // 4MB chunks
        let mut progress = offset;

        loop {
            tokio::select! {
                res = file.read(&mut buffer) => {
                    match res {
                        Ok(0) => break,
                        Ok(n) => {
                            if bi.0.write_all(&buffer[..n]).await.is_err() {
                                return Err(SshError::FileTransferFailed("Connection dropped".into()));
                            }
                            progress += n as u64;
                            if let Some(cb) = &progress_callback {
                                cb(progress);
                            }
                        }
                        Err(e) => return Err(SshError::FileTransferFailed(e.to_string())),
                    }
                }
                _ = cancel_rx.changed() => {
                    if *cancel_rx.borrow() == crate::engine::ControlSignal::Cancel {
                        let _ = bi.0.finish();
                        connection.close(0u32.into(), b"cancelled");
                        return Err(SshError::FileTransferFailed("Transfer cancelled".into()));
                    }
                }
            }
        }

        let _ = bi.0.finish();

        let mut ack_buf = [0u8; 2];
        if let Ok(_) = bi.1.read_exact(&mut ack_buf).await {
            if &ack_buf != b"OK" {
                tracing::warn!("Did not receive valid application-layer ACK");
                connection.close(0u32.into(), b"failed");
                return Err(SshError::FileTransferFailed(
                    "Invalid application-layer ACK".into(),
                ));
            }
        } else {
            tracing::warn!("Failed to read application-layer ACK");
            connection.close(0u32.into(), b"failed");
            return Err(SshError::FileTransferFailed(
                "Failed to read application-layer ACK".into(),
            ));
        }

        connection.close(0u32.into(), b"done");

        Ok(())
    }

    async fn execute_command(&self, command: &str) -> Result<String, SshError> {
        let handle = self
            .handle
            .as_ref()
            .ok_or(SshError::ConnectionFailed("Not connected".to_string()))?;

        let mut channel = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            handle.channel_open_session(),
        )
        .await
        .map_err(|_| SshError::CommandExecutionFailed("Timeout opening channel".to_string()))?
        .map_err(|e| SshError::CommandExecutionFailed(e.to_string()))?;

        tokio::time::timeout(
            std::time::Duration::from_secs(60),
            channel.exec(true, command),
        )
        .await
        .map_err(|_| SshError::CommandExecutionFailed("Timeout executing command".to_string()))?
        .map_err(|e| SshError::CommandExecutionFailed(e.to_string()))?;

        let mut output = String::new();
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(600), channel.wait()).await {
                Ok(Some(msg)) => match msg {
                    russh::ChannelMsg::Data { ref data } => {
                        output.push_str(&String::from_utf8_lossy(data));
                    }
                    russh::ChannelMsg::Eof | russh::ChannelMsg::Close => {
                        break;
                    }
                    _ => {}
                },
                Ok(None) => break,
                Err(_) => {
                    tracing::warn!("Timeout waiting for SSH command output");
                    break;
                }
            }
        }

        Ok(output)
    }
}
