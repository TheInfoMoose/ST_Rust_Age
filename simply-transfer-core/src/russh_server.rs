use async_trait::async_trait;
use russh::server::{Auth, Handler, Server, Session};
use russh::{Channel, ChannelId};
use russh_keys::PublicKeyBase64;
use russh_keys::key;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, mpsc};

#[derive(Clone)]
pub struct TransferServer {
    pub authorized_keys_file_path: PathBuf,
    pub authorized_keys: Arc<tokio::sync::RwLock<HashMap<String, key::PublicKey>>>,
    file_writers: Arc<Mutex<HashMap<ChannelId, mpsc::Sender<Vec<u8>>>>>,
    pub event_sender: Option<tokio::sync::broadcast::Sender<crate::engine::TransferEvent>>,
}

impl Default for TransferServer {
    fn default() -> Self {
        Self::new()
    }
}

impl TransferServer {
    pub fn new() -> Self {
        let home_dir = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let keys_file_path = PathBuf::from(format!(
            "{}/.gemini/antigravity-ide/p2p_authorized_keys.json",
            home_dir
        ));

        let loaded_keys = if keys_file_path.exists() {
            if let Ok(file) = File::open(&keys_file_path) {
                let reader = BufReader::new(file);
                if let Ok(keys_map) = serde_json::from_reader::<_, HashMap<String, String>>(reader)
                {
                    let mut public_keys = HashMap::new();
                    for (fingerprint, key_base64) in keys_map {
                        if let Ok(public_key) = russh_keys::parse_public_key_base64(&key_base64) {
                            public_keys.insert(fingerprint, public_key);
                        }
                    }
                    public_keys
                } else {
                    HashMap::new()
                }
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };

        TransferServer {
            authorized_keys_file_path: keys_file_path,
            authorized_keys: Arc::new(tokio::sync::RwLock::new(loaded_keys)),
            file_writers: Arc::new(Mutex::new(HashMap::new())),
            event_sender: None,
        }
    }

    pub fn set_event_sender(
        &mut self,
        sender: tokio::sync::broadcast::Sender<crate::engine::TransferEvent>,
    ) {
        self.event_sender = Some(sender);
    }

    pub async fn add_authorized_key(&self, key: key::PublicKey) {
        let fingerprint = key.fingerprint().to_string();

        self.authorized_keys.write().await.insert(fingerprint, key);

        let keys = self.authorized_keys.read().await;

        let path = &self.authorized_keys_file_path;
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let mut keys_map: HashMap<String, String> = HashMap::new();
        for (fingerprint, public_key) in keys.iter() {
            keys_map.insert(fingerprint.clone(), public_key.public_key_base64());
        }

        if let Ok(file) = File::create(path) {
            let mut writer = BufWriter::new(file);
            let _ = serde_json::to_writer_pretty(&mut writer, &keys_map);
            let _ = writer.flush();

            #[cfg(unix)]
            {
                if let Ok(mut perms) = fs::metadata(path).map(|m| m.permissions()) {
                    perms.set_mode(0o600);
                    let _ = fs::set_permissions(path, perms);
                }
            }
        }
    }
}

impl Server for TransferServer {
    type Handler = Self;

    fn new_client(&mut self, _peer_addr: Option<std::net::SocketAddr>) -> Self::Handler {
        self.clone()
    }
}

#[async_trait]
impl Handler for TransferServer {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        let fingerprint = public_key.fingerprint().to_string();

        let keys = self.authorized_keys.read().await;
        tracing::info!(
            "Received auth request for user '{}' with fingerprint: {}. Known keys: {:?}",
            user,
            fingerprint,
            keys.keys()
        );

        if keys.contains_key(&fingerprint) {
            tracing::info!("Key accepted for {}", user);
            Ok(Auth::Accept)
        } else {
            tracing::warn!("Key rejected for {}", user);
            Ok(Auth::Reject {
                proceed_with_methods: None,
            })
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<russh::server::Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        tracing::info!("Session channel opened: {:?}", channel.id());
        Ok(true)
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name.starts_with("data-quic|") {
            let payload = name.trim_start_matches("data-quic|");
            let (offset, remote_path) = if let Some((offset_str, path)) = payload.split_once('|') {
                if let Ok(off) = offset_str.parse::<u64>() {
                    (off, path.to_string())
                } else {
                    (0, payload.to_string())
                }
            } else {
                (0, payload.to_string())
            };

            tracing::info!(
                "Accepted QUIC binary transfer for: {} at offset {}",
                remote_path,
                offset
            );

            let _ = rustls::crypto::ring::default_provider().install_default();

            let token: u64 = rand::random();
            let cert = match rcgen::generate_simple_self_signed(vec!["localhost".into()]) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Failed to generate QUIC certificate: {}", e);
                    return Ok(());
                }
            };
            let cert_der = cert.cert.der().to_vec();
            let priv_key = cert.signing_key.serialize_der();

            let cert_chain = vec![rustls::pki_types::CertificateDer::from(cert_der.clone())];
            let key = rustls::pki_types::PrivateKeyDer::Pkcs8(
                rustls::pki_types::PrivatePkcs8KeyDer::from(priv_key),
            );

            let mut server_crypto = match rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(cert_chain, key)
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Failed to build QUIC server crypto: {}", e);
                    return Ok(());
                }
            };
            server_crypto.alpn_protocols = vec![b"simply-transfer".to_vec()];

            let quic_server_config =
                match quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("Failed to convert rustls config: {}", e);
                        return Ok(());
                    }
                };
            let server_config =
                quinn::ServerConfig::with_crypto(std::sync::Arc::new(quic_server_config));
            let endpoint =
                match quinn::Endpoint::server(server_config, "0.0.0.0:0".parse().unwrap()) {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::error!("Failed to bind QUIC endpoint: {}", e);
                        return Ok(());
                    }
                };
            let port = endpoint.local_addr().unwrap().port();

            use base64::{Engine as _, engine::general_purpose::STANDARD};
            let cert_b64 = STANDARD.encode(&cert_der);

            let response = format!("{}|{}|{}", port, token, cert_b64);
            session.data(channel, russh::CryptoVec::from_slice(response.as_bytes()));

            let event_sender_clone = self.event_sender.clone();

            tokio::spawn(async move {
                if let Some(conn) = endpoint.accept().await
                    && let Ok(connection) = conn.await
                {
                    if let Ok(mut bi) = connection.accept_bi().await {
                        let mut token_buf = [0u8; 8];
                        if bi.1.read_exact(&mut token_buf).await.is_ok() {
                            let received_token = u64::from_le_bytes(token_buf);
                            if received_token == token {
                                if let Some(sender) = &event_sender_clone {
                                    let _ =
                                        sender.send(crate::engine::TransferEvent::TransferStarted(
                                            "Incoming".to_string(),
                                        ));
                                }
                                let path = std::path::Path::new(&remote_path);
                                if let Some(parent) = path.parent() {
                                    let _ = tokio::fs::create_dir_all(parent).await;
                                }

                                let file_result = if offset == 0 {
                                    tokio::fs::File::create(&remote_path).await
                                } else {
                                    tokio::fs::OpenOptions::new()
                                        .write(true)
                                        .open(&remote_path)
                                        .await
                                };

                                if let Ok(mut file) = file_result {
                                    if offset > 0 {
                                        use std::io::SeekFrom;
                                        use tokio::io::AsyncSeekExt;
                                        let _ = file.seek(SeekFrom::Start(offset)).await;
                                    }

                                    let mut buf = vec![0u8; 1024 * 1024];
                                    let mut total_bytes_written = offset;

                                    if let Some(sender) = &event_sender_clone {
                                        let _ = sender.send(
                                            crate::engine::TransferEvent::FileStatusChanged(
                                                remote_path.clone(),
                                                crate::engine::FileTransferStatus::Transferring {
                                                    progress_bytes: total_bytes_written,
                                                    total_bytes: 0,
                                                },
                                            ),
                                        );
                                    }

                                    while let Ok(Some(n)) = bi.1.read(&mut buf).await {
                                        if file.write_all(&buf[..n]).await.is_err() {
                                            break;
                                        }
                                        total_bytes_written += n as u64;
                                        if let Some(sender) = &event_sender_clone {
                                            let _ = sender.send(crate::engine::TransferEvent::FileStatusChanged(
                                                    remote_path.clone(),
                                                    crate::engine::FileTransferStatus::Transferring { progress_bytes: total_bytes_written, total_bytes: 0 }
                                                ));
                                        }
                                    }
                                    let _ = file.flush().await;
                                    drop(file);

                                    if let Some(sender) = &event_sender_clone {
                                        let _ = sender.send(
                                            crate::engine::TransferEvent::FileStatusChanged(
                                                remote_path.clone(),
                                                crate::engine::FileTransferStatus::Completed,
                                            ),
                                        );
                                    }

                                    // Send application-layer ACK that the file has been successfully written and closed
                                    let _ = bi.0.write_all(b"OK").await;
                                    let _ = bi.0.finish();
                                }
                            }
                        }
                    }
                    let _ = connection.closed().await;
                }
            });

            Ok(())
        } else {
            tracing::info!("Rejected unknown subsystem: {}", name);
            Ok(())
        }
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        program: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let cmd_str = String::from_utf8_lossy(program).to_string();
        tracing::info!("Executing command via exec_request: {}", cmd_str);

        if cmd_str.starts_with("simply-transfer-exists|") {
            let paths_str = cmd_str.trim_start_matches("simply-transfer-exists|");
            let paths: Vec<String> = paths_str.split('|').map(|s| s.to_string()).collect();

            let handle = session.handle();
            tokio::spawn(async move {
                let mut existing_paths = Vec::new();
                for path in paths.iter().filter(|&p| !p.trim().is_empty()) {
                    if tokio::fs::metadata(path).await.is_ok() {
                        existing_paths.push(path.clone());
                    }
                }
                let response = format!("{}\n", existing_paths.join("|"));
                let _ = handle
                    .data(channel, russh::CryptoVec::from_slice(response.as_bytes()))
                    .await;
                let _ = handle.eof(channel).await;
                let _ = handle.close(channel).await;
            });
            return Ok(());
        }

        if cmd_str.starts_with("simply-transfer-hash|") {
            let paths_str = cmd_str.trim_start_matches("simply-transfer-hash|");
            let paths: Vec<String> = paths_str.split('|').map(|s| s.to_string()).collect();

            tracing::info!(
                "simply-transfer-hash| command received with {} files",
                paths.len()
            );

            let handle = session.handle();
            tokio::spawn(async move {
                let output_str = tokio::task::spawn_blocking(move || {
                    let mut out = String::new();
                    for path in paths.iter().filter(|&p| !p.trim().is_empty()) {
                        tracing::info!("Hashing remote file: {}", path);
                        let local_hash = if let Ok(f) = std::fs::File::open(path) {
                            simply_transfer_crypto::hash::compute_sha256_stream(f)
                                .unwrap_or_else(|_| "hash_failed".to_string())
                        } else {
                            "hash_failed".to_string()
                        };
                        tracing::info!("Remote file {} hashed: {}", path, local_hash);
                        out.push_str(&format!("{}  {}\n", local_hash, path));
                    }
                    out
                })
                .await
                .unwrap_or_else(|_| String::new());

                tracing::info!(
                    "Sending SSH hash response back to client. Payload length: {}",
                    output_str.len()
                );

                let _ = handle
                    .data(channel, russh::CryptoVec::from_slice(output_str.as_bytes()))
                    .await;
                let _ = handle.exit_status_request(channel, 0).await;
                let _ = handle.eof(channel).await;
                let _ = handle.close(channel).await;
            });
            return Ok(());
        }

        #[cfg(windows)]
        let output = {
            use std::os::windows::process::CommandExt;
            std::process::Command::new("cmd")
                .arg("/Q")
                .arg("/c")
                .raw_arg(&cmd_str)
                .output()
        };
        #[cfg(not(windows))]
        let output = std::process::Command::new("sh")
            .args(["-c", &cmd_str])
            .output();

        match output {
            Ok(out) => {
                session.data(channel, russh::CryptoVec::from_slice(&out.stdout));
                session.extended_data(channel, 1, russh::CryptoVec::from_slice(&out.stderr));
                session.exit_status_request(channel, out.status.code().unwrap_or(0) as u32);
            }
            Err(e) => {
                tracing::error!("Failed to execute command: {}", e);
                session.exit_status_request(channel, 1);
            }
        }

        session.eof(channel);
        session.close(channel);
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(tx) = self.file_writers.lock().await.get(&channel) {
            let _ = tx.send(data.to_vec()).await;
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.file_writers.lock().await.remove(&channel);
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.file_writers.lock().await.remove(&channel);
        Ok(())
    }
}

pub async fn run_server(
    port: u16,
    server: TransferServer,
    host_key: key::KeyPair,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = russh::server::Config {
        keys: vec![host_key],
        ..Default::default()
    };

    let addr = format!("0.0.0.0:{}", port);
    tracing::info!("Starting SSH transfer server on {}", addr);

    let config = Arc::new(config);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    loop {
        let (socket, _) = listener.accept().await?;
        let config_clone = config.clone();
        let mut server_clone = server.clone();

        tokio::spawn(async move {
            if let Err(e) =
                russh::server::run_stream(config_clone, socket, server_clone.new_client(None)).await
            {
                tracing::error!("SSH server error: {}", e);
            }
        });
    }
}
