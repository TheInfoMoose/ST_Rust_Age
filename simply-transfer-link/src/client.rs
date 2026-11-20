use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use russh::ChannelStream;
use russh::client::{AuthResult, Config, Handle, Handler, Msg};
use russh::keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, aws_lc_rs, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, Error as RustlsError, SignatureScheme};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::{Mutex, watch};

use crate::LinkError;
use crate::protocol::{
    EntryInfo, Request, Response, ShareRootInfo, VolumeEntry, read_frame, write_frame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostPin(pub [u8; 32]);

impl HostPin {
    pub fn of(key: &PublicKey) -> Self {
        let fp = key.fingerprint(HashAlg::Sha256);
        let bytes = fp.as_bytes();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(bytes);
        Self(arr)
    }

    pub fn from_hex(s: &str) -> Result<Self, LinkError> {
        if s.len() != 64 {
            return Err(LinkError::Protocol(format!(
                "expected 64-char hex string for HostPin, got length {}",
                s.len()
            )));
        }
        let mut out = [0u8; 32];
        let (chunks, _) = s.as_bytes().as_chunks::<2>();
        for (i, &[hi_b, lo_b]) in chunks.iter().enumerate() {
            let hi = decode_nibble(hi_b)
                .ok_or_else(|| LinkError::Protocol("invalid hex char in HostPin".to_string()))?;
            let lo = decode_nibble(lo_b)
                .ok_or_else(|| LinkError::Protocol("invalid hex char in HostPin".to_string()))?;
            out[i] = (hi << 4) | lo;
        }
        Ok(Self(out))
    }

    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for &b in &self.0 {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
        }
        s
    }
}

fn decode_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn pin_matches(pin: &HostPin, key: &PublicKey) -> bool {
    &HostPin::of(key) == pin
}

struct ClientHandler {
    pin: HostPin,
    mismatch: Arc<AtomicBool>,
}

impl Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let matches = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => pin_matches(&self.pin, key),
            PublicKeyOrCertificate::Certificate(_) => false,
        };
        if !matches {
            self.mismatch.store(true, Ordering::SeqCst);
        }
        Ok(matches)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UploadAck {
    pub bytes_written: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Run,
    Pause,
    Cancel,
}

pub struct LinkClient {
    _handle: Handle<ClientHandler>,
    stream: Mutex<ChannelStream<Msg>>,
    server_ip: IpAddr,
}

impl LinkClient {
    pub async fn connect(
        addr: SocketAddr,
        pin: HostPin,
        identity: PrivateKey,
    ) -> Result<Self, LinkError> {
        let mismatch = Arc::new(AtomicBool::new(false));
        let handler = ClientHandler {
            pin,
            mismatch: Arc::clone(&mismatch),
        };
        let config = Arc::new(Config::default());

        let mut handle = match russh::client::connect(config, addr, handler).await {
            Ok(h) => h,
            Err(e) => {
                if mismatch.load(Ordering::SeqCst) {
                    return Err(LinkError::HostKeyMismatch);
                }
                return Err(LinkError::Auth(e.to_string()));
            }
        };

        let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(identity), None);
        let auth_res = match handle.authenticate_publickey("st", key_with_alg).await {
            Ok(res) => res,
            Err(e) => {
                if mismatch.load(Ordering::SeqCst) {
                    return Err(LinkError::HostKeyMismatch);
                }
                return Err(LinkError::Auth(e.to_string()));
            }
        };

        if !matches!(auth_res, AuthResult::Success) {
            if mismatch.load(Ordering::SeqCst) {
                return Err(LinkError::HostKeyMismatch);
            }
            return Err(LinkError::Auth("publickey rejected".to_string()));
        }

        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| LinkError::Protocol(format!("failed to open session channel: {e}")))?;
        channel
            .request_subsystem(true, "st-control/1")
            .await
            .map_err(|e| LinkError::Protocol(format!("failed to request st-control/1: {e}")))?;

        Ok(Self {
            _handle: handle,
            stream: Mutex::new(channel.into_stream()),
            server_ip: addr.ip(),
        })
    }

    pub async fn pair(
        addr: SocketAddr,
        pin: HostPin,
        secret: &str,
        identity: &PrivateKey,
        label: &str,
    ) -> Result<(), LinkError> {
        let mismatch = Arc::new(AtomicBool::new(false));
        let handler = ClientHandler {
            pin,
            mismatch: Arc::clone(&mismatch),
        };
        let config = Arc::new(Config::default());

        let mut handle = match russh::client::connect(config, addr, handler).await {
            Ok(h) => h,
            Err(e) => {
                if mismatch.load(Ordering::SeqCst) {
                    return Err(LinkError::HostKeyMismatch);
                }
                return Err(LinkError::Auth(e.to_string()));
            }
        };

        let auth_res = match handle.authenticate_password("pair", secret).await {
            Ok(res) => res,
            Err(e) => {
                if mismatch.load(Ordering::SeqCst) {
                    return Err(LinkError::HostKeyMismatch);
                }
                return Err(LinkError::Auth(e.to_string()));
            }
        };

        if !matches!(auth_res, AuthResult::Success) {
            if mismatch.load(Ordering::SeqCst) {
                return Err(LinkError::HostKeyMismatch);
            }
            return Err(LinkError::Auth("pairing secret rejected".to_string()));
        }

        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| LinkError::Protocol(format!("failed to open session channel: {e}")))?;
        channel
            .request_subsystem(true, "st-control/1")
            .await
            .map_err(|e| LinkError::Protocol(format!("failed to request st-control/1: {e}")))?;

        let mut stream = channel.into_stream();
        let client_pubkey = identity
            .public_key()
            .to_openssh()
            .map_err(|e| LinkError::Protocol(format!("failed to encode public key: {e}")))?;

        let req = Request::Pair {
            client_pubkey,
            label: label.to_string(),
        };
        write_frame(&mut stream, &req).await?;
        let resp: Response = read_frame(&mut stream).await?;
        match resp {
            Response::Paired => Ok(()),
            Response::Error { code, message } => {
                Err(LinkError::Protocol(format!("{code}: {message}")))
            }
            _ => Err(LinkError::Protocol("unexpected response".to_string())),
        }
    }

    async fn round_trip(&self, req: &Request) -> Result<Response, LinkError> {
        let mut stream = self.stream.lock().await;
        write_frame(&mut *stream, req).await?;
        read_frame(&mut *stream).await
    }

    pub async fn roots(&self) -> Result<Vec<ShareRootInfo>, LinkError> {
        match self.round_trip(&Request::Roots).await? {
            Response::Roots { roots } => Ok(roots),
            Response::Error { code, message } => {
                Err(LinkError::Protocol(format!("{code}: {message}")))
            }
            _ => Err(LinkError::Protocol("unexpected response".to_string())),
        }
    }

    pub async fn list(&self, root: &str, rel: &str) -> Result<Vec<EntryInfo>, LinkError> {
        let req = Request::List {
            root_id: root.to_string(),
            rel_path: rel.to_string(),
        };
        match self.round_trip(&req).await? {
            Response::List { entries } => Ok(entries),
            Response::Error { code, message } => {
                Err(LinkError::Protocol(format!("{code}: {message}")))
            }
            _ => Err(LinkError::Protocol("unexpected response".to_string())),
        }
    }

    pub async fn stat(&self, root: &str, rel: &str) -> Result<EntryInfo, LinkError> {
        let req = Request::Stat {
            root_id: root.to_string(),
            rel_path: rel.to_string(),
        };
        match self.round_trip(&req).await? {
            Response::Stat { entry } => Ok(entry),
            Response::Error { code, message } => {
                Err(LinkError::Protocol(format!("{code}: {message}")))
            }
            _ => Err(LinkError::Protocol("unexpected response".to_string())),
        }
    }

    pub async fn hash(&self, root: &str, rel: &str) -> Result<String, LinkError> {
        let req = Request::Hash {
            root_id: root.to_string(),
            rel_path: rel.to_string(),
        };
        match self.round_trip(&req).await? {
            Response::Hash { sha256 } => Ok(sha256),
            Response::Error { code, message } => {
                Err(LinkError::Protocol(format!("{code}: {message}")))
            }
            _ => Err(LinkError::Protocol("unexpected response".to_string())),
        }
    }

    pub async fn exists(&self, root: &str, rel: &str) -> Result<bool, LinkError> {
        let req = Request::Exists {
            root_id: root.to_string(),
            rel_path: rel.to_string(),
        };
        match self.round_trip(&req).await? {
            Response::Exists { exists } => Ok(exists),
            Response::Error { code, message } => {
                Err(LinkError::Protocol(format!("{code}: {message}")))
            }
            _ => Err(LinkError::Protocol("unexpected response".to_string())),
        }
    }

    pub async fn mkdir(&self, root: &str, rel: &str) -> Result<(), LinkError> {
        let req = Request::Mkdir {
            root_id: root.to_string(),
            rel_path: rel.to_string(),
        };
        match self.round_trip(&req).await? {
            Response::Ok => Ok(()),
            Response::Error { code, message } => {
                Err(LinkError::Protocol(format!("{code}: {message}")))
            }
            _ => Err(LinkError::Protocol("unexpected response".to_string())),
        }
    }

    pub async fn volumes(&self) -> Result<Vec<VolumeEntry>, LinkError> {
        match self.round_trip(&Request::Volumes).await? {
            Response::Volumes { volumes } => Ok(volumes),
            Response::Error { code, message } => {
                Err(LinkError::Protocol(format!("{code}: {message}")))
            }
            _ => Err(LinkError::Protocol("unexpected response".to_string())),
        }
    }

    pub async fn disk_free(&self, root: &str) -> Result<u64, LinkError> {
        let req = Request::DiskFree {
            root_id: root.to_string(),
        };
        match self.round_trip(&req).await? {
            Response::DiskFree { available_bytes } => Ok(available_bytes),
            Response::Error { code, message } => {
                Err(LinkError::Protocol(format!("{code}: {message}")))
            }
            _ => Err(LinkError::Protocol("unexpected response".to_string())),
        }
    }

    pub async fn upload(
        &self,
        local: &Path,
        root: &str,
        rel: &str,
        offset: u64,
        progress: Option<Box<dyn Fn(u64) + Send>>,
        mut cancel: watch::Receiver<Control>,
    ) -> Result<UploadAck, LinkError> {
        let meta = tokio::fs::metadata(local).await?;
        let file_len = meta.len();
        if offset > file_len {
            return Err(LinkError::Protocol(format!(
                "offset {offset} exceeds file length {file_len}"
            )));
        }
        let len = file_len - offset;

        wait_while_paused(&mut cancel).await?;

        let req = Request::BeginUpload {
            root_id: root.to_string(),
            rel_path: rel.to_string(),
            offset,
            len,
        };
        let grant = match self.round_trip(&req).await? {
            Response::UploadGrant { grant } => grant,
            Response::Error { code, message } => {
                return Err(LinkError::Protocol(format!("{code}: {message}")));
            }
            _ => return Err(LinkError::Protocol("unexpected response".to_string())),
        };

        let provider = Arc::new(aws_lc_rs::default_provider());
        let verifier = Arc::new(PinnedCertVerifier {
            expected_sha256: grant.cert_sha256,
            provider: Arc::clone(&provider),
        });

        let mut client_crypto = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| LinkError::Protocol(format!("failed to set TLS versions: {e}")))?
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        client_crypto.alpn_protocols = vec![b"simply-transfer".to_vec()];

        let quic_client_config = quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)
            .map_err(|e| {
            LinkError::Protocol(format!("failed to build QUIC client config: {e}"))
        })?;

        let bind_addr = match self.server_ip {
            IpAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            IpAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
        };
        let mut endpoint = quinn::Endpoint::client(bind_addr)?;
        endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(quic_client_config)));

        let target_addr = SocketAddr::new(self.server_ip, grant.udp_port);
        let connection = endpoint
            .connect(target_addr, "localhost")
            .map_err(|e| LinkError::Protocol(format!("QUIC connect error: {e}")))?
            .await
            .map_err(|e| LinkError::Protocol(format!("QUIC handshake failed: {e}")))?;

        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| LinkError::Protocol(format!("QUIC open_bi failed: {e}")))?;

        send.write_all(&grant.ticket)
            .await
            .map_err(|e| LinkError::Protocol(format!("failed to write ticket: {e}")))?;

        let mut file = tokio::fs::File::open(local).await?;
        if offset > 0 {
            file.seek(std::io::SeekFrom::Start(offset)).await?;
        }

        let mut buffer = vec![0u8; 4 * 1024 * 1024];
        let mut pos = offset;

        loop {
            tokio::select! {
                res = file.read(&mut buffer) => {
                    match res {
                        Ok(0) => break,
                        Ok(n) => {
                            if send.write_all(&buffer[..n]).await.is_err() {
                                return Err(LinkError::Protocol("connection dropped".to_string()));
                            }
                            pos += n as u64;
                            if let Some(cb) = &progress {
                                cb(pos);
                            }
                        }
                        Err(e) => return Err(LinkError::Io(e)),
                    }
                }
                res = cancel.changed() => {
                    if res.is_err() || wait_while_paused(&mut cancel).await.is_err() {
                        let _ = send.finish();
                        connection.close(0u32.into(), b"cancelled");
                        return Err(LinkError::Protocol("cancelled".into()));
                    }
                }
            }
        }

        let _ = send.finish();

        let mut ack_buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match recv.read(&mut byte).await {
                Ok(Some(0)) | Ok(None) => break,
                Ok(Some(_)) => {
                    if byte[0] == b'\n' {
                        break;
                    }
                    if ack_buf.len() >= 4096 {
                        connection.close(0u32.into(), b"ack too large");
                        return Err(LinkError::Protocol("upload ACK exceeded 4 KiB".to_string()));
                    }
                    ack_buf.push(byte[0]);
                }
                Err(e) => {
                    connection.close(0u32.into(), b"failed");
                    return Err(LinkError::Protocol(format!("failed to read ACK: {e}")));
                }
            }
        }

        if ack_buf.is_empty() {
            connection.close(0u32.into(), b"failed");
            return Err(LinkError::Protocol("missing upload ACK".to_string()));
        }

        let ack: UploadAck = serde_json::from_slice(&ack_buf).map_err(|e| {
            connection.close(0u32.into(), b"failed");
            LinkError::Protocol(format!("invalid upload ACK JSON: {e}"))
        })?;

        connection.close(0u32.into(), b"done");

        if ack.bytes_written != len {
            return Err(LinkError::Protocol(format!(
                "ACK bytes_written {} does not match expected {len}",
                ack.bytes_written
            )));
        }

        Ok(ack)
    }
}

async fn wait_while_paused(rx: &mut watch::Receiver<Control>) -> Result<(), LinkError> {
    loop {
        match *rx.borrow_and_update() {
            Control::Run => return Ok(()),
            Control::Cancel => return Err(LinkError::Protocol("cancelled".into())),
            Control::Pause => {}
        }
        if rx.changed().await.is_err() {
            return Err(LinkError::Protocol("cancelled".into()));
        }
    }
}

#[derive(Debug)]
struct PinnedCertVerifier {
    expected_sha256: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        let digest: [u8; 32] = Sha256::digest(end_entity.as_ref()).into();
        if digest == self.expected_sha256 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(RustlsError::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::Algorithm;

    fn random_pubkey() -> PublicKey {
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .unwrap()
            .public_key()
            .clone()
    }

    #[test]
    fn pin_matches_same_key_and_rejects_different_key() {
        let key1 = random_pubkey();
        let key2 = random_pubkey();
        let pin = HostPin::of(&key1);

        assert!(pin_matches(&pin, &key1));
        assert!(!pin_matches(&pin, &key2));
    }

    #[test]
    fn host_pin_hex_roundtrip_and_validation() {
        let key = random_pubkey();
        let pin = HostPin::of(&key);
        let hex = pin.to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(HostPin::from_hex(&hex).unwrap(), pin);

        assert!(HostPin::from_hex("abcd").is_err());
        assert!(HostPin::from_hex(&"z".repeat(64)).is_err());
    }

    #[tokio::test]
    async fn wait_while_paused_starting_at_run_returns_ok() {
        let (_tx, mut rx) = watch::channel(Control::Run);
        assert!(wait_while_paused(&mut rx).await.is_ok());
    }

    #[tokio::test]
    async fn wait_while_paused_starting_at_cancel_returns_err() {
        let (_tx, mut rx) = watch::channel(Control::Cancel);
        assert!(matches!(
            wait_while_paused(&mut rx).await,
            Err(LinkError::Protocol(_))
        ));
    }

    #[tokio::test]
    async fn wait_while_paused_resumes_when_sender_sends_run() {
        let (tx, mut rx) = watch::channel(Control::Pause);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = tx.send(Control::Run);
        });
        assert!(wait_while_paused(&mut rx).await.is_ok());
    }

    #[tokio::test]
    async fn wait_while_paused_treats_dropped_sender_as_cancel() {
        let (tx, mut rx) = watch::channel(Control::Pause);
        drop(tx);
        let res = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            wait_while_paused(&mut rx),
        )
        .await;
        let inner = res.expect("wait_while_paused timed out when sender was dropped");
        assert!(matches!(inner, Err(LinkError::Protocol(_))));
    }
}
