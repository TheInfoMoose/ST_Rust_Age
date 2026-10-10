use simply_transfer_core::engine::{
    ControlSignal, FileTransferStatus, TransferEngine, TransferEvent,
};
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
    let filename = format!("connections_{}.json", std::env::consts::OS);
    if let Ok(data) = std::fs::read_to_string(&filename)
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
        let filename = format!("connections_{}.json", std::env::consts::OS);
        let _ = std::fs::write(&filename, json);
    }
}

fn log_event(log_type: &str, message: &str) {
    use std::io::Write;
    let dir_name = format!("logs_{}", std::env::consts::OS);
    let dir = std::path::Path::new(&dir_name);
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
    let dir_name = format!("logs_{}", std::env::consts::OS);
    let path = format!("{}/{}", dir_name, filename);
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", &path])
        .spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(&path).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
}

use std::sync::OnceLock;
static P2P_SERVER: OnceLock<simply_transfer_core::russh_server::TransferServer> = OnceLock::new();

#[rustfmt::skip]
fn get_file_snapshot_type(filename: &str) -> String {
    let ext_lower = std::path::Path::new(filename)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    
    let snapshot_extensions = [
        "qbw", "qbb", "qbm", "qbo", "qbx", "qba", "qby",
        "nd", "tlg", "iif", "log", "backupbundle", "sparsebundle",
        "hbk", "bki", "bak", "sql", "dump", "vbk", "vib",
        "pst", "ost", "ade", "adp", "lbd", "laccdb", "docm",
        "xlsm", "pptm", "db", "lock", "lck", "_lock", "mdf",
        "ldf", "sqlite", "sqlite3", "db-wal", "db-shm", "ibd",
        "frm", "edb", "vhdx", "vhd", "vmdk", "qcow2", "raw",
        "img", "vdi", "dat", "dmg", "wim", "esd"
    ];
    
    if snapshot_extensions.contains(&ext_lower.as_str()) {
        simply_transfer_snapshots::get_native_driver().snapshot_type().to_string()
    } else {
        "".to_string()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Create a dedicated Tokio runtime, separating it from the Slint event loop
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    // Enter the runtime context so `tokio::spawn` works in Slint callbacks
    let _guard = rt.enter();

    let (global_tx, mut global_rx) =
        tokio::sync::broadcast::channel::<simply_transfer_core::engine::TransferEvent>(1000);

    let host_key = russh_keys::key::KeyPair::generate_ed25519().unwrap();
    let mut p2p_server = simply_transfer_core::russh_server::TransferServer::new();
    p2p_server.set_event_sender(global_tx.clone());
    P2P_SERVER.set(p2p_server.clone()).ok();

    rt.spawn(async move {
        if let Err(e) =
            simply_transfer_core::russh_server::run_server(2222, p2p_server, host_key).await
        {
            tracing::error!("P2P Daemon globally crashed: {}", e);
        }
    });

    let ui = MainWindow::new()?;
    let ui_global_weak = ui.as_weak();

    tokio::spawn(async move {
        let mut local_t_q: Vec<TransferQueueItem> = Vec::new();
        let mut overall_total_bytes: u64 = 0;
        let mut target_conn_name = String::new();
        let mut transfer_start_time = tokio::time::Instant::now();
        let mut current_completed_bytes: u64 = 0;
        let mut last_ui_update = std::time::Instant::now();
        let mut current_snapshot_type = String::from("Live");

        while let Ok(event) = global_rx.recv().await {
            match event {
                simply_transfer_core::engine::TransferEvent::TransferStarted(peer) => {
                    local_t_q.clear();
                    overall_total_bytes = 0;
                    current_completed_bytes = 0;
                    target_conn_name = peer.clone();
                    transfer_start_time = tokio::time::Instant::now();

                    let ui_clone = ui_global_weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_clone.upgrade() {
                            ui.set_active_tab(1); // Assuming 1 is Dashboard or Active Transfer
                            ui.set_transfer_queue(
                                std::rc::Rc::new(slint::VecModel::from(Vec::new())).into(),
                            );
                            ui.set_phase_text("Live Transfer Queue".into());
                            ui.set_overall_status("Receiving...".into());
                            ui.set_current_phase("Receiving".into());
                        }
                    });
                }
                simply_transfer_core::engine::TransferEvent::FileStatusChanged(file, status) => {
                    if let simply_transfer_core::engine::FileTransferStatus::Transferring {
                        progress_bytes,
                        total_bytes,
                    } = status
                    {
                        overall_total_bytes = overall_total_bytes.max(total_bytes);
                        let progress = if total_bytes > 0 {
                            progress_bytes as f32 / total_bytes as f32
                        } else {
                            0.0
                        };

                        let item = TransferQueueItem {
                            name: file.clone().into(),
                            size: format!("{:.2} MB", total_bytes as f64 / 1_048_576.0).into(),
                            progress,
                            snapshot_type: current_snapshot_type.clone().into(),
                        };

                        if local_t_q.is_empty() {
                            local_t_q.push(item);
                        } else {
                            local_t_q[0] = item;
                        }

                        current_completed_bytes = progress_bytes;

                        let elapsed_secs = transfer_start_time.elapsed().as_secs();
                        let clone_eta = if current_completed_bytes > 0
                            && overall_total_bytes > current_completed_bytes
                            && elapsed_secs > 0
                        {
                            let bytes_per_sec =
                                current_completed_bytes as f64 / elapsed_secs as f64;
                            let remaining_bytes =
                                overall_total_bytes.saturating_sub(current_completed_bytes);
                            let remaining_secs = (remaining_bytes as f64 / bytes_per_sec) as u64;
                            format!(
                                "{:02}:{:02}:{:02}",
                                remaining_secs / 3600,
                                (remaining_secs % 3600) / 60,
                                remaining_secs % 60
                            )
                        } else {
                            "Calculating...".to_string()
                        };

                        let clone_mu = if elapsed_secs > 0 {
                            format!(
                                "{:.2} MB/s",
                                (current_completed_bytes as f64 / 1_048_576.0)
                                    / elapsed_secs as f64
                            )
                        } else {
                            "0.0 MB/s".to_string()
                        };

                        let local_t_q_clone = local_t_q.clone();
                        let ui_clone = ui_global_weak.clone();

                        if last_ui_update.elapsed() >= std::time::Duration::from_millis(500) {
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui) = ui_clone.upgrade() {
                                    ui.set_transfer_queue(
                                        std::rc::Rc::new(slint::VecModel::from(local_t_q_clone))
                                            .into(),
                                    );
                                    ui.set_metric_download(clone_mu.into());
                                    ui.set_metric_eta(clone_eta.into());
                                }
                            });
                            last_ui_update = std::time::Instant::now();
                        }
                    } else if let simply_transfer_core::engine::FileTransferStatus::Completed =
                        status
                    {
                        local_t_q.clear();
                        let ui_clone = ui_global_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_clone.upgrade() {
                                ui.set_transfer_queue(
                                    std::rc::Rc::new(slint::VecModel::from(Vec::new())).into(),
                                );
                                ui.set_overall_status("Completed".into());
                                ui.set_current_phase("Idle".into());
                            }
                        });
                    }
                }
                _ => {}
            }
        }
    });

    let loaded = load_connections();
    let slint_conns: Vec<ConnectionItem> = loaded
        .into_iter()
        .map(|c| ConnectionItem {
            name: c.name.into(),
            host: c.host.into(),
            state: "Checking...".into(),
            state_color: slint::Color::from_rgb_u8(128, 128, 128),
            transfer_rate: c.transfer_rate.into(),
            duration: c.duration.into(),
            eta: c.eta.into(),
            transfer_type: c.transfer_type.into(),
            token: c.token.into(),
        })
        .collect();
    ui.set_connections(std::rc::Rc::new(slint::VecModel::from(slint_conns)).into());

    let ui_weak_hb = ui.as_weak();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;

            let (tx, rx) = tokio::sync::oneshot::channel();
            let ui_weak_clone = ui_weak_hb.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak_clone.upgrade() {
                    let conns: Vec<_> = ui.get_connections().iter().collect();
                    let mut conn_data = Vec::new();
                    for (i, c) in conns.iter().enumerate() {
                        conn_data.push((
                            i,
                            c.host.to_string(),
                            c.token.to_string(),
                            c.state.to_string(),
                            c.transfer_type.to_string(),
                        ));
                    }
                    let _ = tx.send(conn_data);
                }
            });

            if let Ok(conn_data) = rx.await {
                let mut updates = Vec::new();
                for (i, host, token, state, transfer_type) in conn_data {
                    if state.starts_with("Transmitting")
                        || state == "Waiting for peer..."
                        || state == "Connected"
                    {
                        continue;
                    }
                    if transfer_type != "Remote Transfer" {
                        continue;
                    }
                    let mut target_ip = String::new();
                    let token_parts: Vec<&str> = token.split(';').collect();
                    let actual_token = token_parts[0];
                    let my_pub_key = if token_parts.len() > 1 {
                        Some(token_parts[1])
                    } else {
                        None
                    };

                    if host.contains('@') {
                        target_ip = host.split('@').nth(1).unwrap_or("").to_string();
                    } else if host == "Remote" {
                        if let Ok(parsed) =
                            simply_transfer_crypto::token::ConnectionToken::parse(actual_token)
                        {
                            target_ip = parsed.ip;
                        }
                    } else {
                        target_ip = host.clone();
                    }

                    if !target_ip.is_empty() {
                        let is_tcp_open = tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            tokio::net::TcpStream::connect(format!("{}:2222", target_ip)),
                        )
                        .await
                        .map(|res| res.is_ok())
                        .unwrap_or(false);

                        let mut is_authenticated = false;
                        let mut dest_user = String::new();
                        if host.contains('@') {
                            dest_user = host.split('@').next().unwrap_or("").to_string();
                        } else if let Ok(parsed) =
                            simply_transfer_crypto::token::ConnectionToken::parse(actual_token)
                        {
                            if let Some(u) = parsed.user {
                                dest_user = u;
                            } else {
                                dest_user = "simply-transfer".to_string();
                            }
                        } else {
                            dest_user = "simply-transfer".to_string();
                        }

                        if is_tcp_open
                            && !dest_user.is_empty()
                            && let Ok(parsed) =
                                simply_transfer_crypto::token::ConnectionToken::parse(actual_token)
                        {
                            let mgr = simply_transfer_crypto::keys::KeyPairManager::new(
                                "com.simplytransfer.app",
                            );
                            let key_to_use = my_pub_key.unwrap_or(&parsed.pub_key);
                            if let Ok(priv_pem) = mgr.get_private_key_pem(key_to_use) {
                                let mut ssh =
                                    simply_transfer_core::russh_client::RusshClient::new();
                                use simply_transfer_core::ssh::SshClient;
                                if ssh.connect(&target_ip, 2222).await.is_ok()
                                    && ssh
                                        .authenticate_publickey(&dest_user, &priv_pem, None)
                                        .await
                                        .is_ok()
                                {
                                    is_authenticated = true;
                                }
                            }
                        }

                        let new_state = if is_authenticated {
                            "Connected"
                        } else if is_tcp_open {
                            "Pending Verification"
                        } else {
                            "Disconnected"
                        };

                        if new_state != state {
                            updates.push((i, new_state.to_string(), is_authenticated));
                        }
                    }
                }

                if !updates.is_empty() {
                    let ui_weak_clone = ui_weak_hb.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak_clone.upgrade() {
                            let mut conns: Vec<ConnectionItem> =
                                ui.get_connections().iter().collect();
                            for (i, new_state, is_connected) in updates {
                                if i < conns.len() {
                                    conns[i].state = new_state.into();
                                    conns[i].state_color = if is_connected {
                                        slint::Color::from_rgb_u8(50, 200, 50)
                                    } else {
                                        slint::Color::from_rgb_u8(200, 50, 50)
                                    };
                                }
                            }
                            ui.set_connections(
                                std::rc::Rc::new(slint::VecModel::from(conns)).into(),
                            );
                        }
                    });
                }
            }
        }
    });

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
    let active_control_tx: Arc<Mutex<Option<watch::Sender<ControlSignal>>>> =
        Arc::new(Mutex::new(None));
    let start_tx = active_control_tx.clone();
    let pause_tx = active_control_tx.clone();
    let cancel_tx = active_control_tx.clone();
    let global_tx_for_start = global_tx.clone();

    ui.on_start_transfer(move || {
        let global_tx = global_tx_for_start.clone();
        let ui_handle = ui_handle.clone();
        let control_tx_ref = start_tx.clone();

        let mut dest_ip = String::new();
        let mut dest_user = String::new();
        let mut actual_token_str = String::new();
        let mut my_pub_key = None;
        let mut conn_name = String::new();
        let mut target_host = String::new();
        if let Some(ui) = ui_handle.upgrade() {
            let conns: Vec<_> = ui.get_connections().iter().collect();
            let idx = ui.get_selected_connection_idx() as usize;
            if idx < conns.len() {
                let host = conns[idx].host.to_string();
                target_host = host.clone();
                conn_name = conns[idx].name.to_string();
                let token_str = conns[idx].token.to_string();
                
                let token_parts: Vec<&str> = token_str.split(';').collect();
                actual_token_str = token_parts[0].to_string();
                if token_parts.len() > 1 {
                    my_pub_key = Some(token_parts[1].to_string());
                }

                if host.contains('@') {
                    let parts: Vec<&str> = host.split('@').collect();
                    dest_user = parts[0].to_string();
                    dest_ip = parts[1].to_string();
                } else if host == "Remote" {
                    if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&actual_token_str) {
                        dest_ip = parsed.ip;
                        dest_user = parsed.user.unwrap_or_else(|| "simply-transfer".to_string());
                    }
                } else {
                    dest_ip = host.clone();
                    dest_user = "simply-transfer".to_string();
                }
            }
        }
        
        if dest_user.is_empty() {
            if let Some(ui) = ui_handle.upgrade() {
                ui.set_overall_status("Pending Remote Verification: Please verify token or use user@IP".into());
                ui.set_current_phase("Idle".into());
            }
            let _ = notify_rust::Notification::new()
                .summary("Verification Pending")
                .body("Pending Remote Verification: Please verify the token on the destination device, or explicitly provide a username (user@IP).")
                .show();
            return;
        }

        let (all_mappings, transfer_type, session_name) = if let Some(ui) = ui_handle.upgrade() {
            ui.set_active_tab(1);
            
            let mut sessions: Vec<slint::SharedString> = ui.get_active_sessions().iter().collect();
            let session_name = if conn_name.trim().is_empty() { format!("Transfer to {}", dest_ip) } else { conn_name.clone() };
            if !sessions.contains(&session_name.clone().into()) {
                sessions.push(session_name.clone().into());
                ui.set_active_sessions(std::rc::Rc::new(slint::VecModel::from(sessions)).into());
                ui.set_selected_session_idx(0);
            } else {
                // If it already exists, select it
                if let Some(pos) = sessions.iter().position(|x| x == &slint::SharedString::from(session_name.clone())) {
                    ui.set_selected_session_idx(pos as i32);
                }
            }
            
            // Clear previous transfer state
            ui.set_transfer_queue(std::rc::Rc::new(slint::VecModel::from(Vec::new())).into());
            ui.set_completed_queue(std::rc::Rc::new(slint::VecModel::from(Vec::new())).into());
            ui.set_phase_text("Live Transfer Queue".into());
            ui.set_overall_status("Preparing...".into());
            ui.set_current_phase("Idle".into());
            ui.set_is_transfer_paused(false);
            
            ui.set_remote_verified_host(format!("{}@{}", dest_user, dest_ip).into());
            
            let mappings: Vec<_> = ui.get_current_mappings().iter().collect();
            if mappings.is_empty() { return; }
            let t_type = ui.get_transfer_type_val().to_string();
            let all_mappings: Vec<(String, String)> = mappings.iter().map(|m| (m.source.to_string(), m.destination.to_string())).collect();
            (all_mappings, t_type, session_name)
        } else {
            return;
        };

        let dest_ip = dest_ip.clone();
        let dest_user = dest_user.clone();

        let target_host_clone = target_host.clone();
        let target_conn_name = conn_name.clone();

        tokio::spawn(async move {
            let (tx, mut rx) = mpsc::channel::<simply_transfer_core::engine::TransferEvent>(100);

            let (control_tx, control_rx) = watch::channel(ControlSignal::Run);
            if let Ok(mut guard) = control_tx_ref.lock() {
                *guard = Some(control_tx);
            }

            let mut ssh_client = simply_transfer_core::russh_client::RusshClient::new();
            let mut val_ssh_client = simply_transfer_core::russh_client::RusshClient::new();
            if !dest_ip.is_empty() {
                use simply_transfer_core::ssh::SshClient;
                let mut priv_pem = String::new();
                
                if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&actual_token_str) {
                    let mgr = simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
                    let key_to_use = my_pub_key.as_deref().unwrap_or(&parsed.pub_key);
                    if let Ok(pem) = mgr.get_private_key_pem(key_to_use) {
                        priv_pem = pem;
                    }
                }
                
                if priv_pem.is_empty() {
                    tracing::error!("Failed to retrieve private key from keyring for authentication.");
                } else {
                    if let Err(e) = ssh_client.connect(&dest_ip, 2222).await {
                        tracing::error!("Failed to connect SSH client in start_transfer: {}", e);
                    } else if let Err(e) = ssh_client.authenticate_publickey(&dest_user, &priv_pem, None).await {
                        tracing::error!("Failed to authenticate SSH client in start_transfer: {}", e);
                    }

                    if let Err(e) = val_ssh_client.connect(&dest_ip, 2222).await {
                        tracing::error!("Failed to connect Validation SSH client: {}", e);
                    } else if let Err(e) = val_ssh_client.authenticate_publickey(&dest_user, &priv_pem, None).await {
                        tracing::error!("Failed to authenticate Validation SSH client: {}", e);
                    }
                }
            }

            let ssh_client = Arc::new(ssh_client);
            let val_ssh_client = Arc::new(val_ssh_client);
            let snapshot_driver = Arc::new(FallbackSnapshotDriver);

            let (batch_tx, batch_rx) = mpsc::channel(10000);

            let global_tx_clone = global_tx.clone();
            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    let _ = global_tx_clone.send(event.clone());
                    batch_tx.send(event).await.ok();
                }
            });

            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
                let mut buffer = Vec::with_capacity(5000);

                let mut local_t_q: Vec<TransferQueueItem> = Vec::new();
                let mut local_c_q: Vec<CompletedItem> = Vec::new();
                let mut phase_txt = "Live Transfer Queue".to_string();
                let mut active_phase = "Idle".to_string();
                let mut status_txt = "Preparing...".to_string();
                let mut metric_upload_txt = "0.0 MB/s".to_string();
                let mut metric_latency_txt = "calculating...".to_string();
                let mut last_progress_bytes = 0;
                let mut last_tick = tokio::time::Instant::now();
                let transfer_start_time = tokio::time::Instant::now();
                let mut total_files = 0;
                let mut current_completed_files = 0;
                let mut overall_total_bytes = 0;
                let mut current_completed_bytes = 0;
                let mut previously_completed_bytes = 0;
                let mut current_file_size = 0;
                let mut bytes_transferred_in_interval = 0;
                let mut speed_ema: f32 = 0.0;
                let mut current_snapshot_type = String::from("Live");

                let mut batch_rx_opt = Some(batch_rx);
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
                                        active_phase = match name.as_str() {
                                            "Snapshot Discovery" => "Preparing Snapshots...",
                                            "Delta Calculation" => "Comparing Files...",
                                            "Data Transfer" => "Transmitting",
                                            "Integrity Validation (Finishing)" => {
                                                local_t_q.clear();
                                                metric_upload_txt = "0.0 MB/s".to_string();
                                                metric_latency_txt = "0ms".to_string();
                                                "Checking Integrity"
                                            },
                                            _ => name.as_str(),
                                        }.to_string();
                                    }
                                    TransferEvent::FileStatusChanged(file, status) => {
                                        match status {
                                            FileTransferStatus::Transferring { progress_bytes, total_bytes } => {
                                                current_file_size = total_bytes;
                                                current_completed_bytes = previously_completed_bytes + progress_bytes;
                                                let progress = if total_bytes > 0 { progress_bytes as f32 / total_bytes as f32 } else { 0.0 };
                                                local_t_q = vec![TransferQueueItem {
                                                    name: file.into(),
                                                    size: format!("{:.2} MB", total_bytes as f64 / 1_048_576.0).into(),
                                                    progress,
                                                    snapshot_type: current_snapshot_type.clone().into(),
                                                }];
                                                
                                                if progress_bytes > last_progress_bytes {
                                                    bytes_transferred_in_interval += progress_bytes - last_progress_bytes;
                                                } else if progress_bytes < last_progress_bytes {
                                                    bytes_transferred_in_interval += progress_bytes;
                                                }
                                                last_progress_bytes = progress_bytes;
                                            }
                                            FileTransferStatus::Completed => {
                                                local_t_q.clear();
                                                current_completed_files += 1;
                                                previously_completed_bytes += current_file_size;
                                                current_file_size = 0;
                                                status_txt = format!("Completed: {} / {} files", current_completed_files, total_files);
                                                log_event("transfer", &format!("Transferred: {}", file));
                                                
                                                if let Some(existing) = local_c_q.iter_mut().find(|i| i.name.as_str() == file.as_str()) {
                                                    existing.status = "Completed".into();
                                                    existing.status_color = slint::Color::from_rgb_u8(200, 200, 50);
                                                } else {
                                                    local_c_q.push(CompletedItem {
                                                        name: file.into(),
                                                        status: "Completed".into(),
                                                        status_color: slint::Color::from_rgb_u8(200, 200, 50),
                                                    });
                                                }
                                            }
                                            FileTransferStatus::Validated => {
                                                log_event("transfer", &format!("Validated Hash: {}", file));
                                                if let Some(existing) = local_c_q.iter_mut().find(|i| i.name.as_str() == file.as_str()) {
                                                    existing.status = "Validated".into();
                                                    existing.status_color = slint::Color::from_rgb_u8(50, 200, 50);
                                                } else {
                                                    local_c_q.push(CompletedItem {
                                                        name: file.into(),
                                                        status: "Validated".into(),
                                                        status_color: slint::Color::from_rgb_u8(50, 200, 50),
                                                    });
                                                }
                                            }
                                            FileTransferStatus::Failed(err) => {
                                                log_event("transfer", &format!("Failed: {} ({})", file, err));
                                                if let Some(existing) = local_c_q.iter_mut().find(|i| i.name.as_str() == file.as_str()) {
                                                    existing.status = format!("Failed: {}", err).into();
                                                    existing.status_color = slint::Color::from_rgb_u8(200, 50, 50);
                                                } else {
                                                    local_c_q.push(CompletedItem {
                                                        name: file.into(),
                                                        status: format!("Failed: {}", err).into(),
                                                        status_color: slint::Color::from_rgb_u8(200, 50, 50),
                                                    });
                                                }
                                            }
                                            FileTransferStatus::Skipped => {
                                                log_event("transfer", &format!("Skipped: {}", file));
                                                if let Some(existing) = local_c_q.iter_mut().find(|i| i.name.as_str() == file.as_str()) {
                                                    existing.status = "Skipped".into();
                                                    existing.status_color = slint::Color::from_rgb_u8(200, 150, 50);
                                                } else {
                                                    local_c_q.push(CompletedItem {
                                                        name: file.into(),
                                                        status: "Skipped".into(),
                                                        status_color: slint::Color::from_rgb_u8(200, 150, 50),
                                                    });
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                    TransferEvent::ManifestGenerated { total_files: total, total_bytes, snapshot_type } => {
                                        total_files = total;
                                        overall_total_bytes = total_bytes;
                                        status_txt = format!("Completed: {} / {} files", current_completed_files, total_files);
                                        current_snapshot_type = snapshot_type;
                                    }
                                    TransferEvent::TransferComplete { successful, failed } => {
                                        status_txt = format!("Completed: {} Success, {} Failed", successful, failed);
                                        phase_txt = "Transfer Complete".to_string();
                                        active_phase = "Transfer Complete".to_string();
                                        let _ = notify_rust::Notification::new()
                                            .summary("Simply Transfer")
                                            .body(&format!("Transfer completed: {} successful, {} failed.", successful, failed))
                                            .show();
                                    }
                                    TransferEvent::TransferFailed(err) => {
                                        status_txt = format!("Transfer Aborted: {}", err);
                                        active_phase = "Transfer Complete".to_string();
                                        let _ = notify_rust::Notification::new()
                                            .summary("Simply Transfer Error")
                                            .body(&format!("Transfer aborted: {}", err))
                                            .show();
                                    }
                                    TransferEvent::TransferStarted(_) => {}
                                }
                            }
                            
                            let now = tokio::time::Instant::now();
                            let elapsed = now.duration_since(last_tick).as_secs_f32();
                            if elapsed >= 0.1 {
                                if bytes_transferred_in_interval > 0 {
                                    let speed_mbps = (bytes_transferred_in_interval as f32 / elapsed) / 1_048_576.0;
                                    if speed_ema == 0.0 {
                                        speed_ema = speed_mbps;
                                    } else {
                                        speed_ema = speed_ema * 0.7 + speed_mbps * 0.3;
                                    }
                                    let dyn_latency = if speed_ema > 5.0 { "<1ms" } else { "2-4ms" };
                                    metric_upload_txt = format!("{:.1} MB/s", speed_ema);
                                    metric_latency_txt = dyn_latency.to_string();
                                    bytes_transferred_in_interval = 0;
                                } else if elapsed >= 1.0 {
                                    speed_ema = 0.0;
                                    metric_upload_txt = "0.0 MB/s".to_string();
                                    metric_latency_txt = "0ms".to_string();
                                }
                                if bytes_transferred_in_interval > 0 || elapsed >= 1.0 {
                                    last_tick = now;
                                }
                            }

                            let ui_handle = ui_handle.clone();
                            let clone_t_q = local_t_q.clone();
                            let clone_c_q = local_c_q.clone();
                            let clone_phase = phase_txt.clone();
                            let clone_status = status_txt.clone();

                            let clone_mu = metric_upload_txt.clone();
                            let clone_ml = metric_latency_txt.clone();
                            let clone_active_phase = active_phase.clone();
                            let session_name_clone = session_name.clone();
                            let target_host_for_closure = target_host_clone.clone();
                            let target_conn_name_for_closure = target_conn_name.clone();
                            let elapsed_secs = transfer_start_time.elapsed().as_secs();
                            let clone_duration = format!("{:02}:{:02}:{:02}", elapsed_secs / 3600, (elapsed_secs % 3600) / 60, elapsed_secs % 60);
                            let clone_eta = if current_completed_bytes > 0 && overall_total_bytes > current_completed_bytes {
                                let bytes_per_sec = current_completed_bytes as f64 / elapsed_secs as f64;
                                let remaining_bytes = overall_total_bytes.saturating_sub(current_completed_bytes);
                                let remaining_secs = (remaining_bytes as f64 / bytes_per_sec) as u64;
                                format!("{:02}:{:02}:{:02}", remaining_secs / 3600, (remaining_secs % 3600) / 60, remaining_secs % 60)
                            } else if overall_total_bytes == 0 {
                                "N/A".to_string()
                            } else {
                                "Calculating...".to_string()
                            };

                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui) = ui_handle.upgrade() {
                                    let active_sessions: Vec<_> = ui.get_active_sessions().iter().collect();
                                    let selected_idx = ui.get_selected_session_idx() as usize;
                                    let is_active = selected_idx < active_sessions.len() && active_sessions[selected_idx] == session_name_clone.as_str();
                                    
                                    if is_active {
                                        let t_model = std::rc::Rc::new(slint::VecModel::from(clone_t_q));
                                        ui.set_transfer_queue(t_model.into());

                                        let c_model = std::rc::Rc::new(slint::VecModel::from(clone_c_q));
                                        ui.set_completed_queue(c_model.into());

                                        ui.set_phase_text(clone_phase.into());
                                        ui.set_overall_status(clone_status.into());
                                        ui.set_metric_upload(clone_mu.clone().into());
                                        ui.set_metric_download("0.0 MB/s".into());
                                        ui.set_metric_eta(clone_ml.clone().into());
                                        ui.set_current_phase(clone_active_phase.clone().into());
                                    }
                                    
                                    if clone_active_phase != "Idle" {
                                        let mut conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
                                        let mut target_idx = None;
                                        for (idx, conn) in conns.iter().enumerate() {
                                            if conn.name.as_str() == target_conn_name_for_closure.as_str() && 
                                               conn.host.as_str() == target_host_for_closure.as_str() {
                                                target_idx = Some(idx);
                                                break;
                                            }
                                        }
                                        if let Some(idx) = target_idx {
                                            conns[idx].state = clone_active_phase.into();
                                            conns[idx].state_color = slint::Color::from_rgb_u8(50, 200, 50);
                                            conns[idx].duration = clone_duration.into();
                                            conns[idx].eta = clone_eta.into();
                                            conns[idx].transfer_rate = clone_mu.clone().into();
                                            
                                            let model = ui.get_connections();
                                            if let Some(vec_model) = model.as_any().downcast_ref::<slint::VecModel<ConnectionItem>>() {
                                                vec_model.set_row_data(idx, conns[idx].clone());
                                            } else {
                                                ui.set_connections(std::rc::Rc::new(slint::VecModel::from(conns)).into());
                                            }
                                        }
                                    }
                                }
                            });
                            if batch_rx_opt.is_none() && buffer.is_empty() {
                                break;
                            }
                        }
                        res = async {
                            if let Some(rx) = &mut batch_rx_opt {
                                rx.recv_many(&mut buffer, 5000).await
                            } else {
                                std::future::pending().await
                            }
                        } => {
                            if res == 0 {
                                batch_rx_opt = None;
                            }
                        }
                    }
                }
            });

            if transfer_type == "Continuous Sync" || transfer_type == "Scheduled Transfer" {
                let log_file = if transfer_type == "Continuous Sync" { "sync" } else { "schedule" };
                
                // Save schedule to JSON file
                let schedule_entry = serde_json::json!({
                    "transfer_type": transfer_type,
                    "mappings": all_mappings.clone(),
                    "sleep_duration": if transfer_type == "Continuous Sync" { 60 } else { 3600 }
                });
                
                let mut schedules = Vec::new();
                if let Ok(contents) = std::fs::read_to_string("schedules.json") {
                    if let Ok(existing_schedules) = serde_json::from_str::<Vec<serde_json::Value>>(&contents) {
                        schedules = existing_schedules;
                    }
                }
                
                schedules.push(schedule_entry);
                
                if let Ok(json_string) = serde_json::to_string_pretty(&schedules) {
                    let _ = std::fs::write("schedules.json", json_string);
                }
                
                log_event(log_file, &format!("Starting background daemon for {}", transfer_type));
                tracing::info!("Starting background daemon for {}", transfer_type);
                loop {
                    log_event(log_file, "Executing background transfer cycle...");
                    tracing::info!("Executing background transfer cycle...");
                    
                    for (src, dest) in &all_mappings {
                        let engine = TransferEngine::new(
                            PathBuf::from(src),
                            dest.clone(),
                            tx.clone(),
                            ssh_client.clone(),
                            val_ssh_client.clone(),
                            snapshot_driver.clone(),
                            Some(control_rx.clone()),
                        );
                        if let Err(e) = engine.execute().await {
                            log_event(log_file, &format!("Engine execution failed for {}: {:?}", src, e));
                            tracing::error!("Engine execution failed for {}: {:?}", src, e);
                        }
                    }
                    
                    log_event(log_file, "Cycle completed successfully, waiting for next interval.");
                    
                    let sleep_duration = if transfer_type == "Continuous Sync" {
                        60 // Mock 1 min sync
                    } else {
                        3600 // Mock 1 hr schedule
                    };
                    
                    tracing::info!("Cycle complete. Sleeping for {} seconds...", sleep_duration);
                    tokio::time::sleep(tokio::time::Duration::from_secs(sleep_duration)).await;
                }
            } else {
                log_event("transfers", "Initiating single transfer queue...");
                for (src, dest) in &all_mappings {
                    log_event("transfers", &format!("Transferring {} -> {}", src, dest));
                    let engine = TransferEngine::new(
                        PathBuf::from(src),
                        dest.clone(),
                        tx.clone(),
                        ssh_client.clone(),
                        val_ssh_client.clone(),
                        snapshot_driver.clone(),
                        Some(control_rx.clone()),
                    );
                    if let Err(e) = engine.execute().await {
                        log_event("transfers", &format!("Engine execution failed for {}: {:?}", src, e));
                        tracing::error!("Engine execution failed for {}: {:?}", src, e);
                    }
                }
                log_event("transfers", "Transfer queue completed successfully.");
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
                                    let dest_pub = req.get("pub_key").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    tracing::info!("Received verify request from {}@{}", dest_user, dest_ip);
                                    
                                    if !dest_pub.trim().is_empty() {
                                        tracing::info!("Ingesting dest_pub from TCP handshake");
                                        if let Ok(dest_pub_parsed) = russh_keys::parse_public_key_base64(dest_pub.trim())
                                            && let Some(srv) = P2P_SERVER.get() {
                                                srv.add_authorized_key(dest_pub_parsed).await;
                                                tracing::info!("Authorized key added to global P2P Daemon!");
                                            }
                                        
                                        let my_user = "simply-transfer".to_string();
                                        let res = format!("{{\"status\":\"ok\",\"user\":\"{}\"}}\n", my_user);
                                        let _ = writer.write_all(res.as_bytes()).await;
                                        
                                        let new_host = dest_ip.clone();
                                        tracing::info!("Attempting to map TCP handshake for remote host: {}", new_host);
                                        let _ = slint::invoke_from_event_loop(move || {
                                            if let Some(ui) = ui_handle.upgrade() {
                                                let mut conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
                                                for conn in &mut conns {
                                                    let mut conn_pub = String::new();
                                                    let clean_token = conn.token.to_string();
                                                    let clean_token = clean_token.split(';').next().unwrap_or(&clean_token).trim();
                                                    if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(clean_token) {
                                                        conn_pub = parsed.pub_key;
                                                    }
                                                    let incoming_actual_token = dest_pub.split(';').next().unwrap_or(&dest_pub).trim().to_string();
                                                    
                                                    if conn_pub == incoming_actual_token && !conn_pub.is_empty() {
                                                        tracing::info!("Found connection waiting for peer. Updating state to Connected.");
                                                        conn.host = new_host.clone().into();
                                                        conn.state = "Connected".into();
                                                        conn.state_color = slint::Color::from_rgb_u8(50, 200, 50);
                                                    }
                                                }
                                                ui.set_connections(std::rc::Rc::new(slint::VecModel::from(conns.clone())).into());
                                                ui.set_overall_status("Connection Confirmed".into());
                                                ui.set_remote_verification_status("Connected".into());
                                                
                                                save_connections(&conns);
                                            }
                                        });
                                        return;
                                    } else {
                                        tracing::warn!("Did not receive dest_pub via OOB. Connection invalid.");
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
                
                let my_user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "simply-transfer".to_string());
                let generated_token = simply_transfer_crypto::token::ConnectionToken::generate(&local_ip, port, &pub_key, Some(&my_user));
                conn_to_save.token = generated_token.into();
                conn_to_save.state = "Waiting for peer...".into();
            } else {
                let token = conn_to_save.token.to_string();
                if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&token) {
                    if conn_to_save.host.trim().is_empty() || conn_to_save.host == "Remote" {
                        conn_to_save.host = parsed.ip.clone().into();
                    }
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
        }
    });

    ui.on_browse_source(move |_| {}); // No-op now

    let ui_weak = ui.as_weak();
    ui.on_browse_destination(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_is_browser_open(true);
            ui.set_browser_is_remote(false);
            ui.set_dest_browser_is_remote(true);
            ui.set_browser_target_idx(idx);

            let mappings: Vec<_> = ui.get_current_mappings().iter().collect();
            let idx = idx as usize;

            let mut start_source = "/".to_string();
            if idx < mappings.len() && !mappings[idx].source.is_empty() {
                start_source = mappings[idx].source.to_string();
            }
            ui.set_browser_current_path(start_source.clone().into());
            ui.invoke_fetch_directory(start_source.into(), false, false);

            let mut start_dest = "/".to_string();
            if idx < mappings.len() && !mappings[idx].destination.is_empty() {
                start_dest = mappings[idx].destination.to_string();
            }
            ui.set_dest_browser_current_path(start_dest.clone().into());
            ui.invoke_fetch_directory(start_dest.into(), true, true);
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_fetch_directory(move |path, is_remote, is_dest| {
        let ui_weak = ui_weak.clone();
        let path_str = path.to_string();
        
        let mut token_str = String::new();
        let mut dest_ip = String::new();
        let mut dest_user = String::new();
        let mut t_type = String::new();
        
        if is_remote
            && let Some(ui) = ui_weak.upgrade() {
                let conns: Vec<_> = ui.get_connections().iter().collect();
                let idx = ui.get_selected_connection_idx() as usize;
                if idx < conns.len() {
                    let host = conns[idx].host.to_string();
                    let token = conns[idx].token.to_string();
                    t_type = conns[idx].transfer_type.to_string();
                    token_str = token.clone();
                    
                    if host.contains('@') {
                        let parts: Vec<&str> = host.split('@').collect();
                        dest_user = parts[0].to_string();
                        dest_ip = parts[1].trim().to_string();
                    } else if host == "Remote" {
                        if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&token) {
                            dest_ip = parsed.ip;
                            dest_user = parsed.user.unwrap_or_else(|| "simply-transfer".to_string());
                        }
                    } else {
                        dest_ip = host.clone();
                        dest_user = "simply-transfer".to_string();
                    }
                }
            }
            
        if is_remote && dest_user.is_empty() {
            let _ = notify_rust::Notification::new()
                .summary("Verification Pending")
                .body("Pending Remote Verification: Please verify the token on the destination device, or explicitly provide a username (user@IP).")
                .show();
            return;
        }
        tokio::spawn(async move {
            let mut nodes = Vec::new();
            let mut success = false;
            
            if is_remote {
                
                if !dest_ip.is_empty() {
                    use simply_transfer_core::ssh::SshClient;
                    use simply_transfer_core::russh_client::RusshClient;
                    
                    let mut ssh_client = RusshClient::new();
                    let mut priv_pem_str = String::new();
                    
                    if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(token_str.split(';').next().unwrap_or(&token_str)) {
                        let mgr = simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
                        // Extract the sender's public key ID from the token (second part after semicolon)
                        let private_key_id = match token_str.split(';').nth(1) {
                            Some(key_id) => key_id.to_string(),
                            None => {
                                if t_type == "Remote Transfer" {
                                    let _ = notify_rust::Notification::new()
                                        .summary("Connection Not Verified")
                                        .body("Please verify the connection to exchange keys before fetching the directory.")
                                        .show();
                                    return;
                                } else {
                                    parsed.pub_key.clone()
                                }
                            }
                        };
                        
                        if let Ok(pem) = mgr.get_private_key_pem(&private_key_id) {
                            priv_pem_str = pem;
                        } else {
                            tracing::error!("on_fetch_directory: Failed to retrieve private key from keyring for pub_key {}", private_key_id);
                        }
                    } else {
                        tracing::error!("on_fetch_directory: Failed to parse token {}", token_str);
                    }
                    
                    if priv_pem_str.is_empty() {
                        tracing::error!("on_fetch_directory: Private key not available for authentication.");
                    } else if let Err(e) = ssh_client.connect(&dest_ip, 2222).await {
                        tracing::error!("on_fetch_directory: SSH connect to {} failed: {}", dest_ip, e);
                    } else if let Err(e) = ssh_client.authenticate_publickey(&dest_user, &priv_pem_str, None).await {
                        tracing::error!("on_fetch_directory: SSH auth for {}@{} failed: {}", dest_user, dest_ip, e);
                    } else {
                        let is_windows = ssh_client.execute_command("cmd.exe /c echo Windows")
                            .await
                            .map(|out| out.trim() == "Windows")
                            .unwrap_or(false);
                            
                        let mut resolved_path = path_str.clone();
                        if is_windows && (resolved_path == "/" || resolved_path.is_empty()) {
                            resolved_path = "C:\\".to_string();
                        }
                            
                        let cmd = if is_windows {
                            format!("powershell -NoProfile -Command \"Get-ChildItem -Path '{}' -Force | ForEach-Object {{ if ($_.PSIsContainer) {{ $_.Name + '/' }} else {{ $_.Name }} }}\"", resolved_path)
                        } else {
                            format!("ls -1p \"{}\"", resolved_path)
                        };

                        match ssh_client.execute_command(&cmd).await {
                            Ok(output) => {
                                tracing::info!("PowerShell command executed successfully: {}", cmd);
                                tracing::info!("Raw output length: {}", output.len());
                                tracing::debug!("Raw output content: {:?}", output);
                                success = true;
                                for line in output.lines() {
                                    let line = line.trim();
                                    if line.is_empty() { continue; }
                                    let is_dir = line.ends_with('/');
                                    let name = if is_dir { &line[..line.len()-1] } else { line };
                                    let full_path = if resolved_path.ends_with('/') || resolved_path.ends_with('\\') {
                                        format!("{}{}", resolved_path, name)
                                    } else {
                                        let sep = if is_windows { "\\" } else { "/" };
                                        format!("{}{}{}", resolved_path, sep, name)
                                    };
                                    nodes.push(FileNode {
                                        name: name.clone().into(),
                                        is_dir,
                                        path: full_path.into(),
                                        is_selected: false,
                                        depth: 0,
                                        is_expanded: false,
                                        snapshot_type: get_file_snapshot_type(&name).into(),
                                    });
                                }
                            }
                            Err(e) => {
                                tracing::error!("PowerShell command failed: {} - Error: {}", cmd, e);
                            }
                        }
                    }
                }
                
                if !success {
                    nodes.push(FileNode {
                        name: if dest_ip.is_empty() { "[Invalid Remote Configuration]".into() } else { "[Remote Connection Failed]".into() },
                        is_dir: false,
                        path: path_str.clone().into(),
                        is_selected: false,
                        depth: 0,
                        is_expanded: false,
                        snapshot_type: "".into(),
                    });
                }
            } else if let Ok(entries) = std::fs::read_dir(&path_str) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                    nodes.push(FileNode {
                        snapshot_type: get_file_snapshot_type(&name).into(),
                        name: name.into(),
                        is_dir,
                        path: entry.path().to_string_lossy().to_string().into(),
                        is_selected: false,
                        depth: 0,
                        is_expanded: false,
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
                    if is_dest {
                        ui.set_dest_browser_nodes(model.into());
                        ui.set_dest_browser_current_path(path_str.into());
                    } else {
                        ui.set_browser_nodes(model.into());
                        ui.set_browser_current_path(path_str.into());
                    }
                }
            });
        });
    });

    let ui_weak = ui.as_weak();
    ui.on_collapse_browser_node(move |node, is_dest| {
        if let Some(ui) = ui_weak.upgrade() {
            let node_path = node.path.as_str();
            let browser_nodes: Vec<_> = if is_dest {
                ui.get_dest_browser_nodes().iter().collect()
            } else {
                ui.get_browser_nodes().iter().collect()
            };

            let index = browser_nodes
                .iter()
                .position(|n| n.path.as_str() == node_path);

            if let Some(index) = index {
                let mut new_nodes = Vec::new();
                for i in 0..=index {
                    let mut n = browser_nodes[i].clone();
                    if i == index {
                        n.is_expanded = false;
                    }
                    new_nodes.push(n);
                }

                let current_depth = browser_nodes[index].depth;
                let mut i = index + 1;
                while i < browser_nodes.len() && browser_nodes[i].depth > current_depth {
                    i += 1;
                }

                for j in i..browser_nodes.len() {
                    new_nodes.push(browser_nodes[j].clone());
                }

                if is_dest {
                    ui.set_dest_browser_nodes(
                        std::rc::Rc::new(slint::VecModel::from(new_nodes)).into(),
                    );
                } else {
                    ui.set_browser_nodes(std::rc::Rc::new(slint::VecModel::from(new_nodes)).into());
                }
            }
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_expand_browser_node(move |node, is_remote, is_dest| {
        let node_path = node.path.to_string();
        let ui_weak = ui_weak.clone();
        
        let mut token_str = String::new();
        let mut dest_ip = String::new();
        let mut dest_user = String::new();

        if is_remote && let Some(ui) = ui_weak.upgrade() {
            let conns: Vec<_> = ui.get_connections().iter().collect();
            let idx = ui.get_selected_connection_idx() as usize;
            if idx < conns.len() {
                let host = conns[idx].host.to_string();
                let token = conns[idx].token.to_string();
                token_str = token.clone();
                if host.contains('@') {
                    let parts: Vec<&str> = host.split('@').collect();
                    dest_user = parts[0].to_string();
                    dest_ip = parts[1].trim().to_string();
                } else if host == "Remote" {
                    if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&token) {
                        dest_ip = parsed.ip;
                        dest_user = parsed.user.unwrap_or_else(|| "simply-transfer".to_string());
                    }
                } else {
                    dest_ip = host;
                    dest_user = "simply-transfer".to_string();
                }
            }
        }
        
        if let Some(ui) = ui_weak.upgrade() {
            let browser_nodes: Vec<_> = if is_dest {
                ui.get_dest_browser_nodes().iter().collect()
            } else {
                ui.get_browser_nodes().iter().collect()
            };
            let index = browser_nodes.iter().position(|n| n.path.as_str() == node_path.as_str());
            if let Some(index) = index {
                let mut new_nodes = browser_nodes.clone();
                new_nodes[index].is_expanded = true;
                if is_dest {
                    ui.set_dest_browser_nodes(std::rc::Rc::new(slint::VecModel::from(new_nodes)).into());
                } else {
                    ui.set_browser_nodes(std::rc::Rc::new(slint::VecModel::from(new_nodes)).into());
                }
            }
        }

        tokio::spawn(async move {
            let mut nodes = Vec::new();
            let depth = node.depth + 1;
            
            if is_remote {
                if !dest_ip.is_empty() {
                    use simply_transfer_core::ssh::SshClient;
                    use simply_transfer_core::russh_client::RusshClient;
                    
                    let mut ssh_client = RusshClient::new();
                    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                    let ssh_dir = std::path::Path::new(&home).join(".ssh");
                    let mut tmp_pem = ssh_dir.join("simply-transfer-tmp.pem");
                    
                    if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(token_str.split(';').next().unwrap_or(&token_str)) {
                        let mgr = simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
                        // Extract the sender's public key ID from the token (second part after semicolon)
                        let private_key_id = if let Some(parts) = token_str.split(';').nth(1) {
                            parts.to_string()
                        } else {
                            parsed.pub_key.clone() // fallback to receiver's public key
                        };
                        
                        if let Ok(priv_pem) = mgr.get_private_key_pem(&private_key_id) {
                            let _ = std::fs::write(&tmp_pem, priv_pem.as_bytes());
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                let _ = std::fs::set_permissions(&tmp_pem, std::fs::Permissions::from_mode(0o600));
                            }
                        } else {
                            tmp_pem = ssh_dir.join("simply-transfer-remote.pem");
                        }
                    } else {
                        tmp_pem = ssh_dir.join("simply-transfer-remote.pem");
                    }
                    
                    if let Err(_) = ssh_client.connect(&dest_ip, 2222).await {}
                    else if let Err(_) = ssh_client.authenticate_publickey(&dest_user, &std::fs::read_to_string(&tmp_pem).unwrap_or_default(), None).await {}
                    else {
                        let is_windows = ssh_client.execute_command("cmd.exe /c echo Windows")
                            .await.map(|out| out.trim() == "Windows").unwrap_or(false);
                            
                        let mut resolved_path = node_path.clone();
                        let cmd = if is_windows {
                            if resolved_path.is_empty() { resolved_path = "C:\\".to_string(); }
                            format!("powershell -NoProfile -Command \"Get-ChildItem -Path '{}' | ForEach-Object {{ if ($_.PSIsContainer) {{ $_.Name + '/' }} else {{ $_.Name }} }}\"", resolved_path.replace("'", "''"))
                        } else {
                            if resolved_path.is_empty() { resolved_path = "/".to_string(); }
                            format!("ls -1p '{}'", resolved_path.replace("'", "'\\''"))
                        };

                        if let Ok(output) = ssh_client.execute_command(&cmd).await {
                            for line in output.lines() {
                                let line = line.trim();
                                if line.is_empty() { continue; }
                                let is_dir = line.ends_with('/');
                                let name = if is_dir { &line[..line.len()-1] } else { line };
                                let full_path = if resolved_path.ends_with('/') || resolved_path.ends_with('\\') {
                                    format!("{}{}", resolved_path, name)
                                } else {
                                    let sep = if is_windows { "\\" } else { "/" };
                                    format!("{}{}{}", resolved_path, sep, name)
                                };
                                nodes.push(FileNode {
                                    name: name.into(),
                                    is_dir,
                                    path: full_path.into(),
                                    is_selected: false,
                                    depth,
                                    is_expanded: false,
                                    snapshot_type: get_file_snapshot_type(&name).into(),
                                });
                            }
                        }
                    }
                }
            } else if let Ok(entries) = std::fs::read_dir(&node_path) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                    nodes.push(FileNode {
                        snapshot_type: get_file_snapshot_type(&name).into(),
                        name: name.into(),
                        is_dir,
                        path: entry.path().to_string_lossy().to_string().into(),
                        is_selected: false,
                        depth,
                        is_expanded: false,
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
                    let browser_nodes: Vec<_> = if is_dest {
                        ui.get_dest_browser_nodes().iter().collect()
                    } else {
                        ui.get_browser_nodes().iter().collect()
                    };
                    let index = browser_nodes.iter().position(|n| n.path.as_str() == node_path.as_str());
                    
                    if let Some(index) = index {
                        let mut new_nodes = Vec::new();
                        for i in 0..=index {
                            new_nodes.push(browser_nodes[i].clone());
                        }
                        
                        for n in nodes {
                            new_nodes.push(n);
                        }
                        
                        for i in index + 1..browser_nodes.len() {
                            new_nodes.push(browser_nodes[i].clone());
                        }
                        
                        if is_dest {
                            ui.set_dest_browser_nodes(std::rc::Rc::new(slint::VecModel::from(new_nodes)).into());
                        } else {
                            ui.set_browser_nodes(std::rc::Rc::new(slint::VecModel::from(new_nodes)).into());
                        }
                    }
                }
            });
        });
    });

    let ui_weak = ui.as_weak();
    ui.on_commit_browser_selection(move |source_paths, dest_paths| {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_is_browser_open(false);
            let idx = ui.get_browser_target_idx() as usize;
            let mut mappings: Vec<_> = ui.get_current_mappings().iter().collect();

            if idx < mappings.len() {
                let dest = if dest_paths.row_count() > 0 {
                    dest_paths.row_data(0).unwrap().to_string()
                } else {
                    ui.get_dest_browser_current_path().to_string()
                };

                if source_paths.row_count() > 0 {
                    mappings[idx].source = source_paths.row_data(0).unwrap();
                    mappings[idx].destination = dest.clone().into();

                    for i in 1..source_paths.row_count() {
                        mappings.insert(
                            idx + i,
                            DirectoryMapping {
                                source: source_paths.row_data(i).unwrap(),
                                destination: dest.clone().into(),
                            },
                        );
                    }
                } else {
                    mappings[idx].source = ui.get_browser_current_path();
                    mappings[idx].destination = dest.clone().into();
                }

                ui.set_current_mappings(std::rc::Rc::new(slint::VecModel::from(mappings)).into());
            }
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_toggle_browser_selection(move |path, is_dest| {
        if let Some(ui) = ui_weak.upgrade() {
            let path_str = path.to_string();

            let nodes: Vec<_> = if is_dest {
                ui.get_dest_browser_nodes().iter().collect()
            } else {
                ui.get_browser_nodes().iter().collect()
            };

            let mut updated_nodes = Vec::new();

            for mut node in nodes {
                if node.path == path_str {
                    node.is_selected = !node.is_selected;
                } else if is_dest {
                    node.is_selected = false;
                }
                updated_nodes.push(node);
            }

            let selected_paths: Vec<slint::SharedString> = updated_nodes
                .iter()
                .filter(|node| node.is_selected)
                .map(|node| node.path.clone())
                .collect();

            let nodes_model = std::rc::Rc::new(slint::VecModel::from(updated_nodes));
            let selection_model = std::rc::Rc::new(slint::VecModel::from(selected_paths));

            if is_dest {
                ui.set_dest_browser_nodes(nodes_model.into());
                ui.set_dest_browser_selection(selection_model.into());
            } else {
                ui.set_browser_nodes(nodes_model.into());
                ui.set_browser_selection(selection_model.into());
            }
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_add_mapping(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let mut mappings: Vec<_> = ui.get_current_mappings().iter().collect();
            mappings.push(DirectoryMapping {
                source: "".into(),
                destination: "".into(),
            });
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



    let ui_weak = ui.as_weak();
    ui.on_delete_connection(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
            let idx = idx as usize;
            if idx < conns.len() {
                let token = conns[idx].token.to_string();
                if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&token) {
                    let key_mgr =
                        simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
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
                    
                    #[cfg(target_os = "windows")]
                    {
                        let admin_keys = "C:\\ProgramData\\ssh\\administrators_authorized_keys";
                        let mut admin_key_exists = false;
                        if let Ok(content) = std::fs::read_to_string(admin_keys) {
                            if content.contains(&parsed.pub_key) {
                                admin_key_exists = true;
                            }
                        }
                        if !admin_key_exists {
                            use std::io::Write;
                            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(admin_keys) {
                                let _ = writeln!(file, "{}", pub_key_line);
                                log_event("connection", "Appended public key to administrators_authorized_keys.");
                            } else {
                                let script = format!(
                                    "Add-Content -Path \\\"{}\\\" -Value \\\"{}\\\"",
                                    admin_keys, pub_key_line
                                );
                                let _ = std::process::Command::new("powershell")
                                    .arg("-NoProfile")
                                    .arg("-Command")
                                    .arg(&format!("Start-Process powershell -ArgumentList '-NoProfile -Command {}' -Verb RunAs -WindowStyle Hidden -Wait", script))
                                    .status();
                                log_event("connection", "Appended public key to administrators_authorized_keys via elevated PowerShell.");
                            }
                        }
                    }
                    
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(&ssh_dir, std::fs::Permissions::from_mode(0o700));
                        let _ = std::fs::set_permissions(&auth_keys, std::fs::Permissions::from_mode(0o600));
                        let _ = std::process::Command::new("/sbin/restorecon").arg("-R").arg(&ssh_dir).status();
                    }
                    
                    // Trigger remote handshake (TCP connect to source device)
                    let addr = format!("{}:{}", parsed.ip, parsed.port);
                    log_event("connection", &format!("Attempting TCP handshake with source at {}", addr));
                    
                    // Generate our own key pair and transmit it over SSH
                    let key_mgr = simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
                    let mut my_pub_key_id = String::new();
                    if let Ok(pub_b) = key_mgr.generate_and_store() {
                        my_pub_key_id = pub_b.clone();
                    }
                    
                    // Use a short timeout for the handshake
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(15),
                        tokio::net::TcpStream::connect(&addr)
                    ).await {
                        Ok(Ok(mut stream)) => {
                            use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
                            let dest_user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "simply-transfer".to_string());
                            let req = format!("{{\"action\":\"verify\",\"user\":\"{}\",\"pub_key\":\"{}\"}}\n", dest_user, my_pub_key_id);
                            let _ = stream.write_all(req.as_bytes()).await;
                            let mut reader = tokio::io::BufReader::new(stream);
                            let mut line = String::new();
                            let read_result = tokio::time::timeout(
                                std::time::Duration::from_secs(10),
                                reader.read_line(&mut line)
                            ).await;
                            
                            if let Ok(Ok(_)) = read_result {
                                if line.contains("\"ok\"") {
                                    log_event("connection", "Remote token validated and handshake succeeded.");
                                
                                // INJECT RECEIVER'S PUBLIC KEY INTO SENDER'S DAEMON
                                tracing::info!("Ingesting receiver's pub_key into P2P Daemon!");
                                if let Ok(dest_pub_parsed) = russh_keys::parse_public_key_base64(parsed.pub_key.trim())
                                    && let Some(srv) = P2P_SERVER.get() {
                                        let srv_clone = srv.clone();
                                        tokio::spawn(async move {
                                            srv_clone.add_authorized_key(dest_pub_parsed).await;
                                            tracing::info!("Receiver's authorized key added to global P2P Daemon!");
                                        });
                                    }

                                if let Ok(res) = serde_json::from_str::<serde_json::Value>(&line) {
                                    let remote_user = res["user"].as_str().unwrap_or("simply-transfer").to_string();
                                    let new_host = format!("{}@{}", remote_user, parsed.ip);
                                    let clone_ui = ui_weak.clone();
                                    let token_clone = token.clone();
                                    tracing::info!("Successfully verified remote token. Updating connection host to: {}", new_host);
                                    let appended_token = if !my_pub_key_id.is_empty() { format!("{};{}", token_clone, my_pub_key_id) } else { token_clone.clone() };
                                    let _ = slint::invoke_from_event_loop(move || {
                                        if let Some(ui) = clone_ui.upgrade() {
                                            ui.set_remote_verified_host(new_host.clone().into());
                                            let mut conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
                                            for conn in &mut conns {
                                                let conn_actual_token = conn.token.to_string();
                                                let conn_actual_token = conn_actual_token.split(';').next().unwrap_or(&conn_actual_token).trim();
                                                let incoming_actual_token = token_clone.split(';').next().unwrap_or(&token_clone).trim();
                                                if conn_actual_token == incoming_actual_token {
                                                    tracing::info!("Updating UI connection to Connected state.");
                                                    conn.host = new_host.clone().into();
                                                    conn.token = appended_token.clone().into();
                                                    conn.state = "Connected".into();
                                                    conn.state_color = slint::Color::from_rgb_u8(50, 200, 50);
                                                }
                                            }
                                            ui.set_remote_verification_status("Verified".into());
                                            ui.set_remote_verified_full_token(appended_token.clone().into());
                                            ui.set_connections(std::rc::Rc::new(slint::VecModel::from(conns)).into());
                                        }
                                    });
                                } else {
                                    log_event("connection", "Remote handshake failed or was rejected.");
                                }
                            } else {
                                log_event("connection", "Remote handshake read timed out.");
                            }
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
                    let key_mgr =
                        simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
                    if let Err(e) = key_mgr.delete_key(&parsed.pub_key) {
                        tracing::warn!("Failed to delete key: {}", e);
                    } else {
                        println!(
                            "Successfully deleted key from OS keyring for connection: {}",
                            conns[idx].name
                        );
                    }
                }
            }
        }
    });

    ui.on_cancel_key_generation(move |pub_key| {
        let pub_key = pub_key.to_string();
        if !pub_key.is_empty() {
            let key_mgr =
                simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
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
            let next = if current == ControlSignal::Pause {
                ControlSignal::Run
            } else {
                ControlSignal::Pause
            };
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

    let ui_weak = ui.as_weak();
    // Initialize Schedule Tab UI
    if let Ok(contents) = std::fs::read_to_string("schedules.json") {
        if let Ok(schedules) = serde_json::from_str::<Vec<serde_json::Value>>(&contents) {
            let mut slint_schedules = Vec::new();
            for (idx, schedule) in schedules.iter().enumerate() {
                let transfer_type = schedule["transfer_type"].as_str().unwrap_or("").to_string();
                if let Some(arr) = schedule["mappings"].as_array() {
                    for item in arr {
                        if let (Some(src), Some(dest)) = (
                            item.get(0).and_then(|v| v.as_str()),
                            item.get(1).and_then(|v| v.as_str()),
                        ) {
                            slint_schedules.push(ScheduleData {
                                id: format!("{}", idx).into(),
                                source: src.into(),
                                dest: dest.into(),
                                r#type: transfer_type.clone().into(),
                            });
                        }
                    }
                }
            }
            ui.set_schedules(std::rc::Rc::new(slint::VecModel::from(slint_schedules)).into());
        }
    }

    ui.on_delete_schedule(move |id| {
        if let Ok(contents) = std::fs::read_to_string("schedules.json") {
            if let Ok(mut schedules) = serde_json::from_str::<Vec<serde_json::Value>>(&contents) {
                if let Ok(idx) = id.as_str().parse::<usize>() {
                    if idx < schedules.len() {
                        schedules.remove(idx);
                        if let Ok(json_string) = serde_json::to_string_pretty(&schedules) {
                            let _ = std::fs::write("schedules.json", json_string);
                        }

                        // Update UI model
                        let mut slint_schedules = Vec::new();
                        for (new_idx, schedule) in schedules.iter().enumerate() {
                            let transfer_type =
                                schedule["transfer_type"].as_str().unwrap_or("").to_string();
                            if let Some(arr) = schedule["mappings"].as_array() {
                                for item in arr {
                                    if let (Some(src), Some(dest)) = (
                                        item.get(0).and_then(|v| v.as_str()),
                                        item.get(1).and_then(|v| v.as_str()),
                                    ) {
                                        slint_schedules.push(ScheduleData {
                                            id: format!("{}", new_idx).into(),
                                            source: src.into(),
                                            dest: dest.into(),
                                            r#type: transfer_type.clone().into(),
                                        });
                                    }
                                }
                            }
                        }
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_schedules(
                                std::rc::Rc::new(slint::VecModel::from(slint_schedules)).into(),
                            );
                        }
                    }
                }
            }
        }
    });

    // Load and run persistent schedules
    {
        let global_tx_clone = global_tx.clone();
        tokio::spawn(async move {
            if let Ok(contents) = std::fs::read_to_string("schedules.json") {
                if let Ok(schedules) = serde_json::from_str::<Vec<serde_json::Value>>(&contents) {
                    for schedule in schedules {
                        let transfer_type =
                            schedule["transfer_type"].as_str().unwrap_or("").to_string();
                        let mappings: Vec<(String, String)> = schedule["mappings"]
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|item| {
                                        if let (Some(src), Some(dest)) = (
                                            item.get(0).and_then(|v| v.as_str()),
                                            item.get(1).and_then(|v| v.as_str()),
                                        ) {
                                            Some((src.to_string(), dest.to_string()))
                                        } else {
                                            None
                                        }
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();

                        let sleep_duration = schedule["sleep_duration"].as_u64().unwrap_or(3600);

                        tracing::info!("Restoring saved schedule: {}", transfer_type);

                        let tx_inner = global_tx_clone.clone();
                        let log_file = if transfer_type == "Continuous Sync" {
                            "sync"
                        } else {
                            "schedule"
                        };

                        tokio::spawn(async move {
                            loop {
                                tracing::info!("Executing background transfer cycle...");
                                for (src, dest) in &mappings {
                                    let (tx, mut rx) = tokio::sync::mpsc::channel(1000);
                                    let g_tx = tx_inner.clone();
                                    tokio::spawn(async move {
                                        while let Some(event) = rx.recv().await {
                                            let _ = g_tx.send(event);
                                        }
                                    });
                                    // For restored schedules, we run without ssh_client (local sync or we'd need to re-auth)
                                    let snapshot_driver =
                                        Arc::new(simply_transfer_snapshots::FallbackSnapshotDriver);
                                    let engine = TransferEngine::new(
                                        PathBuf::from(src),
                                        dest.clone(),
                                        tx,
                                        Arc::new(
                                            simply_transfer_core::russh_client::RusshClient::new(),
                                        ),
                                        Arc::new(
                                            simply_transfer_core::russh_client::RusshClient::new(),
                                        ),
                                        snapshot_driver,
                                        None,
                                    );
                                    if let Err(e) = engine.execute().await {
                                        tracing::error!(
                                            "Engine execution failed for {}: {:?}",
                                            src,
                                            e
                                        );
                                    }
                                }
                                tracing::info!(
                                    "Cycle complete. Sleeping for {} seconds...",
                                    sleep_duration
                                );
                                tokio::time::sleep(tokio::time::Duration::from_secs(
                                    sleep_duration,
                                ))
                                .await;
                            }
                        });
                    }
                }
            }
        });
    }

    ui.run()?;

    // Explicitly drop the context guard
    drop(_guard);

    // Bypass the Tokio Thread Local drop panics by issuing a hard OS-level process exit
    std::process::exit(0);
}
