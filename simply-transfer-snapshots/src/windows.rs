use crate::{Snapshot, SnapshotDriver, SnapshotError};
use std::path::{Path, PathBuf};
use tracing::warn;

pub struct VssSnapshotDriver;

impl VssSnapshotDriver {
    pub fn new() -> Self {
        Self
    }
}

impl SnapshotDriver for VssSnapshotDriver {
    fn create_snapshot(&self, volume_path: &Path) -> Result<Snapshot, SnapshotError> {
        let drive = volume_path
            .to_string_lossy()
            .chars()
            .take(2)
            .collect::<String>()
            + "\\";

        let script = format!(
            "$class = Get-WmiObject -List Win32_ShadowCopy; \
             $res = $class.Create('{}', 'ClientAccessible'); \
             if ($res.ReturnValue -ne 0) {{ exit 1 }}; \
             $shadow = Get-WmiObject Win32_ShadowCopy | Where-Object {{ $_.ID -eq $res.ShadowID }}; \
             Write-Output ($shadow.ID + '|' + $shadow.DeviceObject)",
            drive
        );

        let output = std::process::Command::new("powershell")
            .args(&["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .map_err(|e| SnapshotError::CreationFailed(e.to_string()))?;

        if !output.status.success() {
            return Err(SnapshotError::CreationFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = output_str.trim().split('|').collect();
        if parts.len() != 2 {
            return Err(SnapshotError::CreationFailed(
                "Failed to parse WMI ShadowCopy output".to_string(),
            ));
        }

        Ok(Snapshot {
            id: parts[0].to_string(),
            mount_point: PathBuf::from(parts[1]),
        })
    }

    fn resolve_snapshot_path(
        &self,
        snapshot: &Snapshot,
        original_path: &Path,
    ) -> Result<PathBuf, SnapshotError> {
        let path_str = original_path.to_string_lossy();
        if path_str.len() > 2 && path_str.chars().nth(1) == Some(':') {
            let relative = &path_str[2..];
            let relative = relative.trim_start_matches('\\');
            Ok(snapshot.mount_point.join(relative))
        } else {
            Ok(original_path.to_path_buf())
        }
    }

    fn cleanup_snapshot(&self, snapshot: &Snapshot) -> Result<(), SnapshotError> {
        let script = format!(
            "$shadow = Get-WmiObject Win32_ShadowCopy | Where-Object {{ $_.ID -eq '{}' }}; \
             if ($shadow) {{ $shadow.Delete() }}",
            snapshot.id
        );

        let output = std::process::Command::new("powershell")
            .args(&["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .map_err(|e| SnapshotError::CleanupFailed(e.to_string()))?;

        if !output.status.success() {
            return Err(SnapshotError::CleanupFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        Ok(())
    }
}
