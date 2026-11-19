use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use russh::keys::{HashAlg, PrivateKey, PublicKey};
use russh::server::{Auth, ChannelOpenHandle, Config, Handler, Msg, Session};
use russh::{Channel, ChannelId, Pty};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

use crate::LinkError;
use crate::data::DataPlane;
use crate::paths::{Access, SharePolicy};
use crate::protocol::{
    EntryInfo, Request, Response, ShareRootInfo, VolumeEntry, read_frame, write_frame,
};
use crate::trust::{TrustStore, parse_public_key};

pub struct LinkServer {
    host_key: Option<PrivateKey>,
    trust: Option<Arc<dyn TrustStore>>,
    shares: Arc<RwLock<SharePolicy>>,
    bind_addr: Option<SocketAddr>,
}

impl Default for LinkServer {
    fn default() -> Self {
        Self::new()
    }
}

impl LinkServer {
    pub fn new() -> Self {
        Self {
            host_key: None,
            trust: None,
            shares: Arc::new(RwLock::new(SharePolicy::new())),
            bind_addr: None,
        }
    }

    pub fn builder() -> Self {
        Self::new()
    }

    pub fn host_key(mut self, key: PrivateKey) -> Self {
        self.host_key = Some(key);
        self
    }

    pub fn trust(mut self, trust: Arc<dyn TrustStore>) -> Self {
        self.trust = Some(trust);
        self
    }

    pub fn shares(mut self, shares: Arc<RwLock<SharePolicy>>) -> Self {
        self.shares = shares;
        self
    }

    pub fn bind(mut self, addr: SocketAddr) -> Self {
        self.bind_addr = Some(addr);
        self
    }

    pub async fn serve(self) -> Result<(), LinkError> {
        let addr = self
            .bind_addr
            .ok_or_else(|| LinkError::Protocol("missing bind address".to_string()))?;
        let listener = TcpListener::bind(addr).await?;
        self.serve_on(listener).await
    }

    pub async fn serve_on(self, listener: TcpListener) -> Result<(), LinkError> {
        let host_key = self
            .host_key
            .ok_or_else(|| LinkError::Protocol("missing host key".to_string()))?;
        let trust = self
            .trust
            .ok_or_else(|| LinkError::Protocol("missing trust store".to_string()))?;
        let shares = self.shares;
        let quic_addr = self.bind_addr.unwrap_or(listener.local_addr()?);
        let data = DataPlane::bind(quic_addr)?;

        let config = Arc::new(Config {
            keys: vec![host_key],
            inactivity_timeout: Some(Duration::from_secs(300)),
            auth_rejection_time: Duration::from_secs(1),
            max_auth_attempts: 3,
            ..Default::default()
        });

        loop {
            let (socket, _peer_addr) = listener.accept().await?;
            let config = Arc::clone(&config);
            let handler = ClientSessionHandler {
                trust: Arc::clone(&trust),
                shares: Arc::clone(&shares),
                data: Arc::clone(&data),
                peer_fp: None,
                pairing_only: false,
                channels: HashMap::new(),
            };
            tokio::spawn(async move {
                if let Ok(session) = russh::server::run_stream(config, socket, handler).await {
                    let _ = session.await;
                }
            });
        }
    }
}

pub struct RequestContext {
    pub trust: Arc<dyn TrustStore>,
    pub shares: Arc<RwLock<SharePolicy>>,
    pub(crate) data: Option<Arc<DataPlane>>,
    pub peer_fp: Option<String>,
    pub pairing_only: bool,
}

pub fn handle_request(ctx: &RequestContext, req: Request) -> Response {
    if ctx.pairing_only {
        return match req {
            Request::Pair {
                client_pubkey,
                label,
            } => match parse_public_key(&client_pubkey) {
                Ok(pk) => {
                    ctx.trust.add(&pk, &label);
                    Response::Paired
                }
                Err(e) => Response::Error {
                    code: "invalid_key".to_string(),
                    message: e,
                },
            },
            _ => Response::Error {
                code: "pairing_only".to_string(),
                message: "session is restricted to pairing".to_string(),
            },
        };
    }

    let Some(peer_fp) = ctx.peer_fp.as_deref() else {
        return Response::Error {
            code: "unauthenticated".to_string(),
            message: "session has no authenticated peer fingerprint".to_string(),
        };
    };

    match req {
        Request::Pair { .. } => Response::Error {
            code: "already_paired".to_string(),
            message: "session is already authenticated with a public key".to_string(),
        },
        Request::Roots => {
            let Ok(policy) = ctx.shares.read() else {
                return Response::Error {
                    code: "internal".to_string(),
                    message: "share policy lock poisoned".to_string(),
                };
            };
            let roots = policy
                .roots_for(peer_fp)
                .iter()
                .map(|r| ShareRootInfo {
                    id: r.id.clone(),
                    read: r.read,
                    write: r.write,
                })
                .collect();
            Response::Roots { roots }
        }
        Request::List { root_id, rel_path } => {
            let resolved = match resolve_with_policy(
                &ctx.shares,
                peer_fp,
                &root_id,
                &rel_path,
                Access::Read,
            ) {
                Ok(p) => p,
                Err(resp) => return resp,
            };
            match list_entries(&resolved) {
                Ok(entries) => Response::List { entries },
                Err(e) => io_error_response(e),
            }
        }
        Request::Stat { root_id, rel_path } => {
            let resolved = match resolve_with_policy(
                &ctx.shares,
                peer_fp,
                &root_id,
                &rel_path,
                Access::Read,
            ) {
                Ok(p) => p,
                Err(resp) => return resp,
            };
            match fs::metadata(&resolved) {
                Ok(meta) => {
                    let name = resolved
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| root_id.clone());
                    let is_dir = meta.is_dir();
                    let size = if is_dir { 0 } else { meta.len() };
                    Response::Stat {
                        entry: EntryInfo { name, is_dir, size },
                    }
                }
                Err(e) => io_error_response(e),
            }
        }
        Request::Hash { root_id, rel_path } => {
            let resolved = match resolve_with_policy(
                &ctx.shares,
                peer_fp,
                &root_id,
                &rel_path,
                Access::Read,
            ) {
                Ok(p) => p,
                Err(resp) => return resp,
            };
            match File::open(&resolved).and_then(compute_sha256_stream) {
                Ok(sha256) => Response::Hash { sha256 },
                Err(e) => io_error_response(e),
            }
        }
        Request::Exists { root_id, rel_path } => {
            let resolved = match resolve_with_policy(
                &ctx.shares,
                peer_fp,
                &root_id,
                &rel_path,
                Access::Read,
            ) {
                Ok(p) => p,
                Err(resp) => return resp,
            };
            let exists = fs::metadata(&resolved).is_ok();
            Response::Exists { exists }
        }
        Request::Mkdir { root_id, rel_path } => {
            let resolved =
                match resolve_with_policy(&ctx.shares, peer_fp, &root_id, &rel_path, Access::Write)
                {
                    Ok(p) => p,
                    Err(resp) => return resp,
                };
            match fs::create_dir_all(&resolved) {
                Ok(()) => Response::Ok,
                Err(e) => io_error_response(e),
            }
        }
        Request::Volumes => match simply_transfer_snapshots::list_local_volumes() {
            Ok(vols) => {
                let volumes = vols
                    .into_iter()
                    .map(|v| VolumeEntry {
                        mount_point: v.mount_point.to_string_lossy().to_string(),
                        label: v.label,
                        total_bytes: v.total_bytes,
                        available_bytes: v.available_bytes,
                    })
                    .collect();
                Response::Volumes { volumes }
            }
            Err(e) => Response::Error {
                code: "volumes".to_string(),
                message: e.to_string(),
            },
        },
        Request::DiskFree { root_id } => {
            let resolved =
                match resolve_with_policy(&ctx.shares, peer_fp, &root_id, "", Access::Read) {
                    Ok(p) => p,
                    Err(resp) => return resp,
                };
            match disk_free_for_path(&resolved) {
                Some(available_bytes) => Response::DiskFree { available_bytes },
                None => Response::Error {
                    code: "disk_free".to_string(),
                    message: "unable to determine available disk space".to_string(),
                },
            }
        }
        Request::BeginUpload {
            root_id,
            rel_path,
            offset,
            len,
        } => {
            let resolved =
                match resolve_with_policy(&ctx.shares, peer_fp, &root_id, &rel_path, Access::Write)
                {
                    Ok(p) => p,
                    Err(resp) => return resp,
                };
            let Some(data) = ctx.data.as_ref() else {
                return Response::Error {
                    code: "unimplemented".to_string(),
                    message: "QUIC data plane is not available".to_string(),
                };
            };
            match data.issue(peer_fp, resolved, offset, len) {
                Ok(grant) => Response::UploadGrant { grant },
                Err(e) => Response::Error {
                    code: "upload_grant".to_string(),
                    message: e.to_string(),
                },
            }
        }
    }
}

fn resolve_with_policy(
    shares: &Arc<RwLock<SharePolicy>>,
    peer_fp: &str,
    root_id: &str,
    rel_path: &str,
    access: Access,
) -> Result<std::path::PathBuf, Response> {
    let Ok(policy) = shares.read() else {
        return Err(Response::Error {
            code: "internal".to_string(),
            message: "share policy lock poisoned".to_string(),
        });
    };
    policy
        .resolve(peer_fp, root_id, rel_path, access)
        .map_err(|e| match e {
            LinkError::Path(msg) => Response::Error {
                code: "path".to_string(),
                message: msg,
            },
            other => Response::Error {
                code: "error".to_string(),
                message: other.to_string(),
            },
        })
}

fn io_error_response(e: io::Error) -> Response {
    let code = match e.kind() {
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::PermissionDenied => "permission_denied",
        _ => "io",
    };
    Response::Error {
        code: code.to_string(),
        message: e.to_string(),
    }
}

fn list_entries(dir: &Path) -> io::Result<Vec<EntryInfo>> {
    let rd = fs::read_dir(dir)?;
    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.is_empty() || name == "." || name == ".." || name.starts_with('.') {
            continue;
        }
        let path_buf = entry.path();
        let meta = entry.metadata().or_else(|_| fs::metadata(&path_buf));
        let (is_dir, size) = match meta {
            Ok(m) => {
                let d = m.is_dir();
                (d, if d { 0 } else { m.len() })
            }
            Err(_) => {
                let d = path_buf.is_dir();
                (d, 0)
            }
        };

        let info = EntryInfo { name, is_dir, size };
        if is_dir {
            dirs.push(info);
        } else {
            files.push(info);
        }
    }

    dirs.sort_by_key(|a| a.name.to_lowercase());
    files.sort_by_key(|a| a.name.to_lowercase());
    dirs.extend(files);
    Ok(dirs)
}

fn compute_sha256_stream<R: Read>(mut reader: R) -> io::Result<String> {
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
    let mut hex_str = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(hex_str, "{b:02x}");
    }
    Ok(hex_str)
}

fn disk_free_for_path(path: &Path) -> Option<u64> {
    let mut cur = path;
    let dir = loop {
        if cur.is_dir() {
            break cur;
        }
        cur = cur.parent()?;
    };
    let vols = simply_transfer_snapshots::list_local_volumes().ok()?;
    let mount_point = simply_transfer_snapshots::resolve_owning_volume(dir, &vols)?;
    vols.into_iter()
        .find(|v| v.mount_point == mount_point)
        .map(|v| v.available_bytes)
}

struct ClientSessionHandler {
    trust: Arc<dyn TrustStore>,
    shares: Arc<RwLock<SharePolicy>>,
    data: Arc<DataPlane>,
    peer_fp: Option<String>,
    pairing_only: bool,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl Handler for ClientSessionHandler {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        _user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        if self.trust.is_trusted(public_key) {
            let fp = public_key.fingerprint(HashAlg::Sha256).to_string();
            self.peer_fp = Some(fp);
            self.pairing_only = false;
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        if user == "pair" && self.trust.consume_pair_secret(password) {
            self.pairing_only = true;
            self.peer_fp = None;
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        _data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.remove(&channel);
        let _ = session.channel_failure(channel);
        let _ = session.close(channel);
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.remove(&channel);
        let _ = session.channel_failure(channel);
        let _ = session.close(channel);
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.remove(&channel);
        let _ = session.channel_failure(channel);
        let _ = session.close(channel);
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name != "st-control/1" {
            self.channels.remove(&channel);
            let _ = session.channel_failure(channel);
            let _ = session.close(channel);
            return Ok(());
        }

        let Some(chan) = self.channels.remove(&channel) else {
            let _ = session.channel_failure(channel);
            let _ = session.close(channel);
            return Ok(());
        };

        let _ = session.channel_success(channel);

        let ctx = Arc::new(RequestContext {
            trust: Arc::clone(&self.trust),
            shares: Arc::clone(&self.shares),
            data: Some(Arc::clone(&self.data)),
            peer_fp: self.peer_fp.clone(),
            pairing_only: self.pairing_only,
        });

        tokio::spawn(async move {
            let mut stream = chan.into_stream();
            loop {
                let req = match read_frame::<_, Request>(&mut stream).await {
                    Ok(r) => r,
                    Err(LinkError::Io(ref e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                        break;
                    }
                    Err(LinkError::Protocol(msg)) => {
                        let err_resp = Response::Error {
                            code: "protocol".to_string(),
                            message: msg,
                        };
                        let _ = write_frame(&mut stream, &err_resp).await;
                        break;
                    }
                    Err(_) => break,
                };

                let ctx_clone = Arc::clone(&ctx);
                let resp = match tokio::task::spawn_blocking(move || {
                    handle_request(&ctx_clone, req)
                })
                .await
                {
                    Ok(r) => r,
                    Err(e) => Response::Error {
                        code: "internal".to_string(),
                        message: e.to_string(),
                    },
                };

                if write_frame(&mut stream, &resp).await.is_err() {
                    break;
                }
            }
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::ShareRoot;
    use crate::trust::MemoryTrustStore;

    const TEST_FP: &str = "SHA256:testpeer";

    fn make_ctx(root_path: &Path, pairing_only: bool) -> (RequestContext, Arc<MemoryTrustStore>) {
        let trust = Arc::new(MemoryTrustStore::new());
        let mut policy = SharePolicy::new();
        policy.grant(
            TEST_FP,
            ShareRoot {
                id: "share".to_string(),
                path: root_path.to_path_buf(),
                read: true,
                write: true,
            },
        );
        let ctx = RequestContext {
            trust: trust.clone(),
            shares: Arc::new(RwLock::new(policy)),
            data: None,
            peer_fp: if pairing_only {
                None
            } else {
                Some(TEST_FP.to_string())
            },
            pairing_only,
        };
        (ctx, trust)
    }

    #[test]
    fn pairing_only_session_rejects_list_and_accepts_pair() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, trust) = make_ctx(tmp.path(), true);

        let resp = handle_request(
            &ctx,
            Request::List {
                root_id: "share".to_string(),
                rel_path: String::new(),
            },
        );
        assert!(matches!(resp, Response::Error { .. }));

        let client_key = PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)
            .unwrap()
            .public_key()
            .clone();
        let openssh = client_key.to_openssh().unwrap();

        let pair_resp = handle_request(
            &ctx,
            Request::Pair {
                client_pubkey: openssh,
                label: "testuser".to_string(),
            },
        );
        assert_eq!(pair_resp, Response::Paired);
        assert!(trust.is_trusted(&client_key));
    }

    #[test]
    fn list_through_granted_root_returns_entries() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("subdir")).unwrap();
        fs::write(tmp.path().join("hello.txt"), b"world").unwrap();
        fs::write(tmp.path().join(".hidden"), b"secret").unwrap();

        let (ctx, _) = make_ctx(tmp.path(), false);

        let resp = handle_request(
            &ctx,
            Request::List {
                root_id: "share".to_string(),
                rel_path: String::new(),
            },
        );
        assert_eq!(
            resp,
            Response::List {
                entries: vec![
                    EntryInfo {
                        name: "subdir".to_string(),
                        is_dir: true,
                        size: 0,
                    },
                    EntryInfo {
                        name: "hello.txt".to_string(),
                        is_dir: false,
                        size: 5,
                    },
                ],
            }
        );
    }

    #[test]
    fn list_of_parent_traversal_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let (ctx, _) = make_ctx(tmp.path(), false);

        let resp = handle_request(
            &ctx,
            Request::List {
                root_id: "share".to_string(),
                rel_path: "../".to_string(),
            },
        );
        assert!(matches!(resp, Response::Error { .. }));
    }

    #[test]
    fn hash_of_file_returns_expected_sha256() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("hello.txt"), b"hello world").unwrap();

        let (ctx, _) = make_ctx(tmp.path(), false);

        let resp = handle_request(
            &ctx,
            Request::Hash {
                root_id: "share".to_string(),
                rel_path: "hello.txt".to_string(),
            },
        );
        assert_eq!(
            resp,
            Response::Hash {
                sha256: "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
                    .to_string(),
            }
        );
    }

    #[tokio::test]
    async fn begin_upload_issues_grant_and_respects_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut ctx, _) = make_ctx(tmp.path(), false);
        let plane = DataPlane::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let expected_port = plane.udp_port().unwrap();
        let expected_cert = plane.cert_sha256;
        ctx.data = Some(plane);

        let resp = handle_request(
            &ctx,
            Request::BeginUpload {
                root_id: "share".to_string(),
                rel_path: "upload.bin".to_string(),
                offset: 0,
                len: 1024,
            },
        );
        match resp {
            Response::UploadGrant { grant } => {
                assert_eq!(grant.udp_port, expected_port);
                assert_eq!(grant.cert_sha256, expected_cert);
            }
            other => panic!("expected UploadGrant, got {other:?}"),
        }

        let bad_resp = handle_request(
            &ctx,
            Request::BeginUpload {
                root_id: "share".to_string(),
                rel_path: "../escape.bin".to_string(),
                offset: 0,
                len: 1024,
            },
        );
        assert!(matches!(bad_resp, Response::Error { .. }));
    }
}
