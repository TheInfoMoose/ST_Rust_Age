use russh::server::{Auth, Handler, Server, Session};
use russh::{Channel, ChannelId};
use russh_keys::key;
use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex};
use tokio::io::AsyncWriteExt;

#[derive(Clone)]
pub struct TransferServer {
    pub authorized_keys: Arc<tokio::sync::RwLock<HashMap<String, key::PublicKey>>>,
    file_writers: Arc<Mutex<HashMap<ChannelId, mpsc::Sender<Vec<u8>>>>>,
}

impl TransferServer {
    pub fn new() -> Self {
        TransferServer { 
            authorized_keys: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            file_writers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn add_authorized_key(&self, key: key::PublicKey) {
        let fingerprint = key.fingerprint().to_string();
        self.authorized_keys.write().await.insert(fingerprint, key);
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
        tracing::info!("Received auth request for user '{}' with fingerprint: {}. Known keys: {:?}", user, fingerprint, keys.keys());
        
        if keys.contains_key(&fingerprint) {
            tracing::info!("Key accepted for {}", user);
            Ok(Auth::Accept)
        } else {
            tracing::warn!("Key rejected for {}", user);
            Ok(Auth::Reject { proceed_with_methods: None })
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
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name.starts_with("simply-transfer-data|") {
            let remote_path = name.trim_start_matches("simply-transfer-data|").to_string();
            tracing::info!("Accepted binary transfer for: {}", remote_path);

            let (tx, mut rx) = mpsc::channel::<Vec<u8>>(100);
            self.file_writers.lock().await.insert(channel, tx);

            tokio::spawn(async move {
                let path = std::path::Path::new(&remote_path);
                if let Some(parent) = path.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                
                if let Ok(mut file) = tokio::fs::File::create(&remote_path).await {
                    while let Some(chunk) = rx.recv().await {
                        if file.write_all(&chunk).await.is_err() {
                            break;
                        }
                    }
                    let _ = file.flush().await;
                } else {
                    tracing::error!("Failed to create file: {}", remote_path);
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
        let output = std::process::Command::new("sh").args(&["-c", &cmd_str]).output();

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
            if let Err(e) = russh::server::run_stream(config_clone, socket, server_clone.new_client(None)).await {
                tracing::error!("SSH server error: {}", e);
            }
        });
    }
}
