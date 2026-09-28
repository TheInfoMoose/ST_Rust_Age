use simply_transfer_core::engine::{
    ControlSignal, FileTransferStatus, TransferEngine, TransferEvent,
};
use simply_transfer_core::ssh::MockSshClient;
use simply_transfer_snapshots::FallbackSnapshotDriver;
use slint::Model;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};
slint::include_modules!();

mod network;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct SavedConnection {
    name: String,
    host: String,
    state: String,
    transfer_rate: String,
    duration: String,
    eta: String,
    transfer_type: String,
    token: String,
}

fn load_connections() -> Vec<SavedConnection> {
    if let Ok(data) = std::fs::read_to_string("connections.json")
        && let Ok(conns) = serde_json::from_str(&data)
    {
        return conns;
    }
    Vec::new()
}

fn save_connections(conns: &[ConnectionItem]) {
    let saved: Vec<SavedConnection> = conns
        .iter()
        .map(|c| SavedConnection {
            name: c.name.to_string(),
            host: c.host.to_string(),
            state: c.state.to_string(),
            transfer_rate: c.transfer_rate.to_string(),
            duration: c.duration.to_string(),
            eta: c.eta.to_string(),
            transfer_type: c.transfer_type.to_string(),
            token: c.token.to_string(),
        })
        .collect();
    if let Ok(json) = serde_json::to_string_pretty(&saved) {
        let _ = std::fs::write("connections.json", json);
    }
}

fn log_event(log_type: &str, message: &str) {
    use std::io::Write;
    let dir = std::path::Path::new("logs");
    if !dir.exists() {
        let _ = std::fs::create_dir_all(dir);
    }
    let filepath = dir.join(format!("{}.log", log_type));
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(filepath)
    {
        let _ = writeln!(file, "[{}] {}", timestamp, message);
    }
}

fn open_log(filename: &str) {
    let path = format!("logs/{}", filename);
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", &path])
        .spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(&path).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
}

#[tokio::main]
#[rustfmt::skip]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let ui = MainWindow::new()?;

    let loaded = load_connections();
    let slint_conns: Vec<ConnectionItem> = loaded.into_iter().map(|c| ConnectionItem {
        name: c.name.into(),
        host: c.host.into(),
        state: c.state.into(),
        transfer_rate: c.transfer_rate.into(),
        duration: c.duration.into(),
        eta: c.eta.into(),
        transfer_type: c.transfer_type.into(),
        token: c.token.into(),
    }).collect();
    ui.set_connections(std::rc::Rc::new(slint::VecModel::from(slint_conns)).into());

    // Fetch system info and push to UI
    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();
    let cpu_count = sys.cpus().len();
    let memory_mb = sys.total_memory() / 1024 / 1024;
    ui.set_os_processor_info(format!("Processor: {} Logical Cores Detected", cpu_count).into());
    ui.set_os_memory_info(format!("RAM: {} MB Available", memory_mb).into());

    let (net_link, net_throughput) = network::get_active_network_info();
    ui.set_network_link_type(net_link.into());
    ui.set_network_max_throughput(net_throughput.into());

    ui.on_open_log_file(|filename| {
        open_log(filename.as_str());
    });

    let ui_handle = ui.as_weak();
    
    // Shared control channel for the active transfer
    let active_control_tx: Arc<Mutex<Option<watch::Sender<ControlSignal>>>> = Arc::new(Mutex::new(None));
    let start_tx = active_control_tx.clone();
    let pause_tx = active_control_tx.clone();
    let cancel_tx = active_control_tx.clone();

    ui.on_start_transfer(move || {
        let ui_handle = ui_handle.clone();
        let control_tx_ref = start_tx.clone();

        let (src, dest, transfer_type) = if let Some(ui) = ui_handle.upgrade() {
            ui.set_active_tab(1);
            let mappings: Vec<_> = ui.get_current_mappings().iter().collect();
            if mappings.is_empty() { return; }
            let t_type = ui.get_transfer_type_val().to_string();
            (mappings[0].source.to_string(), mappings[0].destination.to_string(), t_type)
        } else {
            return;
        };

        tokio::spawn(async move {
            let (tx, mut rx) = mpsc::channel(100);

            let (control_tx, control_rx) = watch::channel(ControlSignal::Run);
            if let Ok(mut guard) = control_tx_ref.lock() {
                *guard = Some(control_tx);
            }

            let engine = TransferEngine::new(
                PathBuf::from(src),
                dest,
                tx,
                Arc::new(MockSshClient::new()),
                Arc::new(FallbackSnapshotDriver),
                Some(control_rx),
            );

            let (batch_tx, mut batch_rx) = mpsc::channel(10000);

            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    batch_tx.send(event).await.ok();
                }
            });

            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
                let mut buffer = Vec::with_capacity(5000);

                let mut local_t_q = Vec::new();
                let mut local_c_q = Vec::new();
                let mut phase_txt = "Live Transfer Queue".to_string();
                let mut status_txt = "Completed: 0 / 0 files".to_string();
                let mut metrics_txt = "Upload: 0 MB/s | Download: 0 MB/s | Latency: 0ms".to_string();
                let mut last_progress_bytes = 0;
                let mut last_tick = tokio::time::Instant::now();

                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                            if buffer.is_empty() {
                                continue;
                            }

                            let events = std::mem::replace(&mut buffer, Vec::with_capacity(5000));

                            for event in events {
                                match event {
                                    TransferEvent::PhaseChanged(phase, name) => {
                                        phase_txt = format!("Phase {}: {}", phase, name);
                                    }
                                    TransferEvent::FileStatusChanged(file, status) => {
                                        match status {
                                            FileTransferStatus::Transferring { progress_bytes, total_bytes } => {
                                                let progress = if total_bytes > 0 { progress_bytes as f32 / total_bytes as f32 } else { 0.0 };
                                                local_t_q = vec![TransferQueueItem {
                                                    name: file.into(),
                                                    size: format!("{} bytes", total_bytes).into(),
                                                    progress,
                                                }];
                                                
                                                let now = tokio::time::Instant::now();
                                                let elapsed = now.duration_since(last_tick).as_secs_f32();
                                                if elapsed > 0.0 && progress_bytes >= last_progress_bytes {
                                                    let diff = progress_bytes - last_progress_bytes;
                                                    let speed_mbps = (diff as f32 / elapsed) / 1_048_576.0;
                                                    metrics_txt = format!("Upload: {:.1} MB/s | Download: 0 MB/s | Latency: ~12ms", speed_mbps);
                                                }
                                                last_progress_bytes = progress_bytes;
                                                last_tick = now;
                                            }
                                            FileTransferStatus::Completed => {
                                                local_c_q.push(CompletedItem {
                                                    name: file.into(),
                                                    status: "Completed".into(),
                                                    status_color: slint::Color::from_rgb_u8(200, 200, 50),
                                                });
                                            }
                                            FileTransferStatus::Validated => {
                                                local_c_q.push(CompletedItem {
                                                    name: file.into(),
                                                    status: "Validated".into(),
                                                    status_color: slint::Color::from_rgb_u8(50, 200, 50),
                                                });
                                            }
                                            FileTransferStatus::Failed(err) => {
                                                local_c_q.push(CompletedItem {
                                                    name: file.into(),
                                                    status: format!("Failed: {}", err).into(),
                                                    status_color: slint::Color::from_rgb_u8(200, 50, 50),
                                                });
                                            }
                                            _ => {}
                                        }
                                    }
                                    TransferEvent::TransferComplete { successful, failed } => {
                                        status_txt = format!("Completed: {} Success, {} Failed", successful, failed);
                                        phase_txt = "Transfer Complete".to_string();
                                        local_t_q.clear();
                                        let _ = notify_rust::Notification::new()
                                            .summary("Simply Transfer")
                                            .body(&format!("Transfer completed: {} successful, {} failed.", successful, failed))
                                            .show();
                                    }
                                    TransferEvent::TransferFailed(err) => {
                                        status_txt = format!("Transfer Aborted: {}", err);
                                        local_t_q.clear();
                                        let _ = notify_rust::Notification::new()
                                            .summary("Simply Transfer Error")
                                            .body(&format!("Transfer aborted: {}", err))
                                            .show();
                                    }
                                }
                            }

                            let ui_handle = ui_handle.clone();
                            let clone_t_q = local_t_q.clone();
                            let clone_c_q = local_c_q.clone();
                            let clone_phase = phase_txt.clone();
                            let clone_status = status_txt.clone();

                            let clone_metrics = metrics_txt.clone();

                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui) = ui_handle.upgrade() {
                                    let t_model = std::rc::Rc::new(slint::VecModel::from(clone_t_q));
                                    ui.set_transfer_queue(t_model.into());

                                    let c_model = std::rc::Rc::new(slint::VecModel::from(clone_c_q));
                                    ui.set_completed_queue(c_model.into());

                                    ui.set_phase_text(clone_phase.into());
                                    ui.set_overall_status(clone_status.into());
                                    ui.set_transfer_metrics_text(clone_metrics.into());
                                }
                            });
                        }
                        n = batch_rx.recv_many(&mut buffer, 5000) => {
                            if n == 0 {
                                break;
                            }
                        }
                    }
                }
            });

            if transfer_type == "Continuous Sync" || transfer_type == "Scheduled Transfer" {
                let log_file = if transfer_type == "Continuous Sync" { "sync" } else { "schedule" };
                log_event(log_file, &format!("Starting background daemon for {}", transfer_type));
                tracing::info!("Starting background daemon for {}", transfer_type);
                loop {
                    log_event(log_file, "Executing background transfer cycle...");
                    tracing::info!("Executing background transfer cycle...");
                    if let Err(e) = engine.execute().await {
                        log_event(log_file, &format!("Engine execution failed: {:?}", e));
                        tracing::error!("Engine execution failed: {:?}", e);
                    } else {
                        log_event(log_file, "Cycle completed successfully, waiting for next interval.");
                    }
                    
                    let sleep_duration = if transfer_type == "Continuous Sync" {
                        60 // Mock 1 min sync
                    } else {
                        3600 // Mock 1 hr schedule
                    };
                    
                    tracing::info!("Cycle complete. Sleeping for {} seconds...", sleep_duration);
                    tokio::time::sleep(tokio::time::Duration::from_secs(sleep_duration)).await;
                }
            } else {
                log_event("transfers", "Initiating single transfer...");
                if let Err(e) = engine.execute().await {
                    log_event("transfers", &format!("Engine execution failed: {:?}", e));
                    tracing::error!("Engine execution failed: {:?}", e);
                } else {
                    log_event("transfers", "Transfer and validation completed successfully.");
                }
            }
        });
    });

    let ui_weak = ui.as_weak();
    ui.on_commit_connection(move |new_conn| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
            let mut conn_to_save = new_conn.clone();
            
            tokio::spawn(async move {
                if !simply_transfer_core::ssh_server::SshServer::is_installed() {
                    log_event("connection", "SSH server is not installed. Prompting for privileges to install it...");
                    if let Err(e) = simply_transfer_core::ssh_server::SshServer::install_server() {
                        log_event("connection", &format!("Failed to install SSH server: {}", e));
                    } else {
                        log_event("connection", "SSH server installed successfully.");
                    }
                }
                
                if !simply_transfer_core::ssh_server::SshServer::is_running() {
                    log_event("connection", "SSH server is not running. Prompting for privileges to start it...");
                    if let Err(e) = simply_transfer_core::ssh_server::SshServer::start_server() {
                        log_event("connection", &format!("Failed to start SSH server: {}", e));
                    } else {
                        log_event("connection", "SSH server started successfully.");
                    }
                }
            });

            if conn_to_save.transfer_type != "Remote Transfer" {
                let pub_key = conn_to_save.token.to_string();
                
                // We must embed OUR local IP in the token so the remote device knows where to reach us.
                // The 'host' field here is the destination IP, so we should NOT use it for the token.
                let mut local_ip = String::new();
                if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0")
                    && socket.connect("8.8.8.8:80").is_ok()
                    && let Ok(addr) = socket.local_addr()
                {
                    local_ip = addr.ip().to_string();
                }
                
                if local_ip.trim().is_empty() || local_ip == "0.0.0.0" {
                    local_ip = "127.0.0.1".to_string();
                }
                
                let listener = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
                let port = listener.local_addr().unwrap().port();
                listener.set_nonblocking(true).unwrap();
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                
                if let Err(e) = simply_transfer_core::firewall::Firewall::open_port(port) {
                    log_event("connection", &format!("Warning: Failed to automatically open firewall port {}: {}", port, e));
                } else {
                    log_event("connection", &format!("Opened firewall port {} for OOB trigger.", port));
                }
                
                let pub_key_clone = pub_key.clone();
                let ui_handle = ui_weak.clone();
                
                tokio::spawn(async move {
                    while let Ok((mut socket, addr)) = listener.accept().await {
                        let pub_key = pub_key_clone.clone();
                        let ui_handle = ui_handle.clone();
                        tokio::spawn(async move {
                            use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
                            let (reader, mut writer) = socket.split();
                            let mut buf_reader = tokio::io::BufReader::new(reader);
                            let mut line = String::new();
                            
                            if buf_reader.read_line(&mut line).await.is_ok()
                                && let Ok(req) = serde_json::from_str::<serde_json::Value>(&line)
                            {
                                if req["action"] == "verify" {
                                    let dest_ip = addr.ip().to_string();
                                    let dest_user = req["user"].as_str().unwrap_or("simply-transfer").to_string();
                                    tracing::info!("Received verify request from {}@{}", dest_user, dest_ip);
                                    let mgr = simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
                                    match mgr.get_private_key_pem(&pub_key) {
                                        Ok(priv_pem) => {
                                            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                                            let ssh_dir = std::path::Path::new(&home).join(".ssh");
                                            let _ = std::fs::create_dir_all(&ssh_dir);
                                            let tmp_pem = ssh_dir.join("simply-transfer-tmp.pem");
                                            
                                            if let Err(e) = std::fs::write(&tmp_pem, priv_pem.as_bytes()) {
                                                tracing::error!("Failed to write tmp_pem: {}", e);
                                            } else {
                                                #[cfg(unix)]
                                                {
                                                    use std::os::unix::fs::PermissionsExt;
                                                    let _ = std::fs::set_permissions(&tmp_pem, std::fs::Permissions::from_mode(0o600));
                                                }
                                                use simply_transfer_core::ssh::SshClient;
                                                let mut ssh_client = simply_transfer_core::ssh2_client::Ssh2Client::new();
                                                match ssh_client.connect(&dest_ip, 22) {
                                                    Ok(_) => {
                                                        tracing::info!("SSH connected to {}", dest_ip);
                                                        match ssh_client.authenticate_publickey(&dest_user, tmp_pem.to_str().unwrap(), None) {
                                                            Ok(_) => {
                                                                tracing::info!("SSH authenticated with {}", dest_ip);
                                                                match ssh_client.execute_command("cat ~/.ssh/simply-transfer.pub") {
                                                                    Ok(dest_pub) => {
                                                                        tracing::info!("Fetched dest_pub");
                                                                        let pub_key_line = format!("ssh-ed25519 {} simply-transfer", dest_pub.trim());
                                                                        let auth_keys = ssh_dir.join("authorized_keys");
                                                                        use std::io::Write;
                                                                        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&auth_keys) {
                                                                            let _ = writeln!(f, "{}", pub_key_line);
                                                                            let _ = writer.write_all(b"{\"status\":\"ok\"}\n").await;
                                                                            let _ = slint::invoke_from_event_loop(move || {
                                                                                if let Some(ui) = ui_handle.upgrade() {
                                                                                    ui.set_overall_status("Connection Confirmed".into());
                                                                                }
                                                                            });
                                                                            let _ = std::fs::remove_file(&tmp_pem);
                                                                            return;
                                                                        } else {
                                                                            tracing::error!("Failed to open authorized_keys for append");
                                                                        }
                                                                    }
                                                                    Err(e) => tracing::error!("SSH execute_command failed: {}", e),
                                                                }
                                                            }
                                                            Err(e) => tracing::error!("SSH auth failed: {:?}", e),
                                                        }
                                                    }
                                                    Err(e) => tracing::error!("SSH connect failed: {:?}", e),
                                                }
                                                let _ = std::fs::remove_file(&tmp_pem);
                                            }
                                        }
                                        Err(e) => tracing::error!("Failed to get private key for token pub_key: {:?}", e),
                                    }
                                    let _ = writer.write_all(b"{\"status\":\"error\"}\n").await;
                                } else if req["action"] == "commit" {
                                    let _ = writer.write_all(b"{\"status\":\"ok\"}\n").await;
                                    let _ = slint::invoke_from_event_loop(move || {
                                        if let Some(ui) = ui_handle.upgrade() {
                                            ui.set_overall_status("Bidirectional sync configured".into());
                                        }
                                    });
                                } else if req["action"] == "cancel" {
                                    let _ = writer.write_all(b"{\"status\":\"ok\"}\n").await;
                                }
                            }
                        });
                    }
                });
                
                let generated_token = simply_transfer_crypto::token::ConnectionToken::generate(&local_ip, port, &pub_key);
                conn_to_save.token = generated_token.into();
            } else {
                let token = conn_to_save.token.to_string();
                if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&token) {
                    conn_to_save.host = parsed.ip.clone().into();
                    if conn_to_save.name == "Remote" {
                        conn_to_save.name = format!("Remote ({})", parsed.ip).into();
                    }
                    let addr = format!("{}:{}", parsed.ip, parsed.port);
                    tokio::spawn(async move {
                        if let Ok(mut stream) = tokio::net::TcpStream::connect(&addr).await {
                            use tokio::io::AsyncWriteExt;
                            let _ = stream.write_all(b"{\"action\":\"commit\"}\n").await;
                        }
                    });
                }
            }
            
            conns.push(conn_to_save);
            save_connections(&conns);
            let model = std::rc::Rc::new(slint::VecModel::from(conns));
            ui.set_connections(model.into());
            
            let token = new_conn.token.to_string();
            if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&token) {
                let addr = format!("{}:{}", parsed.ip, parsed.port);
                tokio::spawn(async move {
                    if let Ok(mut stream) = tokio::net::TcpStream::connect(&addr).await {
                        use tokio::io::AsyncWriteExt;
                        let _ = stream.write_all(b"{\"action\":\"commit\"}\n").await;
                    }
                });
            }
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_browse_source(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_is_browser_open(true);
            ui.set_browser_is_remote(false);
            ui.set_browser_is_source(true);
            ui.set_browser_target_idx(idx);
            
            let mappings: Vec<_> = ui.get_current_mappings().iter().collect();
            let idx = idx as usize;
            let mut start_path = "/".to_string();
            if idx < mappings.len() && !mappings[idx].source.is_empty() {
                start_path = mappings[idx].source.to_string();
            }
            ui.set_browser_current_path(start_path.clone().into());
            ui.invoke_fetch_directory(start_path.into(), false);
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_browse_destination(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_is_browser_open(true);
            ui.set_browser_is_remote(true);
            ui.set_browser_is_source(false);
            ui.set_browser_target_idx(idx);
            
            let mappings: Vec<_> = ui.get_current_mappings().iter().collect();
            let idx = idx as usize;
            let mut start_path = "/".to_string();
            if idx < mappings.len() && !mappings[idx].destination.is_empty() {
                start_path = mappings[idx].destination.to_string();
            }
            ui.set_browser_current_path(start_path.clone().into());
            ui.invoke_fetch_directory(start_path.into(), true);
        }
    });
    
    let ui_weak = ui.as_weak();
    ui.on_fetch_directory(move |path, _is_remote| {
        let ui_weak = ui_weak.clone();
        let path_str = path.to_string();
        
        tokio::spawn(async move {
            let mut nodes = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&path_str) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                    nodes.push(FileNode {
                        name: name.into(),
                        is_dir,
                        path: entry.path().to_string_lossy().to_string().into(),
                    });
                }
            }
            
            nodes.sort_by(|a, b| {
                if a.is_dir && !b.is_dir { std::cmp::Ordering::Less }
                else if !a.is_dir && b.is_dir { std::cmp::Ordering::Greater }
                else { a.name.cmp(&b.name) }
            });
            
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    let model = std::rc::Rc::new(slint::VecModel::from(nodes));
                    ui.set_browser_nodes(model.into());
                    ui.set_browser_current_path(path_str.into());
                }
            });
        });
    });

    let ui_weak = ui.as_weak();
    ui.on_commit_browser_selection(move |path| {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_is_browser_open(false);
            let idx = ui.get_browser_target_idx() as usize;
            let mut mappings: Vec<_> = ui.get_current_mappings().iter().collect();
            if idx < mappings.len() {
                if ui.get_browser_is_source() {
                    mappings[idx].source = path.clone();
                } else {
                    mappings[idx].destination = path;
                }
                ui.set_current_mappings(std::rc::Rc::new(slint::VecModel::from(mappings)).into());
            }
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_add_mapping(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let mut mappings: Vec<_> = ui.get_current_mappings().iter().collect();
            mappings.push(DirectoryMapping { source: "".into(), destination: "".into() });
            ui.set_current_mappings(std::rc::Rc::new(slint::VecModel::from(mappings)).into());
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_remove_mapping(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut mappings: Vec<_> = ui.get_current_mappings().iter().collect();
            let idx = idx as usize;
            if idx < mappings.len() {
                mappings.remove(idx);
                ui.set_current_mappings(std::rc::Rc::new(slint::VecModel::from(mappings)).into());
            }
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_update_mapping_source(move |idx, text| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut mappings: Vec<_> = ui.get_current_mappings().iter().collect();
            let idx = idx as usize;
            if idx < mappings.len() {
                mappings[idx].source = text;
                ui.set_current_mappings(std::rc::Rc::new(slint::VecModel::from(mappings)).into());
            }
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_update_mapping_destination(move |idx, text| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut mappings: Vec<_> = ui.get_current_mappings().iter().collect();
            let idx = idx as usize;
            if idx < mappings.len() {
                mappings[idx].destination = text;
                ui.set_current_mappings(std::rc::Rc::new(slint::VecModel::from(mappings)).into());
            }
        }
    });

    ui.on_popout_session(move || {
        let popout = SessionPopout::new().unwrap();
        popout.show().unwrap();
        Box::leak(Box::new(popout));
    });

    let ui_weak = ui.as_weak();
    ui.on_delete_connection(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
            let idx = idx as usize;
            if idx < conns.len() {
                let token = conns[idx].token.to_string();
                if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&token) {
                    let key_mgr = simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
                    let _ = key_mgr.delete_key(&parsed.pub_key);
                    
                    let addr = format!("{}:{}", parsed.ip, parsed.port);
                    tokio::spawn(async move {
                        if let Ok(mut stream) = tokio::net::TcpStream::connect(&addr).await {
                            use tokio::io::AsyncWriteExt;
                            let _ = stream.write_all(b"{\"action\":\"cancel\"}\n").await;
                        }
                    });
                }
                conns.remove(idx);
                save_connections(&conns);
                let model = std::rc::Rc::new(slint::VecModel::from(conns));
                ui.set_connections(model.into());
            }
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_verify_remote_token(move |token| {
        let ui_weak = ui_weak.clone();
        let token = token.to_string();
        tokio::spawn(async move {
            if !simply_transfer_core::ssh_server::SshServer::is_installed() {
                log_event("connection", "SSH server is not installed. Prompting for privileges to install it...");
                if let Err(e) = simply_transfer_core::ssh_server::SshServer::install_server() {
                    log_event("connection", &format!("Failed to install SSH server: {}", e));
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_is_remote_verified(false);
                            ui.set_remote_verification_status(format!("Failed to install SSH: {}", e).into());
                        }
                    });
                    return;
                }
                log_event("connection", "SSH server installed successfully.");
            }

            if !simply_transfer_core::ssh_server::SshServer::is_running() {
                log_event("connection", "SSH server is not running. Prompting for privileges to start it...");
                if let Err(e) = simply_transfer_core::ssh_server::SshServer::start_server() {
                    log_event("connection", &format!("Failed to start SSH server: {}", e));
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_is_remote_verified(false);
                            ui.set_remote_verification_status(format!("Failed to start SSH: {}", e).into());
                        }
                    });
                    return;
                }
                log_event("connection", "SSH server started successfully.");
            }
            
            log_event("connection", &format!("Validating remote token: {}", token));
            
            let (verified, status) = match simply_transfer_crypto::token::ConnectionToken::parse(&token) {
                Ok(parsed) => {
                    // Ingest the public key to authorized_keys
                    let pub_key_line = format!("ssh-ed25519 {} simply-transfer", parsed.pub_key);
                    
                    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                    let ssh_dir = std::path::Path::new(&home).join(".ssh");
                    let _ = std::fs::create_dir_all(&ssh_dir);
                    let auth_keys = ssh_dir.join("authorized_keys");
                    
                    let mut key_exists = false;
                    if let Ok(content) = std::fs::read_to_string(&auth_keys)
                        && content.contains(&parsed.pub_key)
                    {
                        key_exists = true;
                    }
                    
                    if !key_exists {
                        use std::io::Write;
                        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&auth_keys) {
                            let _ = writeln!(file, "{}", pub_key_line);
                            log_event("connection", "Appended public key to authorized_keys.");
                        }
                    } else {
                        log_event("connection", "Public key already in authorized_keys.");
                    }
                    
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(&ssh_dir, std::fs::Permissions::from_mode(0o700));
                        let _ = std::fs::set_permissions(&auth_keys, std::fs::Permissions::from_mode(0o600));
                    }
                    
                    // Trigger remote handshake (TCP connect to source device)
                    let addr = format!("{}:{}", parsed.ip, parsed.port);
                    log_event("connection", &format!("Attempting TCP handshake with source at {}", addr));
                    
                    // Generate our own key pair and transmit it over SSH
                    let key_mgr = simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
                    if let Ok(pub_b) = key_mgr.generate_and_store() {
                        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                        let ssh_dir = std::path::Path::new(&home).join(".ssh");
                        let _ = std::fs::create_dir_all(&ssh_dir);
                        let dest_pub = ssh_dir.join("simply-transfer.pub");
                        let _ = std::fs::write(&dest_pub, pub_b.as_bytes());
                        
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = std::fs::set_permissions(&ssh_dir, std::fs::Permissions::from_mode(0o700));
                            let _ = std::fs::set_permissions(&dest_pub, std::fs::Permissions::from_mode(0o644));
                        }
                    }
                    
                    // Use a short timeout for the handshake
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(15),
                        tokio::net::TcpStream::connect(&addr)
                    ).await {
                        Ok(Ok(mut stream)) => {
                            use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
                            let dest_user = std::env::var("USER").unwrap_or_else(|_| "simply-transfer".to_string());
                            let req = format!("{{\"action\":\"verify\",\"user\":\"{}\"}}\n", dest_user);
                            let _ = stream.write_all(req.as_bytes()).await;
                            let mut reader = tokio::io::BufReader::new(stream);
                            let mut line = String::new();
                            if reader.read_line(&mut line).await.is_ok() && line.contains("\"ok\"") {
                                log_event("connection", "Remote token validated and handshake succeeded.");
                                (true, "Connection Verified Successfully".to_string())
                            } else {
                                log_event("connection", "Handshake failed to return OK.");
                                (false, "Handshake Failed".to_string())
                            }
                        }
                        Ok(Err(e)) => {
                            log_event("connection", &format!("Handshake failed: {}", e));
                            // Still return verified true since we ingested the key, but indicate connection issue
                            (true, format!("Key Ingested, but Handshake Failed: {}", e))
                        }
                        Err(_) => {
                            log_event("connection", "Handshake timed out.");
                            (true, "Key Ingested, but Handshake Timed Out".to_string())
                        }
                    }
                }
                Err(e) => {
                    log_event("connection", &format!("Invalid token format: {}", e));
                    (false, format!("Invalid Token Format: {}", e))
                }
            };
            
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_is_remote_verified(verified);
                    ui.set_remote_verification_status(status.into());
                }
            });
        });
    });

    ui.on_generate_key(move |_host| {
        let key_mgr = simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
        match key_mgr.generate_and_store() {
            Ok(pub_key) => pub_key.into(),
            Err(e) => {
                tracing::error!("Failed to generate key: {}", e);
                "".into()
            }
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_remove_key(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let idx = ui.get_selected_connection_idx() as usize;
            let conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
            if idx < conns.len() {
                let token = conns[idx].token.to_string();
                if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&token) {
                    let key_mgr = simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
                    if let Err(e) = key_mgr.delete_key(&parsed.pub_key) {
                        tracing::warn!("Failed to delete key: {}", e);
                    } else {
                        println!("Successfully deleted key from OS keyring for connection: {}", conns[idx].name);
                    }
                }
            }
        }
    });

    ui.on_cancel_key_generation(move |pub_key| {
        let pub_key = pub_key.to_string();
        if !pub_key.is_empty() {
            let key_mgr = simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
            if let Err(e) = key_mgr.delete_key(&pub_key) {
                tracing::warn!("Failed to delete cancelled key: {}", e);
            } else {
                tracing::info!("Purged cancelled key from keyring.");
            }
        }
    });

    ui.on_purge_orphaned_keys(move || {
        tracing::warn!("Purging orphaned keys from OS keyring requires manual intervention via secret-tool currently.");
        0
    });

    let ui_weak = ui.as_weak();
    ui.on_copy_token(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let idx = ui.get_selected_connection_idx() as usize;
            let conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
            if idx < conns.len() {
                let token = conns[idx].token.to_string();
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    let _ = clipboard.set_text(token.clone());
                }
                println!("Token copied to clipboard: {}", token);
            }
        }
    });

    ui.on_add_connection(move || {
        println!("Add connection initiated");
    });

    ui.on_pause_transfer(move || {
        println!("Transfer paused");
        if let Ok(guard) = pause_tx.lock()
            && let Some(tx) = guard.as_ref()
        {
            // Toggle between Pause and Run for simplicity, assuming the button acts as play/pause
            let current = tx.borrow().clone();
            let next = if current == ControlSignal::Pause { ControlSignal::Run } else { ControlSignal::Pause };
            let _ = tx.send(next);
        }
    });

    ui.on_cancel_transfer(move || {
        println!("Transfer cancelled");
        if let Ok(mut guard) = cancel_tx.lock()
            && let Some(tx) = guard.take()
        {
            let _ = tx.send(ControlSignal::Cancel);
        }
    });

    ui.run()?;

    Ok(())
}
