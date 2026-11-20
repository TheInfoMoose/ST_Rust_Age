use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::SocketAddr;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use russh::keys::{Algorithm, HashAlg, PrivateKey, PublicKey, ssh_key::LineEnding};
use sha2::{Digest, Sha256};
use simply_transfer_link::{
    LinkError,
    client::{Control, HostPin, LinkClient},
    paths::{SharePolicy, ShareRoot},
    server::LinkServer,
    trust::{FileTrustStore, TrustStore},
};

fn fail(what: &str) -> ! {
    eprintln!("FAIL: {what}");
    std::process::exit(1);
}

fn find_arg(args: &[String], flag: &str) -> String {
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        if a == flag {
            match iter.next() {
                Some(v) => return v.clone(),
                None => fail(&format!("missing value for {flag}")),
            }
        }
    }
    fail(&format!("missing required argument {flag}"));
}

fn write_0600(path: &Path, data: &[u8]) {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        opts.mode(0o600);
    }
    let mut file = match opts.open(path) {
        Ok(f) => f,
        Err(e) => fail(&format!("open {}: {e}", path.display())),
    };
    if let Err(e) = file.write_all(data) {
        fail(&format!("write {}: {e}", path.display()));
    }
    if let Err(e) = file.flush() {
        fail(&format!("flush {}: {e}", path.display()));
    }
}

fn write_private_key(path: &Path, key: &PrivateKey) {
    let openssh = match key.to_openssh(LineEnding::LF) {
        Ok(s) => s,
        Err(e) => fail(&format!("encode private key: {e}")),
    };
    write_0600(path, openssh.as_bytes());
}

fn load_private_key(path: &Path) -> PrivateKey {
    let text = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => fail(&format!("read private key {}: {e}", path.display())),
    };
    match PrivateKey::from_openssh(&text) {
        Ok(k) => k,
        Err(e) => fail(&format!("parse private key {}: {e}", path.display())),
    }
}

fn load_public_key(path: &Path) -> PublicKey {
    let text = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => fail(&format!("read public key {}: {e}", path.display())),
    };
    match PublicKey::from_openssh(text.trim()) {
        Ok(k) => k,
        Err(e) => fail(&format!("parse public key {}: {e}", path.display())),
    }
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

fn run_keygen(args: &[String]) {
    let out_dir = PathBuf::from(find_arg(args, "--out"));
    if let Err(e) = fs::create_dir_all(&out_dir) {
        fail(&format!("create out dir {}: {e}", out_dir.display()));
    }
    let key = match PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519) {
        Ok(k) => k,
        Err(e) => fail(&format!("generate Ed25519 key: {e}")),
    };
    write_private_key(&out_dir.join("client_key"), &key);

    let pub_line = match key.public_key().to_openssh() {
        Ok(mut s) => {
            s.push('\n');
            s
        }
        Err(e) => fail(&format!("encode public key: {e}")),
    };
    let pub_path = out_dir.join("client.pub");
    if let Err(e) = fs::write(&pub_path, pub_line) {
        fail(&format!("write {}: {e}", pub_path.display()));
    }
}

async fn run_serve(args: &[String]) {
    let bind_str = find_arg(args, "--bind");
    let state_dir = PathBuf::from(find_arg(args, "--state"));
    let share_dir = PathBuf::from(find_arg(args, "--share"));
    let client_pub_path = PathBuf::from(find_arg(args, "--client-pub"));

    let addr: SocketAddr = match bind_str.parse() {
        Ok(a) => a,
        Err(e) => fail(&format!("parse bind addr '{bind_str}': {e}")),
    };

    if let Err(e) = fs::create_dir_all(&state_dir) {
        fail(&format!("create state dir {}: {e}", state_dir.display()));
    }
    if let Err(e) = fs::create_dir_all(&share_dir) {
        fail(&format!("create share dir {}: {e}", share_dir.display()));
    }

    let host_key_path = state_dir.join("host_key");
    let host_key = if host_key_path.exists() {
        load_private_key(&host_key_path)
    } else {
        let k = match PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519) {
            Ok(k) => k,
            Err(e) => fail(&format!("generate host key: {e}")),
        };
        write_private_key(&host_key_path, &k);
        k
    };

    let trust = Arc::new(FileTrustStore::new(state_dir.join("trust.json")));

    let client_pub = load_public_key(&client_pub_path);
    let fp = client_pub.fingerprint(HashAlg::Sha256).to_string();
    let mut policy = SharePolicy::new();
    policy.grant(
        &fp,
        ShareRoot {
            id: "share".to_string(),
            path: share_dir,
            read: true,
            write: true,
        },
    );

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => fail(&format!("bind {addr}: {e}")),
    };

    let pin_hex = HostPin::of(host_key.public_key()).to_hex();
    let pair_secret = trust.issue_pair_secret();

    let host_pin_path = state_dir.join("host_pin");
    if let Err(e) = fs::write(&host_pin_path, &pin_hex) {
        fail(&format!("write {}: {e}", host_pin_path.display()));
    }
    let pair_secret_path = state_dir.join("pair_secret");
    if let Err(e) = fs::write(&pair_secret_path, &pair_secret) {
        fail(&format!("write {}: {e}", pair_secret_path.display()));
    }
    let ready_path = state_dir.join("ready");
    if let Err(e) = fs::write(&ready_path, b"") {
        fail(&format!("write {}: {e}", ready_path.display()));
    }

    let trust_dyn: Arc<dyn TrustStore> = trust;
    if let Err(e) = LinkServer::builder()
        .host_key(host_key)
        .trust(trust_dyn)
        .shares(Arc::new(RwLock::new(policy)))
        .bind(addr)
        .serve_on(listener)
        .await
    {
        fail(&format!("serve: {e}"));
    }
}

async fn run_client(args: &[String]) {
    let server_str = find_arg(args, "--server");
    let state_dir = PathBuf::from(find_arg(args, "--state"));
    let data_dir = PathBuf::from(find_arg(args, "--data"));

    if let Err(e) = fs::create_dir_all(&data_dir) {
        fail(&format!("create data dir {}: {e}", data_dir.display()));
    }

    let mut addrs = match tokio::net::lookup_host(&server_str).await {
        Ok(it) => it,
        Err(e) => fail(&format!("lookup_host '{server_str}': {e}")),
    };
    let addr = match addrs.next() {
        Some(a) => a,
        None => fail(&format!("lookup_host '{server_str}' returned no addresses")),
    };

    let pin_text = match fs::read_to_string(state_dir.join("host_pin")) {
        Ok(s) => s,
        Err(e) => fail(&format!("read host_pin: {e}")),
    };
    let pin = match HostPin::from_hex(pin_text.trim()) {
        Ok(p) => p,
        Err(e) => fail(&format!("parse host_pin: {e}")),
    };

    let pair_secret = match fs::read_to_string(state_dir.join("pair_secret")) {
        Ok(s) => s.trim().to_string(),
        Err(e) => fail(&format!("read pair_secret: {e}")),
    };

    let client_key = load_private_key(&state_dir.join("client_key"));

    // 1. wrong_pin
    let mut bad_bytes = pin.0;
    bad_bytes[0] ^= 0xff;
    let wrong_pin = HostPin(bad_bytes);
    match LinkClient::connect(addr, wrong_pin, client_key.clone()).await {
        Err(LinkError::HostKeyMismatch) => println!("PASS: wrong_pin"),
        Ok(_) => fail("wrong_pin: expected HostKeyMismatch, got Ok"),
        Err(e) => fail(&format!("wrong_pin: expected HostKeyMismatch, got {e}")),
    }

    // 2. wrong_secret
    let wrong_secret = "00".repeat(16);
    match LinkClient::pair(addr, pin, &wrong_secret, &client_key, "two-host").await {
        Err(_) => println!("PASS: wrong_secret"),
        Ok(()) => fail("wrong_secret: expected Err, got Ok"),
    }

    // 3. pair
    match LinkClient::pair(addr, pin, &pair_secret, &client_key, "two-host").await {
        Ok(()) => println!("PASS: pair"),
        Err(e) => fail(&format!("pair: {e}")),
    }

    // 4. secret_single_use
    match LinkClient::pair(addr, pin, &pair_secret, &client_key, "two-host").await {
        Err(_) => println!("PASS: secret_single_use"),
        Ok(()) => fail("secret_single_use: expected Err, got Ok"),
    }

    // 5. connect
    let client = match LinkClient::connect(addr, pin, client_key).await {
        Ok(c) => {
            println!("PASS: connect");
            c
        }
        Err(e) => fail(&format!("connect: {e}")),
    };

    // 6. roots
    let roots = match client.roots().await {
        Ok(r) => r,
        Err(e) => fail(&format!("roots: {e}")),
    };
    if roots.iter().any(|r| r.id == "share" && r.write) {
        println!("PASS: roots");
    } else {
        fail(&format!(
            "roots: missing writable 'share' root in {roots:?}"
        ));
    }

    // 7. mkdir
    match client.mkdir("share", "inbox").await {
        Ok(()) => println!("PASS: mkdir"),
        Err(e) => fail(&format!("mkdir: {e}")),
    }

    // 8. traversal
    match client.list("share", "../").await {
        Err(_) => println!("PASS: traversal"),
        Ok(entries) => fail(&format!("traversal: expected Err, got {entries:?}")),
    }

    // 9. upload
    let payload_size: usize = 8 * 1024 * 1024;
    let payload: Vec<u8> = (0..payload_size).map(|i| (i % 251) as u8).collect();
    let payload_sha = sha256_hex(&payload);
    let payload_path = data_dir.join("payload.bin");
    if let Err(e) = fs::write(&payload_path, &payload) {
        fail(&format!("write {}: {e}", payload_path.display()));
    }

    let (_tx1, rx1) = tokio::sync::watch::channel(Control::Run);
    let ack1 = match client
        .upload(&payload_path, "share", "inbox/payload.bin", 0, None, rx1)
        .await
    {
        Ok(a) => a,
        Err(e) => fail(&format!("upload: {e}")),
    };
    if ack1.bytes_written != payload_size as u64 {
        fail(&format!(
            "upload: bytes_written {} != {}",
            ack1.bytes_written, payload_size
        ));
    }
    if ack1.sha256 != payload_sha {
        fail(&format!(
            "upload: sha256 {} != {}",
            ack1.sha256, payload_sha
        ));
    }
    println!("PASS: upload");

    // 10. resume
    let part_size: usize = 3 * 1024 * 1024;
    let part_path = data_dir.join("part.bin");
    if let Err(e) = fs::write(&part_path, &payload[..part_size]) {
        fail(&format!("write {}: {e}", part_path.display()));
    }

    let (_tx2, rx2) = tokio::sync::watch::channel(Control::Run);
    if let Err(e) = client
        .upload(&part_path, "share", "inbox/resume.bin", 0, None, rx2)
        .await
    {
        fail(&format!("resume (part upload): {e}"));
    }

    let (_tx3, rx3) = tokio::sync::watch::channel(Control::Run);
    let ack3 = match client
        .upload(
            &payload_path,
            "share",
            "inbox/resume.bin",
            part_size as u64,
            None,
            rx3,
        )
        .await
    {
        Ok(a) => a,
        Err(e) => fail(&format!("resume (tail upload): {e}")),
    };
    if ack3.sha256 != payload_sha {
        fail(&format!(
            "resume: sha256 {} != {}",
            ack3.sha256, payload_sha
        ));
    }
    println!("PASS: resume");

    println!("ALL PASS");
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(subcmd) = args.get(1).map(String::as_str) else {
        fail("usage: link_node <keygen|serve|client> [options]");
    };
    let rest = &args[2..];
    match subcmd {
        "keygen" => run_keygen(rest),
        "serve" => run_serve(rest).await,
        "client" => run_client(rest).await,
        other => fail(&format!("unknown subcommand '{other}'")),
    }
}
