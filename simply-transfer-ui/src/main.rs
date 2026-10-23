#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::redundant_pattern_matching)]
#![allow(clippy::collapsible_if)]
#![allow(unused_assignments, unused_variables)]
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

fn load_job_registry() -> simply_transfer_core::Registry {
    let filename = format!("transfer_jobs_{}.json", std::env::consts::OS);
    if let Ok(data) = std::fs::read_to_string(&filename)
        && let Ok(reg) = serde_json::from_str::<simply_transfer_core::Registry>(&data)
    {
        return reg;
    }
    simply_transfer_core::Registry::new()
}

fn save_job_registry(registry: &simply_transfer_core::Registry) {
    if let Ok(json) = serde_json::to_string_pretty(registry) {
        let filename = format!("transfer_jobs_{}.json", std::env::consts::OS);
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
    let dir = std::path::Path::new(&dir_name);
    if !dir.exists() {
        let _ = std::fs::create_dir_all(dir);
    }
    let path = format!("{}/{}", dir_name, filename);
    if !std::path::Path::new(&path).exists() {
        let _ = std::fs::write(&path, "");
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", &path])
            .creation_flags(0x08000000)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(&path).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
}

fn open_url_in_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(0x08000000)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
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

fn get_user_home_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
}

fn permissions_state_path() -> std::path::PathBuf {
    get_user_home_dir()
        .join(".simply-transfer")
        .join("permissions_state.json")
}

fn is_os_permissions_configured() -> bool {
    if let Ok(content) = std::fs::read_to_string(permissions_state_path())
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
    {
        return val
            .get("granted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    }
    false
}

fn save_os_permissions_configured() {
    let path = permissions_state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let payload = serde_json::json!({
        "granted": true,
        "os": std::env::consts::OS,
        "updated_at": chrono::Local::now().to_rfc3339(),
    });
    if let Ok(s) = serde_json::to_string_pretty(&payload) {
        let _ = std::fs::write(path, s);
    }
}

fn normalize_browse_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "/".to_string();
    }
    let is_win = trimmed.contains('\\') || (trimmed.len() >= 2 && trimmed.as_bytes()[1] == b':');

    if let Some(prefix) = trimmed
        .strip_suffix("/..")
        .or_else(|| trimmed.strip_suffix("\\.."))
    {
        let cleaned = prefix.trim_end_matches(['/', '\\']);
        if cleaned.is_empty() {
            return if is_win {
                "C:\\".to_string()
            } else {
                "/".to_string()
            };
        }
        if is_win && cleaned.len() == 2 && cleaned.as_bytes()[1] == b':' {
            return format!("{}\\", cleaned);
        }
        if let Some(idx) = cleaned.rfind(['/', '\\']) {
            if idx == 0 {
                return "/".to_string();
            }
            let parent = &cleaned[..idx];
            if is_win && parent.len() == 2 && parent.as_bytes()[1] == b':' {
                return format!("{}\\", parent);
            }
            return parent.to_string();
        }
        return if is_win {
            "C:\\".to_string()
        } else {
            "/".to_string()
        };
    }

    trimmed.to_string()
}

fn clean_key_identifier(id: &str) -> String {
    let parts: Vec<&str> = id.split_whitespace().collect();
    if parts.len() >= 2 {
        parts[1].to_string()
    } else {
        id.trim().to_string()
    }
}

fn resolve_private_key_pem(
    mgr: &simply_transfer_crypto::keys::KeyPairManager,
    key_id: &str,
    fallback_id: &str,
) -> Option<String> {
    let clean_key = clean_key_identifier(key_id);
    let clean_fallback = clean_key_identifier(fallback_id);

    if !clean_key.is_empty() {
        if let Ok(pem) = mgr.get_private_key_pem(&clean_key) {
            return Some(pem);
        }
    }
    if !clean_fallback.is_empty() && clean_fallback != clean_key {
        if let Ok(pem) = mgr.get_private_key_pem(&clean_fallback) {
            return Some(pem);
        }
    }
    None
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Sanitize any legacy plaintext temporary PEM files from disk to prevent key leakage
    let ssh_dir = get_user_home_dir().join(".ssh");
    for legacy_name in &["simply-transfer-tmp.pem", "simply-transfer-remote.pem"] {
        let legacy_file = ssh_dir.join(legacy_name);
        if legacy_file.exists() {
            let _ = std::fs::remove_file(legacy_file);
        }
    }

    // Create a dedicated Tokio runtime, separating it from the Slint event loop
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    // Enter the runtime context so `tokio::spawn` works in Slint callbacks
    let _guard = rt.enter();

    let (global_tx, mut global_rx) =
        tokio::sync::broadcast::channel::<simply_transfer_core::engine::TransferEvent>(1000);

    let host_key = simply_transfer_core::russh_server::generate_host_key();
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

    fn sync_transfer_queue_in_place(ui: &MainWindow, items: &[TransferQueueItem]) {
        let model = ui.get_transfer_queue();
        if let Some(vec_model) = model
            .as_any()
            .downcast_ref::<slint::VecModel<TransferQueueItem>>()
        {
            for (i, item) in items.iter().enumerate() {
                if i < vec_model.row_count() {
                    vec_model.set_row_data(i, item.clone());
                } else {
                    vec_model.push(item.clone());
                }
            }
            while vec_model.row_count() > items.len() {
                vec_model.remove(vec_model.row_count() - 1);
            }
        } else {
            let persistent: std::rc::Rc<slint::VecModel<TransferQueueItem>> =
                std::rc::Rc::new(slint::VecModel::from(items.to_vec()));
            ui.set_transfer_queue(persistent.into());
        }
    }

    fn sync_completed_queue_in_place(ui: &MainWindow, items: &[CompletedItem]) {
        let model = ui.get_completed_queue();
        if let Some(vec_model) = model
            .as_any()
            .downcast_ref::<slint::VecModel<CompletedItem>>()
        {
            for (i, item) in items.iter().enumerate() {
                if i < vec_model.row_count() {
                    vec_model.set_row_data(i, item.clone());
                } else {
                    vec_model.push(item.clone());
                }
            }
            while vec_model.row_count() > items.len() {
                vec_model.remove(vec_model.row_count() - 1);
            }
        } else {
            let persistent: std::rc::Rc<slint::VecModel<CompletedItem>> =
                std::rc::Rc::new(slint::VecModel::from(items.to_vec()));
            ui.set_completed_queue(persistent.into());
        }
    }

    fn connection_uuid_for_item(item: &ConnectionItem) -> uuid::Uuid {
        let seed = format!("{}|{}", item.name.as_str(), item.host.as_str());
        let digest = simply_transfer_crypto::hash::compute_sha256_bytes(seed.as_bytes());
        let mut bytes = [0u8; 16];
        for (i, chunk) in digest.as_bytes().chunks(2).take(16).enumerate() {
            let hex_str = std::str::from_utf8(chunk).unwrap_or("00");
            bytes[i] = u8::from_str_radix(hex_str, 16).unwrap_or(0);
        }
        uuid::Uuid::from_bytes(bytes)
    }

    fn active_connection_uuid(ui: &MainWindow) -> uuid::Uuid {
        let conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
        let idx = ui.get_selected_connection_idx() as usize;
        if idx < conns.len() {
            connection_uuid_for_item(&conns[idx])
        } else {
            uuid::Uuid::nil()
        }
    }

    const DAY_TOKENS: [&str; 7] = ["M", "T", "W", "Th", "F", "Sa", "Su"];

    fn parse_active_days_flags(active_days: &str) -> Vec<bool> {
        let stripped = active_days
            .replace("Th", "")
            .replace("Sa", "")
            .replace("Su", "");
        vec![
            active_days.contains("M"),
            stripped.contains('T'),
            active_days.contains("W"),
            active_days.contains("Th"),
            active_days.contains("F"),
            active_days.contains("Sa"),
            active_days.contains("Su"),
        ]
    }

    fn format_active_days_flags(flags: &[bool]) -> String {
        let mut out = String::new();
        for (i, &enabled) in flags.iter().enumerate().take(DAY_TOKENS.len()) {
            if enabled {
                out.push_str(DAY_TOKENS[i]);
            }
        }
        out
    }

    fn populate_inline_browsers(ui: &MainWindow) {
        ui.set_browser_is_remote(false);
        ui.set_dest_browser_is_remote(true);

        let default_local_root = if cfg!(windows) {
            "C:\\".to_string()
        } else {
            "/".to_string()
        };

        let mut first_local_mount = default_local_root.clone();
        if let Ok(vols) = simply_transfer_core::TransferEngine::list_local_volumes() {
            let mut labels: Vec<slint::SharedString> = Vec::new();
            let ui_vols: Vec<VolumeUi> = vols
                .into_iter()
                .map(|v| {
                    let mp = v.mount_point.to_string_lossy().to_string();
                    labels.push(mp.clone().into());
                    let free_gb = v.available_bytes as f64 / 1_073_741_824.0;
                    VolumeUi {
                        mount_point: mp.into(),
                        label: v.label.into(),
                        fs_type: v.fs_type.into(),
                        free_space_label: format!("{:.1} GB free", free_gb).into(),
                    }
                })
                .collect();
            if let Some(first) = labels.first() {
                first_local_mount = first.to_string();
            } else {
                labels.push(default_local_root.clone().into());
            }
            ui.set_local_volume_labels(std::rc::Rc::new(slint::VecModel::from(labels)).into());
            ui.set_local_volumes(std::rc::Rc::new(slint::VecModel::from(ui_vols)).into());
        } else {
            ui.set_local_volume_labels(
                std::rc::Rc::new(slint::VecModel::from(vec![slint::SharedString::from(
                    default_local_root.clone(),
                )]))
                .into(),
            );
        }
        ui.set_current_local_volume(first_local_mount.clone().into());

        if ui.get_remote_volume_labels().row_count() == 0 {
            ui.set_remote_volume_labels(
                std::rc::Rc::new(slint::VecModel::from(vec![slint::SharedString::from("/")]))
                    .into(),
            );
            ui.set_current_remote_volume("/".into());
        }

        let mappings: Vec<_> = ui.get_current_mappings().iter().collect();
        let start_source = mappings
            .first()
            .map(|m| m.source.to_string())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(first_local_mount);

        let start_dest = mappings
            .first()
            .map(|m| m.destination.to_string())
            .filter(|d| !d.trim().is_empty())
            .unwrap_or_else(|| "/".to_string());

        ui.set_browser_current_path(start_source.clone().into());
        ui.invoke_fetch_directory(start_source.into(), false, false);

        ui.set_dest_browser_current_path(start_dest.clone().into());
        ui.invoke_fetch_directory(start_dest.into(), true, true);
    }

    fn reset_draft_job_for_connection(ui: &MainWindow, conn_id: uuid::Uuid) {
        let clean_draft = TransferJobUi {
            job_id: uuid::Uuid::new_v4().to_string().into(),
            connection_id: conn_id.to_string().into(),
            name: "Default Transfer Task".into(),
            job_type: "Single Transfer".into(),
            source: "".into(),
            destination: "".into(),
            schedule_mode: "Manual".into(),
            sync_frequency: "1hr".into(),
            start_permitted: "00:00".into(),
            finish_permitted: "23:59".into(),
            active_days: "MTWThF".into(),
            use_snapshots: true,
        };
        ui.set_draft_job(clean_draft);
        ui.set_active_days_flags(
            std::rc::Rc::new(slint::VecModel::from(parse_active_days_flags("MTWThF"))).into(),
        );
        ui.set_selected_job_idx(-1);
        ui.set_open_menu_job_idx(-1);
        ui.set_editing_job_idx(-1);
        ui.set_is_naming_new_job(false);
        ui.set_new_job_name_draft("".into());
        ui.set_is_job_setup_unlocked(false);
        ui.set_current_mappings(
            std::rc::Rc::new(slint::VecModel::from(Vec::<DirectoryMapping>::new())).into(),
        );
    }

    fn load_job_into_draft_ui(
        ui: &MainWindow,
        registry: &simply_transfer_core::Registry,
        selected: &TransferJobUi,
    ) {
        let mut ui_mappings: Vec<DirectoryMapping> = Vec::new();
        if let Ok(job_uuid) = uuid::Uuid::parse_str(selected.job_id.as_str())
            && let Some(job) = registry.jobs.get(&job_uuid)
        {
            if !job.mappings.is_empty() {
                for (src, dst) in &job.mappings {
                    let s = src.to_string_lossy().to_string();
                    let d = dst.to_string_lossy().to_string();
                    if !s.trim().is_empty() || !d.trim().is_empty() {
                        ui_mappings.push(DirectoryMapping {
                            source: s.into(),
                            destination: d.into(),
                        });
                    }
                }
            } else if !job.source_paths.is_empty() {
                let dst = job.destination_path.to_string_lossy().to_string();
                for src in &job.source_paths {
                    let s = src.to_string_lossy().to_string();
                    if !s.trim().is_empty() || !dst.trim().is_empty() {
                        ui_mappings.push(DirectoryMapping {
                            source: s.into(),
                            destination: dst.clone().into(),
                        });
                    }
                }
            }
        }
        if ui_mappings.is_empty()
            && (!selected.source.trim().is_empty() || !selected.destination.trim().is_empty())
        {
            ui_mappings.push(DirectoryMapping {
                source: selected.source.clone(),
                destination: selected.destination.clone(),
            });
        }
        ui.set_current_mappings(std::rc::Rc::new(slint::VecModel::from(ui_mappings)).into());
        ui.set_active_days_flags(
            std::rc::Rc::new(slint::VecModel::from(parse_active_days_flags(
                selected.active_days.as_str(),
            )))
            .into(),
        );
        ui.set_draft_job(selected.clone());
        ui.set_is_naming_new_job(false);
        ui.set_is_job_setup_unlocked(true);
    }

    fn sync_connection_jobs_ui(
        ui: &MainWindow,
        registry: &simply_transfer_core::Registry,
        conn_id: uuid::Uuid,
    ) {
        let jobs = registry.get_jobs_for_connection(conn_id);
        let ui_jobs: Vec<TransferJobUi> = jobs
            .into_iter()
            .map(|job| {
                let (first_src, dest) = if let Some((s, d)) = job.mappings.first() {
                    (
                        s.to_string_lossy().to_string(),
                        d.to_string_lossy().to_string(),
                    )
                } else {
                    (
                        job.source_paths
                            .first()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default(),
                        job.destination_path.to_string_lossy().to_string(),
                    )
                };
                let schedule_mode = job
                    .schedule
                    .as_ref()
                    .map(|s| s.mode.clone())
                    .unwrap_or_else(|| "Single Transfer".to_string());
                let sync_freq = job
                    .schedule
                    .as_ref()
                    .and_then(|s| s.sync_frequency.clone())
                    .unwrap_or_else(|| "1hr".to_string());
                let start_perm = job
                    .schedule
                    .as_ref()
                    .and_then(|s| s.window_start.clone())
                    .unwrap_or_else(|| "00:00".to_string());
                let finish_perm = job
                    .schedule
                    .as_ref()
                    .and_then(|s| s.window_end.clone())
                    .unwrap_or_else(|| "23:59".to_string());
                let active_days = job
                    .schedule
                    .as_ref()
                    .and_then(|s| s.active_days.clone())
                    .unwrap_or_else(|| "MTWThF".to_string());
                TransferJobUi {
                    job_id: job.job_id.to_string().into(),
                    connection_id: job.connection_id.to_string().into(),
                    name: job.name.into(),
                    job_type: schedule_mode.clone().into(),
                    source: first_src.into(),
                    destination: dest.into(),
                    schedule_mode: schedule_mode.into(),
                    sync_frequency: sync_freq.into(),
                    start_permitted: start_perm.into(),
                    finish_permitted: finish_perm.into(),
                    active_days: active_days.into(),
                    use_snapshots: job.use_snapshots,
                }
            })
            .collect();
        ui.set_connection_jobs(std::rc::Rc::new(slint::VecModel::from(ui_jobs)).into());
    }

    let ui = MainWindow::new()?;
    let job_registry = std::sync::Arc::new(std::sync::Mutex::new(load_job_registry()));
    let persistent_transfer_model: std::rc::Rc<slint::VecModel<TransferQueueItem>> =
        std::rc::Rc::new(slint::VecModel::from(Vec::new()));
    let persistent_completed_model: std::rc::Rc<slint::VecModel<CompletedItem>> =
        std::rc::Rc::new(slint::VecModel::from(Vec::new()));
    ui.set_transfer_queue(persistent_transfer_model.into());
    ui.set_completed_queue(persistent_completed_model.into());
    let ui_global_weak = ui.as_weak();

    tokio::spawn(async move {
        let mut local_t_q: Vec<TransferQueueItem> = Vec::new();
        let mut overall_total_bytes: u64 = 0;
        let mut target_conn_name = String::new();
        let mut transfer_start_time = tokio::time::Instant::now();
        let mut current_completed_bytes: u64 = 0;
        let mut last_ui_update = std::time::Instant::now();
        let current_snapshot_type = String::from("Live");

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
                            sync_transfer_queue_in_place(&ui, &[]);
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

                        if let Some(existing_idx) = local_t_q
                            .iter()
                            .position(|r| r.name.as_str() == file.as_str())
                        {
                            local_t_q[existing_idx] = item;
                        } else if local_t_q.is_empty() {
                            local_t_q.push(item);
                        } else {
                            local_t_q[0] = item;
                        }

                        current_completed_bytes = progress_bytes;

                        let elapsed_secs = transfer_start_time.elapsed().as_secs_f64();
                        let clone_eta = if current_completed_bytes > 0
                            && overall_total_bytes > current_completed_bytes
                            && elapsed_secs > 0.0
                        {
                            let bytes_per_sec = current_completed_bytes as f64 / elapsed_secs;
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

                        let clone_mu = if elapsed_secs > 0.0 {
                            format!(
                                "{:.2} MB/s",
                                (current_completed_bytes as f64 / 1_048_576.0) / elapsed_secs
                            )
                        } else {
                            "0.0 MB/s".to_string()
                        };

                        let local_t_q_clone = local_t_q.clone();
                        let ui_clone = ui_global_weak.clone();

                        if last_ui_update.elapsed() >= std::time::Duration::from_millis(85) {
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui) = ui_clone.upgrade() {
                                    sync_transfer_queue_in_place(&ui, &local_t_q_clone);
                                    ui.set_metric_download(clone_mu.into());
                                    ui.set_metric_eta(clone_eta.into());
                                }
                            });
                            last_ui_update = std::time::Instant::now();
                        }
                    } else if let simply_transfer_core::engine::FileTransferStatus::Completed =
                        status
                    {
                        local_t_q.retain(|r| r.name.as_str() != file.as_str());
                        let local_t_q_clone = local_t_q.clone();
                        let ui_clone = ui_global_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_clone.upgrade() {
                                sync_transfer_queue_in_place(&ui, &local_t_q_clone);
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
    if let Some(srv) = P2P_SERVER.get() {
        for c in &loaded {
            let parts: Vec<&str> = c.token.split(';').collect();
            if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(parts[0]) {
                let srv_clone = srv.clone();
                let pk = parsed.pub_key.trim().to_string();
                tokio::spawn(async move {
                    let _ = srv_clone.add_authorized_key_from_str(&pk).await;
                });
            }
            if parts.len() > 1 {
                let srv_clone = srv.clone();
                let peer_pk = parts[1].trim().to_string();
                tokio::spawn(async move {
                    let _ = srv_clone.add_authorized_key_from_str(&peer_pk).await;
                });
            }
        }
    }
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

    if is_os_permissions_configured() {
        ui.set_os_permissions_status("Configured (Saved)".into());
    }

    {
        let initial_conn_id = active_connection_uuid(&ui);
        reset_draft_job_for_connection(&ui, initial_conn_id);
        if let Ok(reg) = job_registry.lock() {
            sync_connection_jobs_ui(&ui, &reg, initial_conn_id);
            let jobs: Vec<TransferJobUi> = ui.get_connection_jobs().iter().collect();
            if let Some(first_job) = jobs.first() {
                ui.set_selected_job_idx(0);
                load_job_into_draft_ui(&ui, &reg, first_job);
            }
        }
    }

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
                    let mut target_ip = String::new();
                    let token_parts: Vec<&str> = token.split(';').collect();
                    let actual_token = token_parts[0];
                    let my_pub_key = if token_parts.len() > 1 {
                        Some(token_parts[1])
                    } else {
                        None
                    };

                    if host.contains('@') {
                        target_ip = host.split('@').nth(1).unwrap_or("").trim().to_string();
                    } else if host == "Remote"
                        || (host.trim().is_empty() && transfer_type == "Remote Transfer")
                    {
                        if let Ok(parsed) =
                            simply_transfer_crypto::token::ConnectionToken::parse(actual_token)
                        {
                            target_ip = parsed.ip;
                        }
                    } else if !host.trim().is_empty() {
                        target_ip = host.trim().to_string();
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
                            if let Some(priv_pem) =
                                resolve_private_key_pem(&mgr, key_to_use, &parsed.pub_key)
                            {
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
                            let model = ui.get_connections();
                            if let Some(vec_model) = model
                                .as_any()
                                .downcast_ref::<slint::VecModel<ConnectionItem>>()
                            {
                                for (i, new_state, is_connected) in updates {
                                    if let Some(mut item) = vec_model.row_data(i) {
                                        item.state = new_state.into();
                                        item.state_color = if is_connected {
                                            slint::Color::from_rgb_u8(50, 200, 50)
                                        } else {
                                            slint::Color::from_rgb_u8(200, 50, 50)
                                        };
                                        vec_model.set_row_data(i, item);
                                    }
                                }
                            }
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

    ui.on_open_url(|url| {
        open_url_in_browser(url.as_str());
    });

    ui.on_copy_text(|text| {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(text.to_string());
        }
    });

    let ui_weak_refresh = ui.as_weak();
    ui.on_refresh_browsers(move || {
        if let Some(ui) = ui_weak_refresh.upgrade() {
            populate_inline_browsers(&ui);
        }
    });

    let ui_weak_perms = ui.as_weak();
    ui.on_request_os_permissions(move || {
        if is_os_permissions_configured() {
            if let Some(ui) = ui_weak_perms.upgrade() {
                ui.set_os_permissions_status("All Required Permissions Already Configured".into());
            }
            return;
        }
        if let Some(ui) = ui_weak_perms.upgrade() {
            ui.set_os_permissions_status("Configuring one-time OS permissions...".into());
        }
        let ui_weak_done = ui_weak_perms.clone();
        tokio::task::spawn_blocking(move || {
            let home_ssh = get_user_home_dir().join(".ssh");
            let _ = std::fs::create_dir_all(&home_ssh);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&home_ssh, std::fs::Permissions::from_mode(0o700));
            }
            save_os_permissions_configured();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak_done.upgrade() {
                    ui.set_os_permissions_status(
                        "Configured (Saved — One-Time Setup Complete)".into(),
                    );
                }
            });
        });
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

            // Clear previous transfer state in-place
            sync_transfer_queue_in_place(&ui, &[]);
            sync_completed_queue_in_place(&ui, &[]);
            ui.set_phase_text("Live Transfer Queue".into());
            ui.set_overall_status("Preparing...".into());
            ui.set_current_phase("Idle".into());
            ui.set_is_transfer_paused(false);

            ui.set_remote_verified_host(format!("{}@{}", dest_user, dest_ip).into());

            let mappings: Vec<_> = ui.get_current_mappings().iter().collect();
            if mappings.is_empty() { return; }
            let t_type = ui.get_draft_job().job_type.to_string();
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
                    if let Some(pem) = resolve_private_key_pem(&mgr, key_to_use, &parsed.pub_key) {
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
                let mut interval = tokio::time::interval(std::time::Duration::from_millis(85));
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
                                                let item = TransferQueueItem {
                                                    name: file.clone().into(),
                                                    size: format!("{:.2} MB", total_bytes as f64 / 1_048_576.0).into(),
                                                    progress,
                                                    snapshot_type: current_snapshot_type.clone().into(),
                                                };
                                                if let Some(pos) = local_t_q.iter().position(|r| r.name.as_str() == file.as_str()) {
                                                    local_t_q[pos] = item;
                                                } else if local_t_q.is_empty() {
                                                    local_t_q.push(item);
                                                } else {
                                                    local_t_q[0] = item;
                                                }

                                                if progress_bytes > last_progress_bytes {
                                                    bytes_transferred_in_interval += progress_bytes - last_progress_bytes;
                                                } else if progress_bytes < last_progress_bytes {
                                                    bytes_transferred_in_interval += progress_bytes;
                                                }
                                                last_progress_bytes = progress_bytes;
                                            }
                                            FileTransferStatus::Completed => {
                                                local_t_q.retain(|r| r.name.as_str() != file.as_str());
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
                            if elapsed >= 0.085 {
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
                            let clone_eta = if current_completed_bytes > 0 && overall_total_bytes > current_completed_bytes && elapsed_secs > 0 {
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
                                        sync_transfer_queue_in_place(&ui, &clone_t_q);
                                        sync_completed_queue_in_place(&ui, &clone_c_q);

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
                    "connection_name": conn_name.clone(),
                    "sleep_duration": if transfer_type == "Continuous Sync" { 60 } else { 3600 }
                });

                let mut schedules = Vec::new();
                if let Ok(contents) = std::fs::read_to_string("schedules.json")
                    && let Ok(existing_schedules) = serde_json::from_str::<Vec<serde_json::Value>>(&contents) {
                        schedules = existing_schedules;
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
    let reg_for_commit_conn = job_registry.clone();
    ui.on_commit_connection(move |new_conn| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
            let mut conn_to_save = new_conn.clone();

            if conn_to_save.transfer_type != "Remote Transfer" {
                let mut pub_key = conn_to_save.token.trim().to_string();
                if pub_key.starts_with("st:")
                    && let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&pub_key)
                {
                    pub_key = parsed.pub_key;
                }
                if pub_key.is_empty() {
                    let key_mgr = simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
                    if let Ok(gen_key) = key_mgr.generate_and_store() {
                        pub_key = gen_key;
                    }
                }

                if !pub_key.is_empty()
                    && let Some(srv) = P2P_SERVER.get()
                {
                    let srv_clone = srv.clone();
                    let pk_to_auth = pub_key.clone();
                    tokio::spawn(async move {
                        let _ = srv_clone.add_authorized_key_from_str(&pk_to_auth).await;
                    });
                }

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
                        let ui_handle = ui_handle.clone();
                        let pub_key_expected = pub_key_clone.clone();
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
                                        if let Some(srv) = P2P_SERVER.get() {
                                            if srv.add_authorized_key_from_str(dest_pub.trim()).await {
                                                tracing::info!("Authorized key added to global P2P Daemon!");
                                            }
                                        }

                                        let my_user = std::env::var("USER")
                                            .or_else(|_| std::env::var("USERNAME"))
                                            .unwrap_or_else(|_| "simply-transfer".to_string());
                                        let res = format!("{{\"status\":\"ok\",\"user\":\"{}\"}}\n", my_user);
                                        let _ = writer.write_all(res.as_bytes()).await;

                                        let new_host = format!("{}@{}", dest_user, dest_ip);
                                        tracing::info!("Attempting to map TCP handshake for remote host: {}", new_host);
                                        let dest_pub_clean = dest_pub.split(';').next().unwrap_or(&dest_pub).trim().to_string();
                                        let _ = slint::invoke_from_event_loop(move || {
                                            if let Some(ui) = ui_handle.upgrade() {
                                                let mut conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
                                                for conn in &mut conns {
                                                    let mut conn_pub = String::new();
                                                    let clean_token = conn.token.to_string();
                                                    let clean_token = clean_token.split(';').next().unwrap_or(&clean_token).trim().to_string();
                                                    if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&clean_token) {
                                                        conn_pub = parsed.pub_key;
                                                    }

                                                    if (conn_pub == pub_key_expected || conn.state == "Waiting for peer...") && !conn_pub.is_empty() {
                                                        tracing::info!("Found connection waiting for peer. Updating state to Connected.");
                                                        conn.host = new_host.clone().into();
                                                        conn.token = format!("{};{}", clean_token, dest_pub_clean).into();
                                                        conn.state = "Connected".into();
                                                        conn.state_color = slint::Color::from_rgb_u8(50, 200, 50);
                                                    }
                                                }
                                                ui.set_connections(std::rc::Rc::new(slint::VecModel::from(conns.clone())).into());
                                                ui.set_overall_status("Connection Confirmed".into());
                                                ui.set_remote_verification_status("Connected".into());

                                                save_connections(&conns);
                                                populate_inline_browsers(&ui);
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
                        if let Some(u) = &parsed.user {
                            conn_to_save.host = format!("{}@{}", u, parsed.ip).into();
                        } else {
                            conn_to_save.host = parsed.ip.clone().into();
                        }
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
            let new_idx = (conns.len() - 1) as i32;
            save_connections(&conns);
            let model = std::rc::Rc::new(slint::VecModel::from(conns));
            ui.set_connections(model.into());
            ui.set_selected_connection_idx(new_idx);

            let conn_id = active_connection_uuid(&ui);
            reset_draft_job_for_connection(&ui, conn_id);
            if let Ok(reg) = reg_for_commit_conn.lock() {
                sync_connection_jobs_ui(&ui, &reg, conn_id);
            }
            populate_inline_browsers(&ui);
        }
    });

    ui.on_browse_source(move |_| {}); // No-op now

    let ui_weak = ui.as_weak();
    ui.on_browse_destination(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_browser_target_idx(idx);
            populate_inline_browsers(&ui);
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_fetch_directory(move |path, is_remote, is_dest| {
        let ui_weak = ui_weak.clone();
        let path_str = normalize_browse_path(path.as_str());

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
                        dest_user = parts[0].trim().to_string();
                        dest_ip = parts[1].trim().to_string();
                    } else if host == "Remote" || (host.trim().is_empty() && t_type == "Remote Transfer") {
                        if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&token) {
                            dest_ip = parsed.ip;
                            dest_user = parsed.user.unwrap_or_else(|| "simply-transfer".to_string());
                        }
                    } else if !host.trim().is_empty() {
                        dest_ip = host.trim().to_string();
                        if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&token) {
                            dest_user = parsed.user.unwrap_or_else(|| "simply-transfer".to_string());
                        } else {
                            dest_user = "simply-transfer".to_string();
                        }
                    }
                }
            }

        tokio::spawn(async move {
            let mut nodes = Vec::new();
            let mut discovered_remote_volumes: Option<Vec<VolumeUi>> = None;
            let mut success = false;
            let mut final_path = path_str.clone();

            if is_remote {
                if !dest_ip.is_empty() && !dest_user.is_empty() {
                    use simply_transfer_core::ssh::SshClient;
                    use simply_transfer_core::russh_client::RusshClient;

                    let mut ssh_client = RusshClient::new();
                    let mut priv_pem_str = String::new();

                    if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(token_str.split(';').next().unwrap_or(&token_str)) {
                        let mgr = simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
                        let private_key_id = token_str
                            .split(';')
                            .nth(1)
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| parsed.pub_key.clone());

                        if let Some(pem) = resolve_private_key_pem(&mgr, &private_key_id, &parsed.pub_key) {
                            priv_pem_str = pem;
                        } else {
                            tracing::error!(
                                "on_fetch_directory: Failed to retrieve private key from keyring/vault for pub_key {}",
                                private_key_id
                            );
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
                        if let Ok(rvols) = ssh_client.list_remote_volumes().await {
                            discovered_remote_volumes = Some(
                                rvols
                                    .into_iter()
                                    .map(|v| {
                                        let free_gb = v.available_bytes as f64 / 1_073_741_824.0;
                                        VolumeUi {
                                            mount_point: v.mount_point.to_string_lossy().to_string().into(),
                                            label: v.label.into(),
                                            fs_type: v.fs_type.into(),
                                            free_space_label: format!("{:.1} GB free", free_gb).into(),
                                        }
                                    })
                                    .collect(),
                            );
                        }

                        let mut resolved_path = if path_str.trim().is_empty() {
                            "/".to_string()
                        } else {
                            path_str.clone()
                        };
                        final_path = resolved_path.clone();

                        // 1. Primary path: Native built-in Rust P2P directory listing (no shell / cmd.exe required)
                        let native_cmd = format!("simply-transfer-list|{}", resolved_path);
                        if let Ok(output) = ssh_client.execute_command(&native_cmd).await {
                            let trimmed_out = output.trim();
                            let is_native_format = !trimmed_out.starts_with("E|")
                                && trimmed_out
                                    .lines()
                                    .all(|l| {
                                        let t = l.trim();
                                        t.is_empty() || t.starts_with("D|") || t.starts_with("F|")
                                    });
                            if is_native_format {
                                success = true;
                                for line in trimmed_out.lines() {
                                    let line = line.trim();
                                    if line.is_empty() {
                                        continue;
                                    }
                                    let parts: Vec<&str> = line.splitn(3, '|').collect();
                                    if parts.len() == 3 {
                                        let is_dir = parts[0] == "D";
                                        let name = parts[1];
                                        let full_path = parts[2];
                                        nodes.push(FileNode {
                                            name: name.into(),
                                            is_dir,
                                            path: full_path.into(),
                                            is_selected: false,
                                            depth: 0,
                                            is_expanded: false,
                                            snapshot_type: get_file_snapshot_type(name).into(),
                                        });
                                    }
                                }
                            }
                        }

                        // 2. Fallback path for legacy peers
                        if !success {
                            let is_windows = ssh_client
                                .execute_command("cmd.exe /c echo Windows")
                                .await
                                .map(|out| out.trim() == "Windows")
                                .unwrap_or(false);

                            if is_windows && (resolved_path == "/" || resolved_path.is_empty()) {
                                resolved_path = "C:\\".to_string();
                            } else if !is_windows
                                && (resolved_path.is_empty() || resolved_path == "\\")
                            {
                                resolved_path = "/".to_string();
                            }
                            final_path = resolved_path.clone();

                            let cmd = if is_windows {
                                format!(
                                    "powershell -NoProfile -NonInteractive -Command \"Get-ChildItem -Path '{}' -Force | ForEach-Object {{ if ($_.PSIsContainer) {{ $_.Name + '/' }} else {{ $_.Name }} }}\"",
                                    resolved_path
                                )
                            } else {
                                format!("ls -1p \"{}\"", resolved_path)
                            };

                            if let Ok(output) = ssh_client.execute_command(&cmd).await {
                                success = true;
                                for line in output.lines() {
                                    let line = line.trim();
                                    if line.is_empty() {
                                        continue;
                                    }
                                    let is_dir = line.ends_with('/');
                                    let name = if is_dir {
                                        &line[..line.len() - 1]
                                    } else {
                                        line
                                    };
                                    let full_path = if resolved_path.ends_with('/')
                                        || resolved_path.ends_with('\\')
                                    {
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
                                        depth: 0,
                                        is_expanded: false,
                                        snapshot_type: get_file_snapshot_type(name).into(),
                                    });
                                }
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
            } else {
                let effective_path = if cfg!(windows) && (path_str == "/" || path_str.is_empty() || path_str == "\\") {
                    "C:\\".to_string()
                } else {
                    path_str.clone()
                };
                final_path = effective_path.clone();

                if let Ok(entries) = std::fs::read_dir(&effective_path) {
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
            }

            nodes.sort_by(|a, b| {
                if a.is_dir && !b.is_dir { std::cmp::Ordering::Less }
                else if !a.is_dir && b.is_dir { std::cmp::Ordering::Greater }
                else { a.name.cmp(&b.name) }
            });

            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    if let Some(rvols) = discovered_remote_volumes {
                        let mut rlabels: Vec<slint::SharedString> = rvols
                            .iter()
                            .map(|v| v.mount_point.clone())
                            .collect();
                        if rlabels.is_empty() {
                            rlabels.push("/".into());
                        }
                        if let Some(first_r) = rlabels.first() {
                            ui.set_current_remote_volume(first_r.clone());
                        }
                        ui.set_remote_volume_labels(
                            std::rc::Rc::new(slint::VecModel::from(rlabels)).into(),
                        );
                        ui.set_remote_volumes(std::rc::Rc::new(slint::VecModel::from(rvols)).into());
                    }
                    let model = std::rc::Rc::new(slint::VecModel::from(nodes));
                    if is_dest {
                        ui.set_dest_browser_nodes(model.into());
                        ui.set_dest_browser_current_path(final_path.into());
                    } else {
                        ui.set_browser_nodes(model.into());
                        ui.set_browser_current_path(final_path.into());
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
                let t_type = conns[idx].transfer_type.to_string();
                token_str = token.clone();
                if host.contains('@') {
                    let parts: Vec<&str> = host.split('@').collect();
                    dest_user = parts[0].trim().to_string();
                    dest_ip = parts[1].trim().to_string();
                } else if host == "Remote" || (host.trim().is_empty() && t_type == "Remote Transfer") {
                    if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&token) {
                        dest_ip = parsed.ip;
                        dest_user = parsed.user.unwrap_or_else(|| "simply-transfer".to_string());
                    }
                } else if !host.trim().is_empty() {
                    dest_ip = host.trim().to_string();
                    if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(&token) {
                        dest_user = parsed.user.unwrap_or_else(|| "simply-transfer".to_string());
                    } else {
                        dest_user = "simply-transfer".to_string();
                    }
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
                    let mut priv_pem_str = String::new();

                    if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(token_str.split(';').next().unwrap_or(&token_str)) {
                        let mgr = simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
                        let private_key_id = token_str
                            .split(';')
                            .nth(1)
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| parsed.pub_key.clone());

                        if let Some(pem) = resolve_private_key_pem(&mgr, &private_key_id, &parsed.pub_key) {
                            priv_pem_str = pem;
                        }
                    }

                    if priv_pem_str.is_empty() {
                        tracing::error!("on_expand_node: Private key not available for authentication.");
                    } else if let Err(e) = ssh_client.connect(&dest_ip, 2222).await {
                        tracing::error!("on_expand_node: SSH connect to {} failed: {}", dest_ip, e);
                    } else if let Err(e) = ssh_client.authenticate_publickey(&dest_user, &priv_pem_str, None).await {
                        tracing::error!("on_expand_node: SSH auth for {}@{} failed: {}", dest_user, dest_ip, e);
                    } else {
                        let mut resolved_path = if node_path.trim().is_empty() {
                            "/".to_string()
                        } else {
                            node_path.clone()
                        };
                        let mut expanded_via_native = false;

                        let native_cmd = format!("simply-transfer-list|{}", resolved_path);
                        if let Ok(output) = ssh_client.execute_command(&native_cmd).await {
                            let trimmed_out = output.trim();
                            let is_native_format = !trimmed_out.starts_with("E|")
                                && trimmed_out
                                    .lines()
                                    .all(|l| {
                                        let t = l.trim();
                                        t.is_empty() || t.starts_with("D|") || t.starts_with("F|")
                                    });
                            if is_native_format {
                                expanded_via_native = true;
                                for line in trimmed_out.lines() {
                                    let line = line.trim();
                                    if line.is_empty() {
                                        continue;
                                    }
                                    let parts: Vec<&str> = line.splitn(3, '|').collect();
                                    if parts.len() == 3 {
                                        let is_dir = parts[0] == "D";
                                        let name = parts[1];
                                        let full_path = parts[2];
                                        nodes.push(FileNode {
                                            name: name.into(),
                                            is_dir,
                                            path: full_path.into(),
                                            is_selected: false,
                                            depth,
                                            is_expanded: false,
                                            snapshot_type: get_file_snapshot_type(name).into(),
                                        });
                                    }
                                }
                            }
                        }

                        if !expanded_via_native {
                            let is_windows = ssh_client
                                .execute_command("cmd.exe /c echo Windows")
                                .await
                                .map(|out| out.trim() == "Windows")
                                .unwrap_or(false);

                            let cmd = if is_windows {
                                if resolved_path.is_empty() {
                                    resolved_path = "C:\\".to_string();
                                }
                                format!(
                                    "powershell -NoProfile -NonInteractive -Command \"Get-ChildItem -Path '{}' | ForEach-Object {{ if ($_.PSIsContainer) {{ $_.Name + '/' }} else {{ $_.Name }} }}\"",
                                    resolved_path.replace("'", "''")
                                )
                            } else {
                                if resolved_path.is_empty() {
                                    resolved_path = "/".to_string();
                                }
                                format!("ls -1p '{}'", resolved_path.replace("'", "'\\''"))
                            };

                            if let Ok(output) = ssh_client.execute_command(&cmd).await {
                                for line in output.lines() {
                                    let line = line.trim();
                                    if line.is_empty() {
                                        continue;
                                    }
                                    let is_dir = line.ends_with('/');
                                    let name = if is_dir {
                                        &line[..line.len() - 1]
                                    } else {
                                        line
                                    };
                                    let full_path = if resolved_path.ends_with('/')
                                        || resolved_path.ends_with('\\')
                                    {
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
                                        snapshot_type: get_file_snapshot_type(name).into(),
                                    });
                                }
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
            let mut mappings: Vec<DirectoryMapping> = ui
                .get_current_mappings()
                .iter()
                .filter(|m| !m.source.trim().is_empty() || !m.destination.trim().is_empty())
                .collect();

            let dest = if dest_paths.row_count() > 0 {
                dest_paths.row_data(0).unwrap().to_string()
            } else {
                ui.get_dest_browser_current_path().to_string()
            };

            if source_paths.row_count() > 0 {
                for i in 0..source_paths.row_count() {
                    if let Some(src) = source_paths.row_data(i) {
                        mappings.push(DirectoryMapping {
                            source: src,
                            destination: dest.clone().into(),
                        });
                    }
                }
            } else {
                let current_src = ui.get_browser_current_path();
                if !current_src.trim().is_empty() {
                    mappings.push(DirectoryMapping {
                        source: current_src,
                        destination: dest.into(),
                    });
                }
            }

            if let Some(first) = mappings.first() {
                let mut draft = ui.get_draft_job();
                draft.source = first.source.clone();
                draft.destination = first.destination.clone();
                ui.set_draft_job(draft);
            }

            ui.set_current_mappings(std::rc::Rc::new(slint::VecModel::from(mappings)).into());

            // Clear active tree selection in both panes
            let cleared_src_nodes: Vec<FileNode> = ui
                .get_browser_nodes()
                .iter()
                .map(|mut n| {
                    n.is_selected = false;
                    n
                })
                .collect();
            let cleared_dest_nodes: Vec<FileNode> = ui
                .get_dest_browser_nodes()
                .iter()
                .map(|mut n| {
                    n.is_selected = false;
                    n
                })
                .collect();
            ui.set_browser_nodes(std::rc::Rc::new(slint::VecModel::from(cleared_src_nodes)).into());
            ui.set_dest_browser_nodes(
                std::rc::Rc::new(slint::VecModel::from(cleared_dest_nodes)).into(),
            );
            ui.set_browser_selection(
                std::rc::Rc::new(slint::VecModel::from(Vec::<slint::SharedString>::new())).into(),
            );
            ui.set_dest_browser_selection(
                std::rc::Rc::new(slint::VecModel::from(Vec::<slint::SharedString>::new())).into(),
            );
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
    let reg_for_conn_select = job_registry.clone();
    ui.on_connection_selected(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
            let idx = idx as usize;
            let conn_id = if idx < conns.len() {
                connection_uuid_for_item(&conns[idx])
            } else {
                uuid::Uuid::nil()
            };
            // Explicitly reset draft_job to a clean default bound to the active connection_id
            // so previous connection transfer settings never bleed into new connections or tasks.
            reset_draft_job_for_connection(&ui, conn_id);
            if let Ok(reg) = reg_for_conn_select.lock() {
                sync_connection_jobs_ui(&ui, &reg, conn_id);
                let jobs: Vec<TransferJobUi> = ui.get_connection_jobs().iter().collect();
                if let Some(first_job) = jobs.first() {
                    ui.set_selected_job_idx(0);
                    load_job_into_draft_ui(&ui, &reg, first_job);
                }
                if let Some(vpn_cfg) = reg
                    .get_connection(conn_id)
                    .and_then(|c| c.vpn_config.as_ref())
                {
                    ui.set_vpn_enabled(true);
                    ui.set_vpn_endpoint(vpn_cfg.endpoint.clone().into());
                    ui.set_vpn_interface_ip(vpn_cfg.address.clone().into());
                    ui.set_vpn_masked_pubkey(vpn_cfg.masked_public_key().into());
                    ui.set_vpn_tunnel_status("Configured (Ready)".into());
                } else {
                    ui.set_vpn_enabled(false);
                    ui.set_vpn_conf_path("".into());
                    ui.set_vpn_endpoint("".into());
                    ui.set_vpn_interface_ip("".into());
                    ui.set_vpn_masked_pubkey("".into());
                    ui.set_vpn_tunnel_status("Disabled".into());
                }
            }
            populate_inline_browsers(&ui);
        }
    });

    let ui_weak = ui.as_weak();
    let reg_for_vpn_import = job_registry.clone();
    ui.on_import_wireguard_conf(move |path_arg| {
        if let Some(ui) = ui_weak.upgrade() {
            let chosen_path = if !path_arg.trim().is_empty() {
                Some(std::path::PathBuf::from(path_arg.trim()))
            } else {
                rfd::FileDialog::new()
                    .add_filter("WireGuard Config", &["conf"])
                    .pick_file()
            };

            let Some(conf_path) = chosen_path else {
                return;
            };

            match std::fs::read_to_string(&conf_path) {
                Ok(raw_conf) => match simply_transfer_crypto::WireGuardConf::parse(&raw_conf) {
                    Ok(parsed) => {
                        let storage =
                            simply_transfer_crypto::SecureStorage::new("com.simplytransfer.app");
                        match simply_transfer_crypto::VpnConfig::from_wireguard_conf(
                            &parsed, &storage,
                        ) {
                            Ok(vpn_cfg) => {
                                ui.set_vpn_enabled(true);
                                ui.set_vpn_conf_path(
                                    conf_path.to_string_lossy().to_string().into(),
                                );
                                ui.set_vpn_endpoint(vpn_cfg.endpoint.clone().into());
                                ui.set_vpn_interface_ip(vpn_cfg.address.clone().into());
                                ui.set_vpn_masked_pubkey(vpn_cfg.masked_public_key().into());
                                ui.set_vpn_tunnel_status("Configured (Ready)".into());

                                let conns: Vec<ConnectionItem> =
                                    ui.get_connections().iter().collect();
                                let idx = ui.get_selected_connection_idx() as usize;
                                let conn_id = active_connection_uuid(&ui);
                                let (name, host) = if idx < conns.len() {
                                    (conns[idx].name.to_string(), conns[idx].host.to_string())
                                } else {
                                    ("Default Connection".to_string(), vpn_cfg.endpoint.clone())
                                };

                                if let Ok(mut reg) = reg_for_vpn_import.lock() {
                                    let mut profile = reg
                                        .get_connection(conn_id)
                                        .cloned()
                                        .unwrap_or(simply_transfer_core::ConnectionProfile {
                                            id: conn_id,
                                            name,
                                            host,
                                            port: 2222,
                                            username: "simply-transfer".to_string(),
                                            auth_config: simply_transfer_core::AuthConfig::default(
                                            ),
                                            vpn_config: None,
                                        });
                                    profile.vpn_config = Some(vpn_cfg);
                                    reg.upsert_connection(profile);
                                    save_job_registry(&reg);
                                }
                            }
                            Err(e) => {
                                ui.set_vpn_tunnel_status(format!("Vault Error: {}", e).into());
                            }
                        }
                    }
                    Err(e) => {
                        ui.set_vpn_tunnel_status(format!("Parse Error: {}", e).into());
                    }
                },
                Err(e) => {
                    ui.set_vpn_tunnel_status(format!("File Error: {}", e).into());
                }
            }
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_new_transfer_clicked(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let conn_id = active_connection_uuid(&ui);
            reset_draft_job_for_connection(&ui, conn_id);
            ui.set_is_naming_new_job(true);
            ui.set_new_job_name_draft("".into());
            ui.set_is_job_setup_unlocked(false);
        }
    });

    let ui_weak = ui.as_weak();
    let reg_for_create_named_job = job_registry.clone();
    ui.on_create_named_transfer_job(move |raw_name| {
        if let Some(ui) = ui_weak.upgrade() {
            let conn_id = active_connection_uuid(&ui);
            let existing_jobs: Vec<TransferJobUi> = ui.get_connection_jobs().iter().collect();
            let trimmed = raw_name.trim().to_string();
            let final_name = if trimmed.is_empty() {
                if existing_jobs.is_empty() {
                    "Default Transfer Task".to_string()
                } else {
                    format!("Transfer Task {}", existing_jobs.len() + 1)
                }
            } else {
                trimmed
            };

            reset_draft_job_for_connection(&ui, conn_id);
            let job_id = uuid::Uuid::new_v4();
            let mut draft = ui.get_draft_job();
            draft.job_id = job_id.to_string().into();
            draft.connection_id = conn_id.to_string().into();
            draft.name = final_name.clone().into();
            ui.set_draft_job(draft.clone());

            let new_job = simply_transfer_core::TransferJob {
                job_id,
                connection_id: conn_id,
                name: final_name,
                job_type: simply_transfer_core::TransferJobType::Backup,
                source_paths: Vec::new(),
                destination_path: PathBuf::new(),
                mappings: Vec::new(),
                schedule: Some(simply_transfer_core::ScheduleConfig {
                    mode: draft.job_type.to_string(),
                    interval_secs: None,
                    window_start: Some(draft.start_permitted.to_string()),
                    window_end: Some(draft.finish_permitted.to_string()),
                    sync_frequency: Some(draft.sync_frequency.to_string()),
                    active_days: Some(draft.active_days.to_string()),
                }),
                use_snapshots: draft.use_snapshots,
            };

            if let Ok(mut reg) = reg_for_create_named_job.lock() {
                reg.upsert_job(new_job);
                save_job_registry(&reg);
                sync_connection_jobs_ui(&ui, &reg, conn_id);
                let updated_jobs: Vec<TransferJobUi> = ui.get_connection_jobs().iter().collect();
                let job_id_str = job_id.to_string();
                if let Some(pos) = updated_jobs
                    .iter()
                    .position(|j| j.job_id.as_str() == job_id_str.as_str())
                {
                    ui.set_selected_job_idx(pos as i32);
                }
            }

            ui.set_is_naming_new_job(false);
            ui.set_new_job_name_draft("".into());
            ui.set_is_job_setup_unlocked(true);
            populate_inline_browsers(&ui);
        }
    });

    let ui_weak = ui.as_weak();
    let reg_for_select_job = job_registry.clone();
    ui.on_select_transfer_job(move |job_idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let jobs: Vec<TransferJobUi> = ui.get_connection_jobs().iter().collect();
            let idx = job_idx as usize;
            if idx < jobs.len() {
                let selected = jobs[idx].clone();
                if let Ok(reg) = reg_for_select_job.lock() {
                    load_job_into_draft_ui(&ui, &reg, &selected);
                }
                populate_inline_browsers(&ui);
            }
        }
    });

    let ui_weak = ui.as_weak();
    let reg_for_rename_job = job_registry.clone();
    ui.on_rename_transfer_job(move |job_idx, new_name| {
        if let Some(ui) = ui_weak.upgrade() {
            let trimmed = new_name.trim().to_string();
            if trimmed.is_empty() {
                return;
            }
            let conn_id = active_connection_uuid(&ui);
            let mut jobs: Vec<TransferJobUi> = ui.get_connection_jobs().iter().collect();
            let idx = job_idx as usize;
            if idx < jobs.len() {
                let target_id_str = jobs[idx].job_id.to_string();
                jobs[idx].name = trimmed.clone().into();
                ui.set_connection_jobs(
                    std::rc::Rc::new(slint::VecModel::from(jobs.clone())).into(),
                );

                if ui.get_selected_job_idx() == job_idx {
                    let mut draft = ui.get_draft_job();
                    draft.name = trimmed.clone().into();
                    ui.set_draft_job(draft);
                }

                if let Ok(job_uuid) = uuid::Uuid::parse_str(&target_id_str)
                    && let Ok(mut reg) = reg_for_rename_job.lock()
                    && let Some(job) = reg.jobs.get_mut(&job_uuid)
                {
                    job.name = trimmed;
                    save_job_registry(&reg);
                    sync_connection_jobs_ui(&ui, &reg, conn_id);
                    let updated_jobs: Vec<TransferJobUi> =
                        ui.get_connection_jobs().iter().collect();
                    if let Some(pos) = updated_jobs
                        .iter()
                        .position(|j| j.job_id.as_str() == target_id_str.as_str())
                    {
                        ui.set_selected_job_idx(pos as i32);
                    }
                }
            }
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_toggle_active_day(move |day_idx, checked| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut flags: Vec<bool> = ui.get_active_days_flags().iter().collect();
            let idx = day_idx as usize;
            if idx < flags.len() {
                flags[idx] = checked;
                let days_str = format_active_days_flags(&flags);
                ui.set_active_days_flags(std::rc::Rc::new(slint::VecModel::from(flags)).into());
                let mut draft = ui.get_draft_job();
                draft.active_days = days_str.into();
                ui.set_draft_job(draft);
            }
        }
    });

    let ui_weak = ui.as_weak();
    let reg_for_delete_job = job_registry.clone();
    ui.on_delete_transfer_job(move |job_idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let conn_id = active_connection_uuid(&ui);
            let jobs: Vec<TransferJobUi> = ui.get_connection_jobs().iter().collect();
            let idx = job_idx as usize;
            if idx < jobs.len() {
                let target_id_str = jobs[idx].job_id.to_string();
                if let Ok(job_uuid) = uuid::Uuid::parse_str(&target_id_str)
                    && let Ok(mut reg) = reg_for_delete_job.lock()
                {
                    reg.remove_job(job_uuid);
                    save_job_registry(&reg);
                    sync_connection_jobs_ui(&ui, &reg, conn_id);
                    let remaining: Vec<TransferJobUi> = ui.get_connection_jobs().iter().collect();
                    if let Some(first_job) = remaining.first() {
                        ui.set_selected_job_idx(0);
                        load_job_into_draft_ui(&ui, &reg, first_job);
                    } else {
                        reset_draft_job_for_connection(&ui, conn_id);
                    }
                    populate_inline_browsers(&ui);
                }
            }
        }
    });

    let ui_weak = ui.as_weak();
    let reg_for_save_job = job_registry.clone();
    ui.on_save_draft_job(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let conn_id = active_connection_uuid(&ui);
            let mut draft = ui.get_draft_job();
            let mappings: Vec<DirectoryMapping> = ui.get_current_mappings().iter().collect();
            let source_paths: Vec<PathBuf> = mappings
                .iter()
                .filter(|m| !m.source.trim().is_empty())
                .map(|m| PathBuf::from(m.source.as_str()))
                .collect();
            let dest_path = mappings
                .first()
                .map(|m| PathBuf::from(m.destination.as_str()))
                .unwrap_or_else(|| PathBuf::from(draft.destination.as_str()));
            let mappings_vec: Vec<(PathBuf, PathBuf)> = mappings
                .iter()
                .filter(|m| !m.source.trim().is_empty() || !m.destination.trim().is_empty())
                .map(|m| {
                    (
                        PathBuf::from(m.source.as_str()),
                        PathBuf::from(m.destination.as_str()),
                    )
                })
                .collect();

            let job_id = uuid::Uuid::parse_str(draft.job_id.as_str())
                .unwrap_or_else(|_| uuid::Uuid::new_v4());
            draft.job_id = job_id.to_string().into();
            draft.connection_id = conn_id.to_string().into();
            if let Some(first_map) = mappings.first() {
                draft.source = first_map.source.clone();
                draft.destination = first_map.destination.clone();
            }
            ui.set_draft_job(draft.clone());

            let job_type = if draft.job_type.as_str() == "Continuous Sync" {
                simply_transfer_core::TransferJobType::Sync
            } else {
                simply_transfer_core::TransferJobType::Backup
            };

            let job = simply_transfer_core::TransferJob {
                job_id,
                connection_id: conn_id,
                name: if draft.name.trim().is_empty() {
                    "Transfer Task".to_string()
                } else {
                    draft.name.to_string()
                },
                job_type,
                source_paths,
                destination_path: dest_path,
                mappings: mappings_vec,
                schedule: Some(simply_transfer_core::ScheduleConfig {
                    mode: draft.job_type.to_string(),
                    interval_secs: None,
                    window_start: Some(draft.start_permitted.to_string()),
                    window_end: Some(draft.finish_permitted.to_string()),
                    sync_frequency: Some(draft.sync_frequency.to_string()),
                    active_days: Some(draft.active_days.to_string()),
                }),
                use_snapshots: draft.use_snapshots,
            };

            if let Ok(mut reg) = reg_for_save_job.lock() {
                reg.upsert_job(job);
                save_job_registry(&reg);
                sync_connection_jobs_ui(&ui, &reg, conn_id);
                let updated_jobs: Vec<TransferJobUi> = ui.get_connection_jobs().iter().collect();
                let job_id_str = job_id.to_string();
                if let Some(pos) = updated_jobs
                    .iter()
                    .position(|j| j.job_id.as_str() == job_id_str.as_str())
                {
                    ui.set_selected_job_idx(pos as i32);
                }
            }
        }
    });

    let ui_weak = ui.as_weak();
    let reg_for_delete_conn = job_registry.clone();
    ui.on_delete_connection(move |idx| {
        if let Some(ui) = ui_weak.upgrade() {
            let mut conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
            let idx = idx as usize;
            if idx < conns.len() {
                let deleted_conn_id = connection_uuid_for_item(&conns[idx]);
                let deleted_conn_name = conns[idx].name.to_string(); // Extracted connection name
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

                if let Ok(mut reg) = reg_for_delete_conn.lock() {
                    reg.remove_connection(deleted_conn_id);
                    save_job_registry(&reg);
                    let new_conn_id = active_connection_uuid(&ui);
                    reset_draft_job_for_connection(&ui, new_conn_id);
                    sync_connection_jobs_ui(&ui, &reg, new_conn_id);
                    let jobs: Vec<TransferJobUi> = ui.get_connection_jobs().iter().collect();
                    if let Some(first_job) = jobs.first() {
                        ui.set_selected_job_idx(0);
                        load_job_into_draft_ui(&ui, &reg, first_job);
                    }
                    populate_inline_browsers(&ui);
                }

                // Load existing schedules and filter out those tied to this connection
                let mut updated_schedules = Vec::new();
                if let Ok(file) = std::fs::File::open("schedules.json")
                    && let Ok(loaded_schedules) =
                        serde_json::from_reader::<_, Vec<serde_json::Value>>(file)
                {
                    for schedule in loaded_schedules {
                        // Only keep schedules not associated with the deleted connection
                        if schedule
                            .get("connection_name")
                            .is_none_or(|v| v.as_str() != Some(&deleted_conn_name))
                        {
                            updated_schedules.push(schedule);
                        }
                    }
                }

                // Save filtered schedules back to disk
                if let Ok(mut file) = std::fs::File::create("schedules.json") {
                    if let Err(e) = serde_json::to_writer_pretty(&mut file, &updated_schedules) {
                        eprintln!("Failed to write schedules: {}", e);
                    }
                } else {
                    eprintln!("Failed to create schedules.json");
                }

                // Rebuild UI model with updated schedule data
                let mut slint_schedules = Vec::new();
                for (new_idx, schedule) in updated_schedules.iter().enumerate() {
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
                ui.set_schedules(std::rc::Rc::new(slint::VecModel::from(slint_schedules)).into());
            }
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_verify_remote_token(move |token| {
        let ui_weak = ui_weak.clone();
        let token = token.trim().to_string();
        tokio::spawn(async move {
            log_event("connection", &format!("Validating remote token: {}", token));

            let (verified, status) = match simply_transfer_crypto::token::ConnectionToken::parse(&token) {
                Ok(parsed) => {
                    // 1. Immediately ingest the peer's public key into the built-in P2P SSH server (port 2222)
                    // before any network handshake or external OS commands so Windows/Linux key ingestion is never delayed.
                    if let Some(srv) = P2P_SERVER.get() {
                        if srv.add_authorized_key_from_str(parsed.pub_key.trim()).await {
                            tracing::info!("Immediately ingested remote public key into P2P Daemon!");
                            log_event("connection", "Added public key to built-in P2P SSH server.");
                        }
                    }

                    // 2. Ingest the public key to ~/.ssh/authorized_keys
                    let pub_key_line = format!("ssh-ed25519 {} simply-transfer", parsed.pub_key.trim());

                    let home = get_user_home_dir();
                    let ssh_dir = home.join(".ssh");
                    let _ = std::fs::create_dir_all(&ssh_dir);
                    let auth_keys = ssh_dir.join("authorized_keys");

                    let mut key_exists = false;
                    if let Ok(content) = std::fs::read_to_string(&auth_keys)
                        && content.contains(parsed.pub_key.trim())
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
                            if content.contains(parsed.pub_key.trim()) {
                                admin_key_exists = true;
                            }
                        }
                        if !admin_key_exists {
                            use std::io::Write;
                            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(admin_keys) {
                                let _ = writeln!(file, "{}", pub_key_line);
                                log_event("connection", "Appended public key to administrators_authorized_keys.");
                            } else {
                                use std::os::windows::process::CommandExt;
                                let ps_inner = format!(
                                    "$dir = 'C:\\ProgramData\\ssh'; \
                                     if (!(Test-Path $dir)) {{ New-Item -ItemType Directory -Path $dir -Force | Out-Null }}; \
                                     $file = 'C:\\ProgramData\\ssh\\administrators_authorized_keys'; \
                                     $key = '{}'; \
                                     if (!(Test-Path $file) -or !((Get-Content $file -Raw -ErrorAction SilentlyContinue) -match [regex]::Escape($key))) {{ \
                                         Add-Content -Path $file -Value $key -Encoding UTF8 \
                                     }}; \
                                     icacls.exe $file /inheritance:r /grant 'Administrators:F' /grant 'SYSTEM:F' | Out-Null",
                                    pub_key_line.replace('\'', "''")
                                );
                                let encoded_inner = simply_transfer_crypto::token::encode_utf16le_base64(&ps_inner);
                                let outer_cmd = format!(
                                    "Start-Process powershell -Verb RunAs -WindowStyle Hidden -Wait -ArgumentList '-NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {}'",
                                    encoded_inner
                                );
                                match std::process::Command::new("powershell")
                                    .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &outer_cmd])
                                    .creation_flags(0x08000000)
                                    .status()
                                {
                                    Ok(st) if st.success() => {
                                        log_event("connection", "Appended public key to administrators_authorized_keys via elevated PowerShell.");
                                    }
                                    Ok(st) => {
                                        log_event("connection", &format!("Elevated PowerShell exited with status: {}", st));
                                    }
                                    Err(e) => {
                                        log_event("connection", &format!("Failed to launch elevated PowerShell: {}", e));
                                    }
                                }
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

                    // Generate our own key pair and securely store it in the system keyring & encrypted vault
                    let key_mgr = simply_transfer_crypto::keys::KeyPairManager::new("com.simplytransfer.app");
                    let mut my_pub_key_id = String::new();
                    match key_mgr.generate_and_store() {
                        Ok(pub_b) => {
                            my_pub_key_id = pub_b;
                        }
                        Err(e) => {
                            tracing::error!("on_verify_remote_token: Failed to generate and store key: {}", e);
                            log_event("connection", &format!("Failed to generate secure credentials: {}", e));
                        }
                    }

                    if my_pub_key_id.is_empty() {
                        log_event("connection", "Handshake aborted: missing local public key ID.");
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_is_remote_verified(false);
                                ui.set_remote_verification_status("Key Generation Failed".into());
                            }
                        });
                        return;
                    }

                    if let Some(srv) = P2P_SERVER.get() {
                        let _ = srv.add_authorized_key_from_str(my_pub_key_id.trim()).await;
                    }

                    let fallback_remote_user = parsed
                        .user
                        .clone()
                        .unwrap_or_else(|| "simply-transfer".to_string());
                    let fallback_host = format!("{}@{}", fallback_remote_user, parsed.ip);
                    let default_appended_token = format!("{};{}", token, my_pub_key_id);
                    {
                        let ui_init = ui_weak.clone();
                        let host_init = fallback_host.clone();
                        let tok_init = default_appended_token.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_init.upgrade() {
                                ui.set_remote_verified_host(host_init.into());
                                ui.set_remote_verified_full_token(tok_init.into());
                            }
                        });
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

                                    if let Ok(res) = serde_json::from_str::<serde_json::Value>(&line) {
                                        let remote_user = res["user"].as_str().unwrap_or(&fallback_remote_user).to_string();
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
                                                save_connections(&conns);
                                                ui.set_remote_verification_status("Verified".into());
                                                ui.set_remote_verified_full_token(appended_token.clone().into());
                                                ui.set_connections(std::rc::Rc::new(slint::VecModel::from(conns)).into());
                                                populate_inline_browsers(&ui);
                                            }
                                        });
                                    } else {
                                        log_event("connection", "Remote handshake failed or was rejected.");
                                    }
                                    (true, "Connection Verified Successfully".to_string())
                                } else {
                                    log_event("connection", "Handshake failed to return OK.");
                                    (false, "Handshake Failed".to_string())
                                }
                            } else {
                                log_event("connection", "Remote handshake read timed out.");
                                (true, "Key Ingested, but Handshake Read Timed Out".to_string())
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

    let ui_weak = ui.as_weak();
    let reg_for_purge = job_registry.clone();
    ui.on_purge_orphaned_keys(move || {
        let mut active_keys: Vec<String> = Vec::new();
        let mut active_pub_keys: Vec<String> = Vec::new();
        if let Some(ui) = ui_weak.upgrade() {
            let conns: Vec<ConnectionItem> = ui.get_connections().iter().collect();
            for c in conns {
                let tok = c.token.to_string();
                let parts: Vec<&str> = tok.split(';').collect();
                if let Ok(parsed) = simply_transfer_crypto::token::ConnectionToken::parse(parts[0])
                {
                    let clean_pk = clean_key_identifier(&parsed.pub_key);
                    if !clean_pk.is_empty() {
                        active_keys.push(clean_pk);
                        active_pub_keys.push(parsed.pub_key);
                    }
                }
                if parts.len() > 1 {
                    let peer_raw = parts[1].trim().to_string();
                    let clean_peer = clean_key_identifier(&peer_raw);
                    if !clean_peer.is_empty() {
                        active_keys.push(clean_peer);
                        active_pub_keys.push(peer_raw);
                    }
                }
            }
        }
        if let Ok(reg) = reg_for_purge.lock() {
            for conn in reg.connections.values() {
                if !conn.auth_config.key_id.is_empty() {
                    active_keys.push(conn.auth_config.key_id.clone());
                }
                if let Some(vpn) = &conn.vpn_config {
                    active_keys.push(format!("wg_priv_{}", vpn.peer_public_key));
                    active_keys.push(format!("wg_psk_{}", vpn.peer_public_key));
                }
            }
        }

        let storage = simply_transfer_crypto::SecureStorage::new("com.simplytransfer.app");
        let vault_purged = storage.purge_orphaned_keys(&active_keys).unwrap_or(0);

        let p2p_purged = if let Some(srv) = P2P_SERVER.get() {
            let srv_clone = srv.clone();
            let pks = active_pub_keys.clone();
            tokio::task::block_in_place(move || {
                tokio::runtime::Handle::current()
                    .block_on(async move { srv_clone.purge_unauthorized_keys(&pks).await })
            })
        } else {
            0
        };

        let total = vault_purged + p2p_purged;
        log_event(
            "connection",
            &format!(
                "Purged {} orphaned keys ({} vault, {} P2P authorized keys).",
                total, vault_purged, p2p_purged
            ),
        );
        total as i32
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
    if let Ok(contents) = std::fs::read_to_string("schedules.json")
        && let Ok(schedules) = serde_json::from_str::<Vec<serde_json::Value>>(&contents)
    {
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

    ui.on_delete_schedule(move |id| {
        // Parse schedule ID
        let idx = match id.as_str().parse::<usize>() {
            Ok(idx) => idx,
            Err(_) => return,
        };

        // Read and parse schedules
        let contents = match std::fs::read_to_string("schedules.json") {
            Ok(contents) => contents,
            Err(_) => return,
        };

        let schedules = match serde_json::from_str::<Vec<serde_json::Value>>(&contents) {
            Ok(schedules) => schedules,
            Err(_) => return,
        };

        // Check if index is valid
        if idx >= schedules.len() {
            return;
        }

        // Remove schedule at index
        let mut updated_schedules = schedules.clone();
        updated_schedules.remove(idx);

        // Write back to file
        match serde_json::to_string_pretty(&updated_schedules) {
            Ok(json_string) => {
                if std::fs::write("schedules.json", json_string).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }

        // Update UI model
        let mut slint_schedules = Vec::new();
        for (new_idx, schedule) in updated_schedules.iter().enumerate() {
            let transfer_type = schedule["transfer_type"].as_str().unwrap_or("").to_string();
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
            ui.set_schedules(std::rc::Rc::new(slint::VecModel::from(slint_schedules)).into());
        }
    });

    // Load and run persistent schedules
    {
        let global_tx_clone = global_tx.clone();
        tokio::spawn(async move {
            if let Ok(contents) = std::fs::read_to_string("schedules.json")
                && let Ok(schedules) = serde_json::from_str::<Vec<serde_json::Value>>(&contents)
            {
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
                    let log_type = if transfer_type == "Continuous Sync" {
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
                                    let mut total_bytes = 0u64;
                                    let mut transferred_files = Vec::new();
                                    while let Some(event) = rx.recv().await {
                                        match &event {
                                            TransferEvent::TransferStarted(_) => {
                                                log_event(log_type, "Connected successfully.");
                                            }
                                            TransferEvent::ManifestGenerated {
                                                total_bytes: bytes,
                                                ..
                                            } => {
                                                total_bytes = *bytes;
                                            }
                                            TransferEvent::FileStatusChanged(
                                                file,
                                                FileTransferStatus::Completed,
                                            ) => {
                                                transferred_files.push(file.clone());
                                            }
                                            TransferEvent::FileStatusChanged(_, _) => {}
                                            TransferEvent::TransferComplete {
                                                successful,
                                                failed,
                                            } => {
                                                log_event(
                                                    log_type,
                                                    &format!(
                                                        "Task complete: {} successful, {} failed",
                                                        successful, failed
                                                    ),
                                                );
                                                log_event(
                                                    log_type,
                                                    &format!("Total size: {} bytes", total_bytes),
                                                );
                                                log_event(
                                                    log_type,
                                                    &format!(
                                                        "Files transferred: {:?}",
                                                        transferred_files
                                                    ),
                                                );
                                            }
                                            TransferEvent::TransferFailed(err) => {
                                                log_event(
                                                    log_type,
                                                    &format!("Task failed: {}", err),
                                                );
                                            }
                                            _ => {}
                                        }
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
                                    Arc::new(simply_transfer_core::russh_client::RusshClient::new()),
                                    Arc::new(simply_transfer_core::russh_client::RusshClient::new()),
                                    snapshot_driver,
                                    None,
                                );
                                if let Err(e) = engine.execute().await {
                                    tracing::error!("Engine execution failed for {}: {:?}", src, e);
                                }
                            }
                            tracing::info!(
                                "Cycle complete. Sleeping for {} seconds...",
                                sleep_duration
                            );
                            tokio::time::sleep(tokio::time::Duration::from_secs(sleep_duration))
                                .await;
                        }
                    });
                }
            }
        });
    }

    // Auto-initialize inline dual-pane browser volumes and directory trees on startup
    populate_inline_browsers(&ui);

    ui.run()?;

    // Explicitly drop the context guard
    drop(_guard);

    // Bypass the Tokio Thread Local drop panics by issuing a hard OS-level process exit
    std::process::exit(0);
}
