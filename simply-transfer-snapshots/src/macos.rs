use crate::{Snapshot, SnapshotDriver, SnapshotError};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::warn;

pub struct ApfsSnapshotDriver;

impl ApfsSnapshotDriver {
    pub fn new() -> Self {
        Self
    }
}

impl SnapshotDriver for ApfsSnapshotDriver {
    fn create_snapshot(&self, volume_path: &Path) -> Result<Snapshot, SnapshotError> {
        warn!("APFS snapshotting requires elevated privileges (tmutil / diskutil).");

        // Example: `tmutil localsnapshot` or `diskutil apfs snapshot /`
        let output = Command::new("tmutil")
            .arg("localsnapshot")
            .output()
            .map_err(|e| SnapshotError::CreationFailed(e.to_string()))?;

        if !output.status.success() {
            return Err(SnapshotError::CreationFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        // Parse stdout for the snapshot name, e.g., "Created local snapshot with date: 2026-09-25-103000"
        let output_str = String::from_utf8_lossy(&output.stdout);
        let id = output_str
            .trim()
            .split_whitespace()
            .last()
            .unwrap_or("unknown")
            .to_string();

        Ok(Snapshot {
            id,
            // APFS local snapshots are accessible via /Volumes/com.apple.TimeMachine.localsnapshots/
            mount_point: PathBuf::from(format!(
                "/Volumes/com.apple.TimeMachine.localsnapshots/Backups.backupdb/Mac/{}/Macintosh HD",
                "id-placeholder"
            )),
        })
    }

    fn resolve_snapshot_path(
        &self,
        _snapshot: &Snapshot,
        original_path: &Path,
    ) -> Result<PathBuf, SnapshotError> {
        // TODO: proper resolution against the mounted APFS snapshot point
        Ok(original_path.to_path_buf())
    }

    fn cleanup_snapshot(&self, snapshot: &Snapshot) -> Result<(), SnapshotError> {
        // Example: `tmutil deletelocalsnapshots <date>`
        let status = Command::new("tmutil")
            .arg("deletelocalsnapshots")
            .arg(&snapshot.id)
            .status()
            .map_err(|e| SnapshotError::CleanupFailed(e.to_string()))?;

        if !status.success() {
            return Err(SnapshotError::CleanupFailed(
                "tmutil failed to delete snapshot".to_string(),
            ));
        }
        Ok(())
    }
}
