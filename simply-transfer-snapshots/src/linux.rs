use crate::{Snapshot, SnapshotDriver, SnapshotError};
use std::path::{Path, PathBuf};
use std::process::Command;
pub struct LinuxSnapshotDriver;

impl Default for LinuxSnapshotDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl LinuxSnapshotDriver {
    pub fn new() -> Self {
        Self
    }
}

impl SnapshotDriver for LinuxSnapshotDriver {
    fn snapshot_type(&self) -> &'static str {
        "LVM"
    }

    fn create_snapshot(&self, volume_path: &Path) -> Result<Snapshot, SnapshotError> {
        let snap_id = format!(
            "st_snap_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        );
        let mount_point = PathBuf::from(format!("/mnt/{}", snap_id));

        // Create mount directory
        std::fs::create_dir_all(&mount_point).map_err(|e| {
            SnapshotError::CreationFailed(format!("Failed to create mount directory: {}", e))
        })?;

        // Create LVM snapshot
        let output = Command::new("lvcreate")
            .args([
                "-s",
                "-n",
                &snap_id,
                "-l",
                "100%FREE",
                volume_path.to_str().unwrap_or(""),
            ])
            .output()
            .map_err(|e| {
                SnapshotError::CreationFailed(format!("Failed to execute lvcreate: {}", e))
            })?;

        if !output.status.success() {
            return Err(SnapshotError::CreationFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        // Mount the snapshot (assuming the vg name can be extracted, we'll try a generic mount by finding the newly created block device)
        // Note: In a production app, the volume group parsing requires more complex logic.
        let dev_path = format!("/dev/mapper/*-{}", snap_id.replace("_", "--"));

        let mount_output = Command::new("sh")
            .args([
                "-c",
                &format!("mount {} {}", dev_path, mount_point.display()),
            ])
            .output()
            .map_err(|e| {
                SnapshotError::CreationFailed(format!("Failed to execute mount: {}", e))
            })?;

        if !mount_output.status.success() {
            return Err(SnapshotError::CreationFailed(
                String::from_utf8_lossy(&mount_output.stderr).to_string(),
            ));
        }

        Ok(Snapshot {
            id: snap_id,
            mount_point,
        })
    }

    fn resolve_snapshot_path(
        &self,
        snapshot: &Snapshot,
        original_path: &Path,
    ) -> Result<PathBuf, SnapshotError> {
        Ok(snapshot
            .mount_point
            .join(original_path.strip_prefix("/").unwrap_or(original_path)))
    }

    fn cleanup_snapshot(&self, snapshot: &Snapshot) -> Result<(), SnapshotError> {
        // Unmount
        let umount_output = Command::new("umount")
            .arg(&snapshot.mount_point)
            .output()
            .map_err(|e| {
                SnapshotError::CleanupFailed(format!("Failed to execute umount: {}", e))
            })?;

        if !umount_output.status.success() {
            tracing::warn!(
                "Failed to unmount snapshot: {}",
                String::from_utf8_lossy(&umount_output.stderr)
            );
        }

        // Remove snapshot volume (using wildcards via sh)
        let dev_path = format!("/dev/mapper/*-{}", snapshot.id.replace("_", "--"));
        let rm_output = Command::new("sh")
            .args(["-c", &format!("lvremove -f {}", dev_path)])
            .output()
            .map_err(|e| {
                SnapshotError::CleanupFailed(format!("Failed to execute lvremove: {}", e))
            })?;

        if !rm_output.status.success() {
            return Err(SnapshotError::CleanupFailed(
                String::from_utf8_lossy(&rm_output.stderr).to_string(),
            ));
        }

        // Remove mount directory
        let _ = std::fs::remove_dir(&snapshot.mount_point);

        Ok(())
    }
}
