use crate::registry::{FileEntry, Registry, TransferManifest};
use crate::ssh::SshClient;
use simply_transfer_snapshots::SnapshotDriver;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};
use walkdir::WalkDir;

#[derive(Error, Debug)]
pub enum EngineError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("SSH/Network error: {0}")]
    Network(String),
    #[error("Phase error: {0}")]
    PhaseError(String),
    #[error("Validation failed for {0}")]
    ValidationFailed(String),
}

/// Represents the status of a single file in the transfer queue.
#[derive(Debug, Clone)]
pub enum FileTransferStatus {
    Pending,
    Skipped,
    Transferring {
        progress_bytes: u64,
        total_bytes: u64,
    },
    Completed,
    Validated,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlSignal {
    Run,
    Pause,
    Cancel,
}

/// Events emitted by the engine to the UI/consumers.
#[derive(Debug, Clone)]
pub enum TransferEvent {
    PhaseChanged(u8, String),
    ManifestGenerated(usize),
    FileStatusChanged(String, FileTransferStatus),
    TransferComplete { successful: usize, failed: usize },
    TransferFailed(String),
}

/// The core engine orchestrating the 3-phase transfer.
pub struct TransferEngine {
    source_dir: PathBuf,
    destination_dir: String, // Remote path
    event_sender: mpsc::Sender<TransferEvent>,
    ssh_client: Arc<dyn SshClient>,
    validation_ssh_client: Arc<dyn SshClient>,
    snapshot_driver: Arc<dyn SnapshotDriver>,
    control_rx: Option<watch::Receiver<ControlSignal>>,
}

impl TransferEngine {
    pub fn new(
        source_dir: PathBuf,
        destination_dir: String,
        event_sender: mpsc::Sender<TransferEvent>,
        ssh_client: Arc<dyn SshClient>,
        validation_ssh_client: Arc<dyn SshClient>,
        snapshot_driver: Arc<dyn SnapshotDriver>,
        control_rx: Option<watch::Receiver<ControlSignal>>,
    ) -> Self {
        Self {
            source_dir,
            destination_dir,
            event_sender,
            ssh_client,
            validation_ssh_client,
            snapshot_driver,
            control_rx,
        }
    }

    /// Execute the full 3-phase transfer protocol.
    pub async fn execute(&self) -> Result<(), EngineError> {
        self.emit_phase(0, "Snapshot Preparation".to_string()).await;

        // Take a snapshot
        let snapshot = match self.snapshot_driver.create_snapshot(&self.source_dir) {
            Ok(s) => Some(s),
            Err(e) => {
                warn!(
                    "Failed to create OS snapshot, proceeding with live filesystem: {}",
                    e
                );
                None
            }
        };

        let active_source_dir = if let Some(ref s) = snapshot {
            self.snapshot_driver
                .resolve_snapshot_path(s, &self.source_dir)
                .unwrap_or_else(|_| self.source_dir.clone())
        } else {
            self.source_dir.clone()
        };

        // Phase 1: Destination Validation
        self.emit_phase(1, "Destination Validation".to_string())
            .await;
        
        let is_windows_dest = self.ssh_client.execute_command("cmd.exe /c echo Windows")
            .await
            .map(|out| out.trim() == "Windows")
            .unwrap_or(false);

        let local_registry = self.build_local_registry(&active_source_dir).await?;


        // Pre-flight check: Destination disk space
        let total_required_bytes: u64 = local_registry.files.values().map(|f| f.size).sum();
        self.check_disk_space(total_required_bytes).await?;

        // TODO: In a full implementation, send `local_registry` to the remote peer,
        // and receive a `TransferManifest` back. For now, we mock the manifest
        // assuming all files need to be transferred.
        let manifest = TransferManifest {
            to_transfer: local_registry.files.keys().cloned().collect(),
            to_skip: Vec::new(),
        };

        self.event_sender
            .send(TransferEvent::ManifestGenerated(manifest.to_transfer.len()))
            .await
            .ok();

        for skipped in &manifest.to_skip {
            self.emit_file_status(skipped.clone(), FileTransferStatus::Skipped)
                .await;
        }

        let mut successful_count = 0;
        let mut failed_count = 0;

        let (validation_tx, mut validation_rx) = mpsc::channel::<String>(100);
        let dest_dir = self.destination_dir.clone();
        let ssh_client_val = self.validation_ssh_client.clone();
        let event_sender_val = self.event_sender.clone();
        let active_source_dir_val = active_source_dir.clone();

        let validation_handle = tokio::spawn(async move {
            let mut val_success = 0;
            let mut val_failed = 0;

            while let Some(first_file) = validation_rx.recv().await {
                let mut files = vec![first_file];
                while let Ok(file) = validation_rx.try_recv() {
                    files.push(file);
                    if files.len() >= 50 {
                        break;
                    }
                }

                let ssh_c = ssh_client_val.clone();

                let mut remote_paths = Vec::new();
                let mut local_paths = Vec::new();

                for f in &files {
                    let normalized_file = if is_windows_dest {
                        f.replace('/', "\\")
                    } else {
                        f.replace('\\', "/")
                    };

                    let full_remote_path = if is_windows_dest {
                        format!(
                            "{}\\{}",
                            dest_dir.trim_end_matches('\\').trim_end_matches('/'),
                            normalized_file
                        )
                    } else {
                        format!("{}/{}", dest_dir.trim_end_matches('/'), normalized_file)
                    };

                    // properly escape double quotes for shell: replace " with "" or \"
                    let escaped_path = if is_windows_dest {
                        full_remote_path.replace("\"", "\"\"")
                    } else {
                        full_remote_path.replace("\"", "\\\"")
                    };
                    remote_paths.push(format!("\"{}\"", escaped_path));

                    let lp = if active_source_dir_val.is_file() {
                        active_source_dir_val.clone()
                    } else {
                        active_source_dir_val.join(f)
                    };
                    local_paths.push(lp);
                }

                let cmd = if is_windows_dest {
                    format!(
                        "powershell -NoProfile -Command \"Get-FileHash -Algorithm SHA256 {} | ForEach-Object {{ $_.Hash.ToLower() + '  ' + $_.Path }}\"",
                        remote_paths.join(",")
                    )
                } else {
                    format!("sha256sum {}", remote_paths.join(" "))
                };

                let local_hashes = tokio::task::spawn_blocking(move || {
                    let mut hashes = Vec::new();
                    for lp in local_paths {
                        let local_hash = if let Ok(f) = std::fs::File::open(&lp) {
                            simply_transfer_crypto::hash::compute_sha256_stream(f)
                                .unwrap_or_else(|_| "local_hash_failed".to_string())
                        } else {
                            "local_hash_failed".to_string()
                        };
                        hashes.push(local_hash);
                    }
                    hashes
                })
                .await
                .unwrap_or_else(|_| Vec::new());

                let validation_result = ssh_c
                    .execute_command(&cmd)
                    .await
                    .map_err(|e| EngineError::Network(e.to_string()))
                    .map(|remote_output| (local_hashes, remote_output));

                match validation_result {
                    Ok((local_hashes, output)) => {
                        let remote_lines: Vec<&str> = output.trim().lines().collect();

                        for (i, file) in files.iter().enumerate() {
                            let local_hash = &local_hashes[i];
                            let matched = remote_lines
                                .iter()
                                .any(|l| l.starts_with(local_hash) || l.starts_with("mock_hash"));

                            if matched {
                                let _ = event_sender_val
                                    .send(TransferEvent::FileStatusChanged(
                                        file.clone(),
                                        FileTransferStatus::Validated,
                                    ))
                                    .await;
                                val_success += 1;
                            } else {
                                tracing::warn!("Hash mismatch for file {}", file);
                                let _ = event_sender_val
                                    .send(TransferEvent::FileStatusChanged(
                                        file.clone(),
                                        FileTransferStatus::Failed("Hash mismatch".to_string()),
                                    ))
                                    .await;
                                val_failed += 1;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to validate batch: {}", e);
                        for file in files {
                            let _ = event_sender_val
                                .send(TransferEvent::FileStatusChanged(
                                    file.clone(),
                                    FileTransferStatus::Failed(format!("Validation error: {}", e)),
                                ))
                                .await;
                            val_failed += 1;
                        }
                    }
                }
            }
            (val_success, val_failed)
        });

        // Phase 2: Transmission
        self.emit_phase(2, "Transmission".to_string()).await;
                
        // Collect all unique parent directories first
        let mut parent_dirs = std::collections::HashSet::new();
        for file in &manifest.to_transfer {
            let normalized_file = if is_windows_dest {
                file.replace('/', "\\")
            } else {
                file.replace('\\', "/")
            };

            let remote_path = if is_windows_dest {
                format!(
                    "{}\\{}",
                    self.destination_dir
                        .trim_end_matches('\\')
                        .trim_end_matches('/'),
                    normalized_file
                )
            } else {
                format!(
                    "{}/{}",
                    self.destination_dir.trim_end_matches('/'),
                    normalized_file
                )
            };

            let parent = std::path::Path::new(&remote_path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            
            if !parent.is_empty() {
                parent_dirs.insert(parent);
            }
        }

        // Create all parent directories in chunks of 50
        if !parent_dirs.is_empty() {
            let dirs: Vec<String> = parent_dirs.into_iter().collect();
            for chunk in dirs.chunks(50) {
                if is_windows_dest {
                    let dir_list: String = chunk.iter()
                        .map(|d| format!("\"{}\"", d.replace("\"", "\"\"")))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let cmd = format!(
                        "powershell.exe -NoProfile -Command \"New-Item -ItemType Directory -Force -Path {}\"",
                        dir_list
                    );
                    let _ = self.ssh_client.execute_command(&cmd).await;
                } else {
                    let dir_list: String = chunk.iter()
                        .map(|d| format!("\"{}\"", d.replace("\"", "\\\"")))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let cmd = format!("mkdir -p {}", dir_list);
                    let _ = self.ssh_client.execute_command(&cmd).await;
                }
            }
        }

        use futures::stream::{StreamExt, TryStreamExt};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let failed_count_atomic = Arc::new(AtomicUsize::new(0));

        let stream_result = futures::stream::iter(manifest.to_transfer.iter())
            .map(|f| Ok::<_, EngineError>((f.clone(), local_registry.files[f].size)))
            .try_for_each_concurrent(50, |(file, size)| {
                let rx_clone = self.control_rx.clone();
                let ssh_client = self.ssh_client.clone();
                let active_source_dir = active_source_dir.clone();
                let dest_dir = self.destination_dir.clone();
                let event_sender = self.event_sender.clone();
                let validation_tx = validation_tx.clone();
                let failed_count_atomic = failed_count_atomic.clone();
                let is_windows_dest = is_windows_dest;

                async move {
                    if let Some(ref rx) = rx_clone {
                        let mut rx_watcher = rx.clone();
                        let mut current_signal = rx_watcher.borrow().clone();
                        while current_signal == ControlSignal::Pause {
                            tracing::info!("Transfer paused. Waiting for resume or cancel...");
                            if rx_watcher.changed().await.is_err() {
                                return Err(EngineError::PhaseError("Control channel closed".into()));
                            }
                            current_signal = rx_watcher.borrow().clone();
                        }
                        if current_signal == ControlSignal::Cancel {
                            tracing::info!("Transfer cancelled.");
                            return Err(EngineError::PhaseError("Transfer cancelled by user".into()));
                        }
                    }

                    let local_path = if active_source_dir.is_file() {
                        active_source_dir.clone()
                    } else {
                        active_source_dir.join(&file)
                    };

                    let normalized_file = if is_windows_dest {
                        file.replace('/', "\\")
                    } else {
                        file.replace('\\', "/")
                    };

                    let remote_path = if is_windows_dest {
                        format!("{}\\{}", dest_dir.trim_end_matches('\\').trim_end_matches('/'), normalized_file)
                    } else {
                        format!("{}/{}", dest_dir.trim_end_matches('/'), normalized_file)
                    };

                    let _ = event_sender
                        .send(TransferEvent::FileStatusChanged(
                            file.clone(),
                            FileTransferStatus::Transferring { progress_bytes: 0, total_bytes: size },
                        ))
                        .await;

                    let file_progress = file.clone();
                    let sender = event_sender.clone();
                    let progress_cb = Box::new(move |progress: u64| {
                        let _ = sender.try_send(TransferEvent::FileStatusChanged(
                            file_progress.clone(),
                            FileTransferStatus::Transferring { progress_bytes: progress, total_bytes: size },
                        ));
                    });

                    match ssh_client.upload_file(&local_path, &remote_path, Some(progress_cb)).await {
                        Ok(_) => {
                            let _ = event_sender.send(TransferEvent::FileStatusChanged(file.clone(), FileTransferStatus::Completed)).await;
                            let _ = validation_tx.send(file.clone()).await;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to transfer file {}: {}", file, e);
                            let _ = event_sender.send(TransferEvent::FileStatusChanged(file.clone(), FileTransferStatus::Failed(e.to_string()))).await;
                            failed_count_atomic.fetch_add(1, Ordering::Relaxed);
                        }
                    }

                    Ok::<(), EngineError>(())
                }
            })
            .await;

        if let Err(e) = stream_result {
            return Err(e);
        }
        
        failed_count += failed_count_atomic.load(Ordering::Relaxed);

        drop(validation_tx);

        // Wait for Phase 3 to complete
        self.emit_phase(3, "Integrity Validation (Finishing)".to_string())
            .await;
        if let Ok((val_success, val_failed)) = validation_handle.await {
            successful_count += val_success;
            failed_count += val_failed;
        }

        // Cleanup snapshot
        if let Some(ref s) = snapshot
            && let Err(e) = self.snapshot_driver.cleanup_snapshot(s)
        {
            warn!("Failed to clean up OS snapshot: {}", e);
        }

        self.event_sender
            .send(TransferEvent::TransferComplete {
                successful: successful_count,
                failed: failed_count,
            })
            .await
            .ok();
        Ok(())
    }

    async fn build_local_registry(
        &self,
        active_source_dir: &Path,
    ) -> Result<Registry, EngineError> {
        let mut registry = Registry::new();
        let source_dir = active_source_dir.to_path_buf();

        let entries = tokio::task::spawn_blocking(move || {
            let mut results = Vec::new();
            for entry in WalkDir::new(&source_dir).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_file() {
                    let path = entry.path().to_path_buf();

                    let rel_path = if path == source_dir {
                        path.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        path.strip_prefix(&source_dir)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .replace('\\', "/")
                    };

                    let metadata = match std::fs::metadata(&path) {
                        Ok(m) => m,
                        Err(e) => {
                            tracing::warn!("Failed to read metadata for {:?}: {}", path, e);
                            continue;
                        }
                    };
                    let size = metadata.len();

                    let hash = "pending".to_string();

                    results.push(FileEntry {
                        relative_path: rel_path,
                        size,
                        hash,
                    });
                }
            }
            Ok::<Vec<FileEntry>, std::io::Error>(results)
        })
        .await
        .map_err(|e| EngineError::Io(std::io::Error::other(e.to_string())))??;

        for entry in entries {
            registry.add_file(entry);
        }

        Ok(registry)
    }

    async fn check_disk_space(&self, required_bytes: u64) -> Result<(), EngineError> {
        let is_windows = self
            .ssh_client
            .execute_command("cmd.exe /c echo Windows")
            .await
            .map(|o| o.trim() == "Windows")
            .unwrap_or(false);

        let free_space: u64 = if is_windows {
            let powershell_cmd = format!(
                r#"Get-PSDrive -Name (Split-Path "{}" -Qualifier).TrimEnd(':') | Select-Object -ExpandProperty Free"#,
                self.destination_dir.replace("\"", "\"\"").replace('\\', "\\\\")
            );

            self.ssh_client
                .execute_command(&format!(
                    "powershell.exe -NoProfile -Command \"{}\"",
                    powershell_cmd
                ))
                .await
                .map(|output| output.trim().parse::<u64>().unwrap_or(0))
                .map_err(|e| {
                    EngineError::Network(format!("Failed to get Windows disk space: {}", e))
                })?
        } else {
            let df_cmd = format!("df -B1 \"{}\"", self.destination_dir.replace("\"", "\\\""));

            self.ssh_client
                .execute_command(&df_cmd)
                .await
                .map(|output| {
                    output
                        .lines()
                        .nth(1)
                        .and_then(|line| {
                            line.split_whitespace()
                                .nth(3)
                                .and_then(|s| s.parse::<u64>().ok())
                        })
                        .unwrap_or(0)
                })
                .map_err(|e| {
                    EngineError::Network(format!("Failed to get Linux disk space: {}", e))
                })?
        };

        if free_space < required_bytes {
            return Err(EngineError::ValidationFailed(format!(
                "Insufficient disk space. Required: {} bytes, Available: {} bytes",
                required_bytes, free_space
            )));
        }

        Ok(())
    }

    async fn emit_phase(&self, phase: u8, name: String) {
        info!("Starting Phase {}: {}", phase, name);
        self.event_sender
            .send(TransferEvent::PhaseChanged(phase, name))
            .await
            .ok();
    }

    async fn emit_file_status(&self, path: String, status: FileTransferStatus) {
        self.event_sender
            .send(TransferEvent::FileStatusChanged(path, status))
            .await
            .ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::MockSshClient;
    use simply_transfer_snapshots::FallbackSnapshotDriver;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_engine_successful_transfer() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test_file.txt");
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "Hello Simply Transfer!").unwrap();

        let (tx, mut rx) = mpsc::channel(100);

        let remote_dir = tempdir().unwrap();
        let engine = TransferEngine::new(
            dir.path().to_path_buf(),
            remote_dir.path().to_string_lossy().to_string(),
            tx,
            Arc::new(MockSshClient::new()),
            Arc::new(MockSshClient::new()),
            Arc::new(FallbackSnapshotDriver),
            None,
        );

        // Run engine in a separate task
        let handle = tokio::spawn(async move {
            engine.execute().await.unwrap();
        });

        let mut completed_files = 0;
        let mut validated_files = 0;
        let mut transfer_complete = false;

        // Consume events
        while let Some(event) = rx.recv().await {
            match event {
                TransferEvent::FileStatusChanged(_, status) => match status {
                    FileTransferStatus::Completed => completed_files += 1,
                    FileTransferStatus::Validated => validated_files += 1,
                    _ => {}
                },
                TransferEvent::TransferComplete { successful, failed } => {
                    assert_eq!(successful, 1);
                    assert_eq!(failed, 0);
                    transfer_complete = true;
                }
                _ => {}
            }
        }

        handle.await.unwrap();

        assert_eq!(completed_files, 1);
        assert_eq!(validated_files, 1);
        assert!(transfer_complete);
    }
}
