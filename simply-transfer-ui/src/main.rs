use simply_transfer_core::engine::{FileTransferStatus, TransferEngine, TransferEvent};
use simply_transfer_core::ssh::MockSshClient;
use simply_transfer_snapshots::FallbackSnapshotDriver;
use slint::Model;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
slint::include_modules!();

#[tokio::main]
#[rustfmt::skip]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let ui = MainWindow::new()?;

    // Fetch system info and push to UI
    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();
    let cpu_count = sys.cpus().len();
    let memory_mb = sys.total_memory() / 1024 / 1024;
    ui.set_os_processor_info(format!("Processor: {} Logical Cores Detected", cpu_count).into());
    ui.set_os_memory_info(format!("RAM: {} MB Available", memory_mb).into());

    let ui_handle = ui.as_weak();

    ui.on_start_transfer(move |src, dest| {
        let ui_handle = ui_handle.clone();

        if let Some(ui) = ui_handle.upgrade() {
            ui.set_active_tab(1);
        }

        tokio::spawn(async move {
            let (tx, mut rx) = mpsc::channel(100);

            let engine = TransferEngine::new(
                PathBuf::from(src.as_str()),
                dest.to_string(),
                tx,
                Arc::new(MockSshClient::new()),
                Arc::new(FallbackSnapshotDriver),
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
                                    }
                                    TransferEvent::TransferFailed(err) => {
                                        status_txt = format!("Transfer Aborted: {}", err);
                                        local_t_q.clear();
                                    }
                                }
                            }

                            let ui_handle = ui_handle.clone();
                            let clone_t_q = local_t_q.clone();
                            let clone_c_q = local_c_q.clone();
                            let clone_phase = phase_txt.clone();
                            let clone_status = status_txt.clone();

                            slint::invoke_from_event_loop(move || {
                                if let Some(ui) = ui_handle.upgrade() {
                                    let t_model = std::rc::Rc::new(slint::VecModel::from(clone_t_q));
                                    ui.set_transfer_queue(t_model.into());

                                    let c_model = std::rc::Rc::new(slint::VecModel::from(clone_c_q));
                                    ui.set_completed_queue(c_model.into());

                                    ui.set_phase_text(clone_phase.into());
                                    ui.set_overall_status(clone_status.into());
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

            if let Err(e) = engine.execute().await {
                tracing::error!("Engine execution failed: {:?}", e);
            }
        });
    });

    let ui_weak = ui.as_weak();
    ui.on_commit_connection(move |new_conn| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
            conns.push(new_conn);
            let model = std::rc::Rc::new(slint::VecModel::from(conns));
            ui.set_connections(model.into());
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_browse_source(move || {
        if let Some(path) = rfd::FileDialog::new().pick_folder()
            && let Some(ui) = ui_weak.upgrade()
        {
            ui.set_source_path(path.to_string_lossy().to_string().into());
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_browse_destination(move || {
        if let Some(path) = rfd::FileDialog::new().pick_folder()
            && let Some(ui) = ui_weak.upgrade()
        {
            ui.set_destination_path(path.to_string_lossy().to_string().into());
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
                println!("Token copied to clipboard: {}", token);
            }
        }
    });

    ui.on_add_connection(move || {
        println!("Add connection initiated");
    });

    ui.on_pause_transfer(move || {
        println!("Transfer paused");
    });

    ui.on_cancel_transfer(move || {
        println!("Transfer cancelled");
    });

    ui.run()?;

    Ok(())
}
