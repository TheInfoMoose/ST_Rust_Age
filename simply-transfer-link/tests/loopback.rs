use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use russh::client::{AuthResult, Config, Handle, Handler, Msg};
use russh::keys::{Algorithm, HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, ChannelStream};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, aws_lc_rs, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, Error as RustlsError, SignatureScheme};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::watch;

use simply_transfer_link::LinkError;
use simply_transfer_link::client::{Control, HostPin, LinkClient};
use simply_transfer_link::paths::{SharePolicy, ShareRoot};
use simply_transfer_link::protocol::{
    Request, Response, ShareRootInfo, UploadGrant, read_frame, write_frame,
};
use simply_transfer_link::server::LinkServer;
use simply_transfer_link::trust::{MemoryTrustStore, TrustStore};

struct TestServer {
    addr: SocketAddr,
    pin: HostPin,
    trust: Arc<MemoryTrustStore>,
    shares: Arc<RwLock<SharePolicy>>,
    root: TempDir,
}

async fn spawn_server() -> TestServer {
    let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let pin = HostPin::of(host_key.public_key());
    let trust = Arc::new(MemoryTrustStore::new());
    let shares = Arc::new(RwLock::new(SharePolicy::new()));
    let root = tempfile::tempdir().unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let trust_clone: Arc<dyn TrustStore> = trust.clone();
    let shares_clone = shares.clone();
    tokio::spawn(async move {
        let _ = LinkServer::builder()
            .host_key(host_key)
            .trust(trust_clone)
            .shares(shares_clone)
            .serve_on(listener)
            .await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    TestServer {
        addr,
        pin,
        trust,
        shares,
        root,
    }
}

fn client_identity() -> (PrivateKey, String) {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let fp = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    (key, fp)
}

fn grant(ts: &TestServer, fp: &str, read: bool, write: bool) {
    let mut guard = ts.shares.write().unwrap();
    guard.grant(
        fp,
        ShareRoot {
            id: "share".to_string(),
            path: ts.root.path().to_path_buf(),
            read,
            write,
        },
    );
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

struct TestClientHandler {
    pin: HostPin,
}

impl Handler for TestClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let matches = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => HostPin::of(key) == self.pin,
            PublicKeyOrCertificate::Certificate(_) => false,
        };
        Ok(matches)
    }
}

async fn raw_session_pubkey(ts: &TestServer, key: &PrivateKey) -> Handle<TestClientHandler> {
    let config = Arc::new(Config::default());
    let handler = TestClientHandler { pin: ts.pin };
    let mut handle = russh::client::connect(config, ts.addr, handler)
        .await
        .unwrap();
    let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key.clone()), None);
    let res = handle
        .authenticate_publickey("st", key_with_alg)
        .await
        .unwrap();
    assert!(matches!(res, AuthResult::Success));
    handle
}

async fn raw_session_pair(ts: &TestServer, secret: &str) -> Handle<TestClientHandler> {
    let config = Arc::new(Config::default());
    let handler = TestClientHandler { pin: ts.pin };
    let mut handle = russh::client::connect(config, ts.addr, handler)
        .await
        .unwrap();
    let res = handle.authenticate_password("pair", secret).await.unwrap();
    assert!(matches!(res, AuthResult::Success));
    handle
}

async fn raw_control(handle: &Handle<TestClientHandler>) -> ChannelStream<Msg> {
    let channel = handle.channel_open_session().await.unwrap();
    channel
        .request_subsystem(true, "st-control/1")
        .await
        .unwrap();
    channel.into_stream()
}

#[derive(Debug)]
struct PinVerifier {
    expected_sha256: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinVerifier {
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

async fn quic_send(ts: &TestServer, grant: &UploadGrant, body: &[u8]) -> Option<Vec<u8>> {
    let provider = Arc::new(aws_lc_rs::default_provider());
    let verifier = Arc::new(PinVerifier {
        expected_sha256: grant.cert_sha256,
        provider: Arc::clone(&provider),
    });

    let mut client_crypto = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .ok()?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    client_crypto.alpn_protocols = vec![b"simply-transfer".to_vec()];

    let quic_client_config =
        quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto).ok()?;
    let mut endpoint =
        quinn::Endpoint::client(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)).ok()?;
    endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(quic_client_config)));

    let target = SocketAddr::new(ts.addr.ip(), grant.udp_port);
    let connecting = endpoint.connect(target, "localhost").ok()?;
    let connection = tokio::time::timeout(Duration::from_secs(5), connecting)
        .await
        .ok()?
        .ok()?;

    let (mut send, mut recv) = tokio::time::timeout(Duration::from_secs(5), connection.open_bi())
        .await
        .ok()?
        .ok()?;

    send.write_all(&grant.ticket).await.ok()?;
    send.write_all(body).await.ok()?;
    send.finish().ok()?;

    let data = tokio::time::timeout(Duration::from_secs(5), recv.read_to_end(4096))
        .await
        .ok()?
        .ok()?;

    connection.close(0u32.into(), b"done");
    if data.is_empty() { None } else { Some(data) }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_host_pin_is_rejected() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let ts = spawn_server().await;
        let (key, _fp) = client_identity();
        ts.trust.add(key.public_key(), "testuser");

        let other_host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let wrong_pin = HostPin::of(other_host_key.public_key());

        let res = LinkClient::connect(ts.addr, wrong_pin, key).await;
        assert!(matches!(res, Err(LinkError::HostKeyMismatch)));
    })
    .await
    .expect("test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn untrusted_key_is_rejected() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let ts = spawn_server().await;
        let (key, _fp) = client_identity();

        let res = LinkClient::connect(ts.addr, ts.pin, key).await;
        assert!(matches!(res, Err(LinkError::Auth(_))));
    })
    .await
    .expect("test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_with_wrong_secret_fails() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let ts = spawn_server().await;
        let _secret = ts.trust.issue_pair_secret();
        let (key, _fp) = client_identity();

        let res = LinkClient::pair(ts.addr, ts.pin, &"0".repeat(32), &key, "testuser").await;
        assert!(matches!(res, Err(LinkError::Auth(_))));
        assert!(!ts.trust.is_trusted(key.public_key()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_secret_is_single_use() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let ts = spawn_server().await;
        let secret = ts.trust.issue_pair_secret();
        let (key1, _fp1) = client_identity();
        let (key2, _fp2) = client_identity();

        LinkClient::pair(ts.addr, ts.pin, &secret, &key1, "testuser")
            .await
            .unwrap();
        assert!(ts.trust.is_trusted(key1.public_key()));

        let res2 = LinkClient::pair(ts.addr, ts.pin, &secret, &key2, "testuser").await;
        assert!(matches!(res2, Err(LinkError::Auth(_))));
        assert!(!ts.trust.is_trusted(key2.public_key()));
    })
    .await
    .expect("test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pair_then_connect_succeeds() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let ts = spawn_server().await;
        let secret = ts.trust.issue_pair_secret();
        let (key, fp) = client_identity();

        LinkClient::pair(ts.addr, ts.pin, &secret, &key, "testuser")
            .await
            .unwrap();
        grant(&ts, &fp, true, true);

        let client = LinkClient::connect(ts.addr, ts.pin, key).await.unwrap();
        let roots = client.roots().await.unwrap();
        assert_eq!(
            roots,
            vec![ShareRootInfo {
                id: "share".to_string(),
                read: true,
                write: true,
            }]
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairing_session_cannot_list() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let ts = spawn_server().await;
        let secret = ts.trust.issue_pair_secret();

        let handle = raw_session_pair(&ts, &secret).await;
        let mut stream = raw_control(&handle).await;

        let req = Request::List {
            root_id: "share".to_string(),
            rel_path: String::new(),
        };
        write_frame(&mut stream, &req).await.unwrap();
        let resp: Response = read_frame(&mut stream).await.unwrap();
        match resp {
            Response::Error { code, .. } => assert_eq!(code, "pairing_only"),
            other => panic!("expected Response::Error with pairing_only, got {other:?}"),
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_request_is_refused() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let ts = spawn_server().await;
        let (key, _fp) = client_identity();
        ts.trust.add(key.public_key(), "testuser");

        let handle = raw_session_pubkey(&ts, &key).await;
        let mut channel = handle.channel_open_session().await.unwrap();
        let _ = channel.exec(true, "id").await;

        let mut messages = Vec::new();
        let collect = async {
            while let Some(msg) = channel.wait().await {
                let is_close = matches!(msg, ChannelMsg::Close);
                messages.push(msg);
                if is_close {
                    break;
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(3), collect).await;

        for msg in &messages {
            assert!(
                !matches!(
                    msg,
                    ChannelMsg::Data { .. }
                        | ChannelMsg::ExtendedData { .. }
                        | ChannelMsg::Success
                        | ChannelMsg::ExitStatus { exit_status: 0 }
                ),
                "unexpected channel message on exec_request: {msg:?}"
            );
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn traversal_is_rejected() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let ts = spawn_server().await;
        let (key, fp) = client_identity();
        ts.trust.add(key.public_key(), "testuser");
        grant(&ts, &fp, true, true);

        let client = LinkClient::connect(ts.addr, ts.pin, key).await.unwrap();

        let res1 = client.list("share", "../").await;
        assert!(matches!(res1, Err(LinkError::Protocol(_))));

        let res2 = client.list("share", "/etc").await;
        assert!(matches!(res2, Err(LinkError::Protocol(_))));

        let local_tmp = tempfile::tempdir().unwrap();
        let local_file = local_tmp.path().join("x.bin");
        std::fs::write(&local_file, b"hello").unwrap();
        let (_tx, rx) = watch::channel(Control::Run);

        let res3 = client
            .upload(&local_file, "share", "../x.bin", 0, None, rx)
            .await;
        assert!(res3.is_err());
    })
    .await
    .expect("test timed out");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn symlink_escape_is_rejected() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let ts = spawn_server().await;
        let (key, fp) = client_identity();
        ts.trust.add(key.public_key(), "testuser");
        grant(&ts, &fp, true, true);

        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), b"secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), ts.root.path().join("link")).unwrap();

        let client = LinkClient::connect(ts.addr, ts.pin, key).await.unwrap();
        let res = client.list("share", "link").await;
        assert!(matches!(res, Err(LinkError::Protocol(_))));
    })
    .await
    .expect("test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_roundtrip_hash_matches() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let ts = spawn_server().await;
        let (key, fp) = client_identity();
        ts.trust.add(key.public_key(), "testuser");
        grant(&ts, &fp, true, true);

        let size: usize = 1024 * 1024;
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let expected_sha = sha256_hex(&data);

        let local_tmp = tempfile::tempdir().unwrap();
        let local_file = local_tmp.path().join("src.bin");
        std::fs::write(&local_file, &data).unwrap();

        let client = LinkClient::connect(ts.addr, ts.pin, key).await.unwrap();
        let (_tx, rx) = watch::channel(Control::Run);
        let ack = client
            .upload(&local_file, "share", "up.bin", 0, None, rx)
            .await
            .unwrap();

        assert_eq!(ack.bytes_written, size as u64);
        assert_eq!(ack.sha256, expected_sha);

        let remote_bytes = std::fs::read(ts.root.path().join("up.bin")).unwrap();
        assert_eq!(remote_bytes, data);
    })
    .await
    .expect("test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_resume_from_offset() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let ts = spawn_server().await;
        let (key, fp) = client_identity();
        ts.trust.add(key.public_key(), "testuser");
        grant(&ts, &fp, true, true);

        let size: usize = 1024 * 1024;
        let offset: usize = 300_000;
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let expected_sha = sha256_hex(&data);

        let local_tmp = tempfile::tempdir().unwrap();
        let local_file = local_tmp.path().join("src.bin");
        std::fs::write(&local_file, &data).unwrap();

        let dest_part = ts.root.path().join("part.bin");
        std::fs::write(&dest_part, &data[..offset]).unwrap();

        let client = LinkClient::connect(ts.addr, ts.pin, key).await.unwrap();
        let (_tx, rx) = watch::channel(Control::Run);
        let ack = client
            .upload(&local_file, "share", "part.bin", offset as u64, None, rx)
            .await
            .unwrap();

        assert_eq!(ack.bytes_written, (size - offset) as u64);
        assert_eq!(ack.sha256, expected_sha);

        let remote_bytes = std::fs::read(&dest_part).unwrap();
        assert_eq!(remote_bytes, data);
    })
    .await
    .expect("test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ticket_replay_is_rejected() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let ts = spawn_server().await;
        let (key, fp) = client_identity();
        ts.trust.add(key.public_key(), "testuser");
        grant(&ts, &fp, true, true);

        let handle = raw_session_pubkey(&ts, &key).await;
        let mut stream = raw_control(&handle).await;

        let req = Request::BeginUpload {
            root_id: "share".to_string(),
            rel_path: "r.bin".to_string(),
            offset: 0,
            len: 4,
        };
        write_frame(&mut stream, &req).await.unwrap();
        let resp: Response = read_frame(&mut stream).await.unwrap();
        let grant = match resp {
            Response::UploadGrant { grant } => grant,
            other => panic!("expected UploadGrant, got {other:?}"),
        };

        let first = quic_send(&ts, &grant, b"abcd").await.unwrap();
        let first_str = String::from_utf8(first).unwrap();
        assert!(
            first_str.contains("\"bytes_written\":4"),
            "unexpected ACK: {first_str}"
        );

        let second = quic_send(&ts, &grant, b"abcd").await;
        assert!(second.is_none(), "expected second quic_send to return None");
    })
    .await
    .expect("test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_to_read_only_root_is_rejected() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let ts = spawn_server().await;
        let (key, fp) = client_identity();
        ts.trust.add(key.public_key(), "testuser");
        grant(&ts, &fp, true, false);

        let client = LinkClient::connect(ts.addr, ts.pin, key).await.unwrap();

        let mkdir_res = client.mkdir("share", "d").await;
        assert!(matches!(mkdir_res, Err(LinkError::Protocol(_))));

        let local_tmp = tempfile::tempdir().unwrap();
        let local_file = local_tmp.path().join("f.bin");
        std::fs::write(&local_file, b"data").unwrap();
        let (_tx, rx) = watch::channel(Control::Run);

        let upload_res = client
            .upload(&local_file, "share", "f.bin", 0, None, rx)
            .await;
        assert!(matches!(upload_res, Err(LinkError::Protocol(_))));

        let entries: Vec<_> = std::fs::read_dir(ts.root.path()).unwrap().collect();
        assert!(entries.is_empty());
    })
    .await
    .expect("test timed out");
}
