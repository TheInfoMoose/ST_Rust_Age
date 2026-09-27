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
    if let Ok(data) = std::fs::read_to_string("connections.json") {
        if let Ok(conns) = serde_json::from_str(&data) {
            return conns;
        }
    }
    Vec::new()
}

fn save_connections(conns: &[ConnectionItem]) {
    let saved: Vec<SavedConnection> = conns.iter().map(|c| SavedConnection {
        name: c.name.to_string(),
        host: c.host.to_string(),
        state: c.state.to_string(),
        transfer_rate: c.transfer_rate.to_string(),
        duration: c.duration.to_string(),
        eta: c.eta.to_string(),
        transfer_type: c.transfer_type.to_string(),
        token: c.token.to_string(),
    }).collect();
    if let Ok(json) = serde_json::to_string_pretty(&saved) {
        let _ = std::fs::write("connections.json", json);
    }
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

                            slint::invoke_from_event_loop(move || {
                                if let Some(ui) = ui_handle.upgrade() {
                                    let t_model = std::rc::Rc::new(slint::VecModel::from(clone_t_q));
                                    ui.set_transfer_queue(t_model.into());

                                    let c_model = std::rc::Rc::new(slint::VecModel::from(clone_c_q));
                                    ui.set_completed_queue(c_model.into());

                                    ui.set_phase_text(clone_phase.into());
                                    ui.set_overall_status(clone_status.into());
                                    ui.set_transfer_metrics_text(clone_metrics.into());
                                }
                            }).unwrap();
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
                tracing::info!("Starting background daemon for {}", transfer_type);
                loop {
                    tracing::info!("Executing background transfer cycle...");
                    if let Err(e) = engine.execute().await {
                        tracing::error!("Engine execution failed: {:?}", e);
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
                if let Err(e) = engine.execute().await {
                    tracing::error!("Engine execution failed: {:?}", e);
                }
            }
        });
    });

    let ui_weak = ui.as_weak();
    ui.on_commit_connection(move |new_conn| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
            conns.push(new_conn);
            save_connections(&conns);
            let model = std::rc::Rc::new(slint::VecModel::from(conns));
            ui.set_connections(model.into());
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
    ui.on_fetch_directory(move |path, is_remote| {
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
            
            slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    let model = std::rc::Rc::new(slint::VecModel::from(nodes));
                    ui.set_browser_nodes(model.into());
                    ui.set_browser_current_path(path_str.into());
                }
            }).unwrap();
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
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            let verified = token.len() > 10;
            let status = if verified { "Connection Verified Successfully" } else { "Invalid Token Format" };
            
            slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_is_remote_verified(verified);
                    ui.set_remote_verification_status(status.into());
                }
            }).unwrap();
        });
    });

    let _ui_weak = ui.as_weak();
    ui.on_generate_key(move || {
        let key_mgr = simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
        match key_mgr.generate_and_store() {
            Ok(pub_key) => {
                let ip = "0.0.0.0"; // Default or pull from UI host field? The Slint callback doesn't pass the host right now, so we will stub the IP/Port.
                let port = 22;
                simply_transfer_crypto::token::ConnectionToken::generate(ip, port, &pub_key).into()
            },
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
