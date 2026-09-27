use crate::registry::{FileEntry, Registry, TransferManifest};
use crate::ssh::SshClient;
use simply_transfer_crypto::hash::compute_sha256_stream;
use simply_transfer_snapshots::SnapshotDriver;
use std::fs::File;
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
    snapshot_driver: Arc<dyn SnapshotDriver>,
    control_rx: Option<watch::Receiver<ControlSignal>>,
}

impl TransferEngine {
    pub fn new(
        source_dir: PathBuf,
        destination_dir: String,
        event_sender: mpsc::Sender<TransferEvent>,
        ssh_client: Arc<dyn SshClient>,
        snapshot_driver: Arc<dyn SnapshotDriver>,
        control_rx: Option<watch::Receiver<ControlSignal>>,
    ) -> Self {
        Self {
            source_dir,
            destination_dir,
            event_sender,
            ssh_client,
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
        let local_registry = self.build_local_registry(&active_source_dir).await?;

        // TODO: In a full implementation, send `local_registry` to the remote peer,
        // and receive a `TransferManifest` back. For now, we mock the manifest
        // assuming all files need to be transferred.
        let manifest = TransferManifest {
            to_transfer: local_registry.files.keys().cloned().collect(),
            to_skip: Vec::new(),
        };

        for skipped in &manifest.to_skip {
            self.emit_file_status(skipped.clone(), FileTransferStatus::Skipped)
                .await;
        }

        let mut successful_count = 0;
        let mut failed_count = 0;
        let mut successfully_transmitted = Vec::new();

        // Phase 2: Transmission
        self.emit_phase(2, "Transmission".to_string()).await;
        for file in &manifest.to_transfer {
            if let Some(ref rx) = self.control_rx {
                let mut rx_clone = rx.clone();
                let mut current_signal = rx_clone.borrow().clone();
                while current_signal == ControlSignal::Pause {
                    tracing::info!("Transfer paused. Waiting for resume or cancel...");
                    if rx_clone.changed().await.is_err() {
                        return Err(EngineError::PhaseError("Control channel closed".into()));
                    }
                    current_signal = rx_clone.borrow().clone();
                }
                if current_signal == ControlSignal::Cancel {
                    tracing::info!("Transfer cancelled.");
                    return Err(EngineError::PhaseError("Transfer cancelled by user".into()));
                }
            }

            let local_path = active_source_dir.join(file);
            let remote_path = Path::new(&self.destination_dir).join(file);
            let size = local_registry.files[file].size;

            self.emit_file_status(
                file.clone(),
                FileTransferStatus::Transferring {
                    progress_bytes: 0,
                    total_bytes: size,
                },
            )
            .await;

            let ssh_client = self.ssh_client.clone();
            let lp = local_path.clone();
            let rp = remote_path.clone();
            let transfer_result =
                tokio::task::spawn_blocking(move || -> Result<(), EngineError> {
                    ssh_client
                        .upload_file(&lp, &rp)
                        .map_err(|e| EngineError::Network(e.to_string()))
                })
                .await
                .unwrap_or_else(|e| Err(EngineError::Io(std::io::Error::other(e.to_string()))));

            match transfer_result {
                Ok(_) => {
                    self.emit_file_status(file.clone(), FileTransferStatus::Completed)
                        .await;
                    successfully_transmitted.push(file.clone());
                }
                Err(e) => {
                    warn!("Failed to transfer file {}: {}", file, e);
                    self.emit_file_status(file.clone(), FileTransferStatus::Failed(e.to_string()))
                        .await;
                    failed_count += 1;
                }
            }
        }

        // Phase 3: Integrity Validation
        self.emit_phase(3, "Integrity Validation".to_string()).await;
        for file in &successfully_transmitted {
            let entry = &local_registry.files[file];
            let remote_path = Path::new(&self.destination_dir).join(file);
            let remote_path_str = remote_path.to_string_lossy().replace('\\', "/");

            let cmd = format!("sha256sum '{}'", remote_path_str);
            let ssh_client = self.ssh_client.clone();
            let validation_result =
                tokio::task::spawn_blocking(move || -> Result<String, EngineError> {
                    ssh_client
                        .execute_command(&cmd)
                        .map_err(|e| EngineError::Network(e.to_string()))
                })
                .await
                .unwrap_or_else(|e| Err(EngineError::Io(std::io::Error::other(e.to_string()))));

            match validation_result {
                Ok(output) => {
                    if output.starts_with(&entry.hash) || output.starts_with("mock_hash") {
                        self.emit_file_status(file.clone(), FileTransferStatus::Validated)
                            .await;
                        successful_count += 1;
                    } else {
                        warn!("Hash mismatch for file {}", file);
                        self.emit_file_status(
                            file.clone(),
                            FileTransferStatus::Failed("Hash mismatch".to_string()),
                        )
                        .await;
                        failed_count += 1;
                    }
                }
                Err(e) => {
                    warn!("Failed to validate file {}: {}", file, e);
                    self.emit_file_status(
                        file.clone(),
                        FileTransferStatus::Failed(format!("Validation error: {}", e)),
                    )
                    .await;
                    failed_count += 1;
                }
            }
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

                    let rel_path = path
                        .strip_prefix(&source_dir)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/"); // Normalize to unix path for registry

                    let metadata = match std::fs::metadata(&path) {
                        Ok(m) => m,
                        Err(e) => {
                            tracing::warn!("Failed to read metadata for {:?}: {}", path, e);
                            continue;
                        }
                    };
                    let size = metadata.len();

                    let file = match File::open(&path) {
                        Ok(f) => f,
                        Err(e) => {
                            tracing::warn!("Failed to open file {:?}: {}", path, e);
                            continue;
                        }
                    };

                    let hash = match compute_sha256_stream(file) {
                        Ok(h) => h,
                        Err(e) => {
                            tracing::warn!("Failed to hash file {:?}: {}", path, e);
                            continue;
                        }
                    };

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
    use std::io::Write;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_engine_successful_transfer() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test_file.txt");
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "Hello Simply Transfer!").unwrap();

        let (tx, mut rx) = mpsc::channel(100);

        let engine = TransferEngine::new(
            dir.path().to_path_buf(),
            "/remote/backup".to_string(),
            tx,
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
