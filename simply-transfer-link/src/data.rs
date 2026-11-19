use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::LinkError;
use crate::paths::open_for_write;
use crate::protocol::UploadGrant;

const GRANT_TTL: Duration = Duration::from_secs(30);
const MAX_GRANTS_PER_PEER: usize = 8;
const ACCEPT_BI_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub(crate) struct Grant {
    pub(crate) peer_fp: String,
    pub(crate) path: PathBuf,
    pub(crate) offset: u64,
    pub(crate) len: u64,
    pub(crate) expires: Instant,
}

#[derive(Debug, Serialize)]
struct UploadAck {
    bytes_written: u64,
    sha256: String,
}

pub(crate) struct DataPlane {
    pub(crate) cert_sha256: [u8; 32],
    pub(crate) endpoint: quinn::Endpoint,
    pub(crate) tickets: Mutex<HashMap<[u8; 32], Grant>>,
}

impl DataPlane {
    pub(crate) fn bind(addr: SocketAddr) -> Result<Arc<Self>, LinkError> {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .map_err(|e| LinkError::Protocol(format!("failed to generate QUIC cert: {e}")))?;
        let cert_der = cert.cert.der().to_vec();
        let priv_key = cert.signing_key.serialize_der();

        let cert_sha256: [u8; 32] = Sha256::digest(&cert_der).into();

        let cert_chain = vec![rustls::pki_types::CertificateDer::from(cert_der)];
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(priv_key),
        );

        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut server_crypto = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| LinkError::Protocol(format!("failed to set TLS versions: {e}")))?
            .with_no_client_auth()
            .with_single_cert(cert_chain, key)
            .map_err(|e| LinkError::Protocol(format!("failed to build QUIC server crypto: {e}")))?;
        server_crypto.alpn_protocols = vec![b"simply-transfer".to_vec()];

        let quic_server_config = quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)
            .map_err(|e| {
            LinkError::Protocol(format!("failed to convert rustls config: {e}"))
        })?;
        let server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_server_config));
        let endpoint = quinn::Endpoint::server(server_config, addr)?;

        let plane = Arc::new(Self {
            cert_sha256,
            endpoint,
            tickets: Mutex::new(HashMap::new()),
        });

        let accept_plane = Arc::clone(&plane);
        tokio::spawn(async move {
            accept_plane.run_accept_loop().await;
        });

        Ok(plane)
    }

    pub(crate) fn udp_port(&self) -> Result<u16, LinkError> {
        Ok(self.endpoint.local_addr()?.port())
    }

    pub(crate) fn issue(
        &self,
        peer_fp: &str,
        path: PathBuf,
        offset: u64,
        len: u64,
    ) -> Result<UploadGrant, LinkError> {
        self.issue_at(peer_fp, path, offset, len, Instant::now())
    }

    fn issue_at(
        &self,
        peer_fp: &str,
        path: PathBuf,
        offset: u64,
        len: u64,
        now: Instant,
    ) -> Result<UploadGrant, LinkError> {
        let udp_port = self.udp_port()?;
        let mut guard = self
            .tickets
            .lock()
            .map_err(|_| LinkError::Protocol("ticket store lock poisoned".to_string()))?;

        guard.retain(|_, g| g.expires > now);

        let live_for_peer = guard.values().filter(|g| g.peer_fp == peer_fp).count();
        if live_for_peer >= MAX_GRANTS_PER_PEER {
            return Err(LinkError::Protocol(
                "too many concurrent upload grants for peer".to_string(),
            ));
        }

        let ticket: [u8; 32] = rand::random();
        guard.insert(
            ticket,
            Grant {
                peer_fp: peer_fp.to_string(),
                path,
                offset,
                len,
                expires: now + GRANT_TTL,
            },
        );

        Ok(UploadGrant {
            udp_port,
            cert_sha256: self.cert_sha256,
            ticket,
        })
    }

    pub(crate) fn take_grant(&self, ticket: &[u8; 32], now: Instant) -> Option<Grant> {
        let mut guard = self.tickets.lock().ok()?;
        let grant = guard.remove(ticket)?;
        if grant.expires <= now {
            return None;
        }
        Some(grant)
    }

    async fn run_accept_loop(self: Arc<Self>) {
        while let Some(incoming) = self.endpoint.accept().await {
            let plane = Arc::clone(&self);
            tokio::spawn(async move {
                if let Ok(connection) = incoming.await {
                    plane.handle_connection(connection).await;
                }
            });
        }
    }

    async fn handle_connection(&self, connection: quinn::Connection) {
        let bi = match tokio::time::timeout(ACCEPT_BI_TIMEOUT, connection.accept_bi()).await {
            Ok(Ok(bi)) => bi,
            _ => {
                connection.close(1u32.into(), b"accept_bi timeout");
                return;
            }
        };
        let (mut send, mut recv) = bi;

        let mut ticket = [0u8; 32];
        if tokio::time::timeout(ACCEPT_BI_TIMEOUT, recv.read_exact(&mut ticket))
            .await
            .is_err()
        {
            connection.close(2u32.into(), b"ticket read failed");
            return;
        }

        let Some(grant) = self.take_grant(&ticket, Instant::now()) else {
            connection.close(3u32.into(), b"invalid or expired ticket");
            return;
        };

        if let Some(parent) = grant.path.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            connection.close(4u32.into(), b"mkdir failed");
            return;
        }

        let mut file = match open_for_write(&grant.path, grant.offset) {
            Ok(f) => f,
            Err(_) => {
                connection.close(4u32.into(), b"open failed");
                return;
            }
        };

        let mut written: u64 = 0;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match recv.read(&mut buf).await {
                Ok(Some(0)) | Ok(None) => break,
                Ok(Some(n)) => {
                    let next = written.saturating_add(n as u64);
                    if next > grant.len {
                        connection.close(5u32.into(), b"length exceeded");
                        return;
                    }
                    if file.write_all(&buf[..n]).is_err() {
                        connection.close(6u32.into(), b"write failed");
                        return;
                    }
                    written = next;
                }
                Err(_) => {
                    connection.close(7u32.into(), b"read error");
                    return;
                }
            }
        }

        if file.flush().is_err() {
            connection.close(6u32.into(), b"flush failed");
            return;
        }
        drop(file);

        let sha256 = match File::open(&grant.path).and_then(compute_sha256) {
            Ok(h) => h,
            Err(_) => {
                connection.close(8u32.into(), b"hash failed");
                return;
            }
        };

        let ack = UploadAck {
            bytes_written: written,
            sha256,
        };
        let Ok(mut line) = serde_json::to_vec(&ack) else {
            return;
        };
        line.push(b'\n');

        if send.write_all(&line).await.is_err() {
            return;
        }
        let _ = send.finish();
        let _ = connection.closed().await;
    }
}

fn compute_sha256<R: Read>(mut reader: R) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_plane() -> Arc<DataPlane> {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        DataPlane::bind(addr).unwrap()
    }

    #[tokio::test]
    async fn ticket_is_single_use() {
        let plane = test_plane();
        let now = Instant::now();
        let grant = plane
            .issue_at("SHA256:peer1", PathBuf::from("/tmp/a"), 0, 100, now)
            .unwrap();

        let taken = plane.take_grant(&grant.ticket, now + Duration::from_secs(1));
        assert!(taken.is_some());

        let second = plane.take_grant(&grant.ticket, now + Duration::from_secs(2));
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn expired_ticket_is_rejected() {
        let plane = test_plane();
        let now = Instant::now();
        let grant = plane
            .issue_at("SHA256:peer1", PathBuf::from("/tmp/a"), 0, 100, now)
            .unwrap();

        let expired = plane.take_grant(&grant.ticket, now + GRANT_TTL + Duration::from_millis(1));
        assert!(expired.is_none());
    }

    #[tokio::test]
    async fn ninth_live_grant_for_one_peer_is_rejected() {
        let plane = test_plane();
        let now = Instant::now();
        for i in 0..8 {
            plane
                .issue_at(
                    "SHA256:peer1",
                    PathBuf::from(format!("/tmp/{i}")),
                    0,
                    10,
                    now,
                )
                .unwrap();
        }

        let ninth = plane.issue_at("SHA256:peer1", PathBuf::from("/tmp/9"), 0, 10, now);
        assert!(ninth.is_err());

        // Another peer can still obtain a grant
        let other = plane.issue_at("SHA256:peer2", PathBuf::from("/tmp/other"), 0, 10, now);
        assert!(other.is_ok());

        // Once the first 8 grants expire, peer1 can issue again
        let after_expiry = plane.issue_at(
            "SHA256:peer1",
            PathBuf::from("/tmp/after"),
            0,
            10,
            now + GRANT_TTL + Duration::from_secs(1),
        );
        assert!(after_expiry.is_ok());
    }
}
