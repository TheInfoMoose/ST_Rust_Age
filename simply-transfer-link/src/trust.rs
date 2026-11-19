use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use russh::keys::{HashAlg, PublicKey};
use sha2::{Digest, Sha256};

const PAIR_SECRET_TTL: Duration = Duration::from_secs(600);

pub trait TrustStore: Send + Sync {
    fn is_trusted(&self, key: &PublicKey) -> bool;
    fn add(&self, key: &PublicKey, label: &str);
    fn issue_pair_secret(&self) -> String;
    fn consume_pair_secret(&self, secret: &str) -> bool;
}

#[derive(Default)]
struct PendingSecrets {
    entries: Mutex<HashMap<String, Instant>>,
}

impl PendingSecrets {
    fn issue(&self) -> String {
        let bytes = rand::random::<[u8; 16]>();
        let mut secret = String::with_capacity(32);
        for b in bytes {
            use std::fmt::Write as _;
            let _ = write!(secret, "{b:02x}");
        }
        let hash = sha256_hex(secret.as_bytes());
        let now = Instant::now();
        let expiry = now + PAIR_SECRET_TTL;
        if let Ok(mut pending) = self.entries.lock() {
            pending.retain(|_, exp| *exp > now);
            pending.insert(hash, expiry);
        }
        secret
    }

    fn consume(&self, secret: &str) -> bool {
        let now = Instant::now();
        let hash = sha256_hex(secret.as_bytes());
        if let Ok(mut pending) = self.entries.lock() {
            pending.retain(|_, exp| *exp > now);
            pending.remove(&hash).is_some_and(|exp| exp > now)
        } else {
            false
        }
    }

    #[cfg(test)]
    fn insert_expired(&self, secret: &str) {
        let hash = sha256_hex(secret.as_bytes());
        let expired = Instant::now() - Duration::from_secs(1);
        self.entries.lock().unwrap().insert(hash, expired);
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[derive(Clone)]
struct TrustedEntry {
    key: PublicKey,
    label: String,
}

#[derive(Default)]
pub struct MemoryTrustStore {
    keys: RwLock<HashMap<String, TrustedEntry>>,
    pending: PendingSecrets,
}

impl MemoryTrustStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl TrustStore for MemoryTrustStore {
    fn is_trusted(&self, key: &PublicKey) -> bool {
        let fp = key.fingerprint(HashAlg::Sha256).to_string();
        let raw_fp = fp.strip_prefix("SHA256:").unwrap_or(&fp);
        if let Ok(guard) = self.keys.read() {
            guard.contains_key(&fp)
                || guard.contains_key(raw_fp)
                || guard.values().any(|e| &e.key == key)
        } else {
            false
        }
    }

    fn add(&self, key: &PublicKey, label: &str) {
        let fp = key.fingerprint(HashAlg::Sha256).to_string();
        if let Ok(mut guard) = self.keys.write() {
            guard.insert(
                fp,
                TrustedEntry {
                    key: key.clone(),
                    label: label.to_string(),
                },
            );
        }
    }

    fn issue_pair_secret(&self) -> String {
        self.pending.issue()
    }

    fn consume_pair_secret(&self, secret: &str) -> bool {
        self.pending.consume(secret)
    }
}

pub struct FileTrustStore {
    path: PathBuf,
    keys: RwLock<HashMap<String, TrustedEntry>>,
    pending: PendingSecrets,
}

impl FileTrustStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let loaded = load_keys_file(&path);
        Self {
            path,
            keys: RwLock::new(loaded),
            pending: PendingSecrets::default(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl TrustStore for FileTrustStore {
    fn is_trusted(&self, key: &PublicKey) -> bool {
        let fp = key.fingerprint(HashAlg::Sha256).to_string();
        let raw_fp = fp.strip_prefix("SHA256:").unwrap_or(&fp);
        if let Ok(guard) = self.keys.read() {
            guard.contains_key(&fp)
                || guard.contains_key(raw_fp)
                || guard.values().any(|e| &e.key == key)
        } else {
            false
        }
    }

    fn add(&self, key: &PublicKey, label: &str) {
        let fp = key.fingerprint(HashAlg::Sha256).to_string();
        let snapshot = if let Ok(mut guard) = self.keys.write() {
            guard.insert(
                fp,
                TrustedEntry {
                    key: key.clone(),
                    label: label.to_string(),
                },
            );
            guard.clone()
        } else {
            return;
        };

        let mut map = HashMap::with_capacity(snapshot.len());
        for entry in snapshot.values() {
            let canonical_fp = entry.key.fingerprint(HashAlg::Sha256).to_string();
            if let Ok(mut openssh) = entry.key.to_openssh() {
                let trimmed_label = entry.label.trim();
                if !trimmed_label.is_empty() && !openssh.ends_with(trimmed_label) {
                    openssh.push(' ');
                    openssh.push_str(trimmed_label);
                }
                map.insert(canonical_fp, openssh);
            }
        }
        write_keys_file(&self.path, &map);
    }

    fn issue_pair_secret(&self) -> String {
        self.pending.issue()
    }

    fn consume_pair_secret(&self, secret: &str) -> bool {
        self.pending.consume(secret)
    }
}

fn load_keys_file(path: &Path) -> HashMap<String, TrustedEntry> {
    let Ok(file) = File::open(path) else {
        return HashMap::new();
    };
    let reader = BufReader::new(file);
    let Ok(raw_map) = serde_json::from_reader::<_, HashMap<String, String>>(reader) else {
        return HashMap::new();
    };
    let mut entries = HashMap::new();
    for (_stored_fp, key_str) in raw_map {
        if let Ok(pk) = parse_public_key(&key_str) {
            let canonical_fp = pk.fingerprint(HashAlg::Sha256).to_string();
            let comment = pk.comment().to_string();
            entries.insert(
                canonical_fp,
                TrustedEntry {
                    key: pk,
                    label: comment,
                },
            );
        }
    }
    entries
}

fn write_keys_file(path: &Path, keys_map: &HashMap<String, String>) {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        let _ = fs::create_dir_all(parent);
        #[cfg(unix)]
        {
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }
    let tmp_path = path.with_extension("json.tmp");
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        opts.mode(0o600);
    }
    if let Ok(file) = opts.open(&tmp_path) {
        let mut writer = BufWriter::new(file);
        if serde_json::to_writer_pretty(&mut writer, keys_map).is_ok() && writer.flush().is_ok() {
            drop(writer);
            let _ = fs::rename(&tmp_path, path);
        }
    }
}

pub fn parse_public_key(key_str: &str) -> Result<PublicKey, String> {
    let clean = key_str.split(';').next().unwrap_or(key_str).trim();
    PublicKey::from_openssh(clean)
        .or_else(|_| PublicKey::from_openssh(&format!("ssh-ed25519 {clean}")))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::{Algorithm, PrivateKey};

    fn random_pubkey() -> PublicKey {
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .unwrap()
            .public_key()
            .clone()
    }

    #[test]
    fn memory_trust_store_trust_and_pair_secrets() {
        let store = MemoryTrustStore::new();
        let key = random_pubkey();
        assert!(!store.is_trusted(&key));

        store.add(&key, "testuser-laptop");
        assert!(store.is_trusted(&key));

        let secret = store.issue_pair_secret();
        assert_eq!(secret.len(), 32);
        assert!(store.consume_pair_secret(&secret));
        assert!(!store.consume_pair_secret(&secret));
        assert!(!store.consume_pair_secret("unknown"));

        store
            .pending
            .insert_expired("0123456789abcdef0123456789abcdef");
        assert!(!store.consume_pair_secret("0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn file_trust_store_persists_with_0600_permissions() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("trust.json");

        let key = random_pubkey();
        {
            let store = FileTrustStore::new(&file_path);
            assert!(!store.is_trusted(&key));
            store.add(&key, "peer-a");
            assert!(store.is_trusted(&key));
        }

        #[cfg(unix)]
        {
            let mode = fs::metadata(&file_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        let reloaded = FileTrustStore::new(&file_path);
        assert!(reloaded.is_trusted(&key));
    }

    #[test]
    fn parse_public_key_accepts_openssh_and_raw_base64() {
        let key = random_pubkey();
        let openssh = key.to_openssh().unwrap();
        let raw_b64 = openssh.strip_prefix("ssh-ed25519 ").unwrap();

        assert_eq!(parse_public_key(&openssh).unwrap(), key);
        assert_eq!(parse_public_key(raw_b64).unwrap(), key);
        assert_eq!(parse_public_key(&format!("{openssh};extra")).unwrap(), key);
    }
}
