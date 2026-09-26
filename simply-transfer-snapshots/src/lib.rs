use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SnapshotError {
    #[error("Snapshot creation failed: {0}")]
    CreationFailed(String),
    #[error("Path resolution failed: {0}")]
    ResolutionFailed(String),
    #[error("Snapshot cleanup failed: {0}")]
    CleanupFailed(String),
    #[error("Unsupported platform or file system: {0}")]
    Unsupported(String),
    #[error("Elevated privileges required for this operation")]
    PrivilegeRequired,
}

/// Represents an active OS-level snapshot.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// A platform-specific identifier for the snapshot.
    /// (e.g., VSS Shadow Copy ID, APFS snapshot name, or LVM volume name)
    pub id: String,
    /// The mount point or prefix of the snapshot volume.
    pub mount_point: PathBuf,
}

/// A common trait abstracting native OS shadow-copy/snapshotting mechanisms.
pub trait SnapshotDriver: Send + Sync {
    /// Create a point-in-time snapshot of the volume containing the given path.
    fn create_snapshot(&self, volume_path: &Path) -> Result<Snapshot, SnapshotError>;

    /// Translate a standard file path into its snapshot-relative equivalent
    /// so it can be safely read even if exclusively locked by an application.
    fn resolve_snapshot_path(&self, snapshot: &Snapshot, original_path: &Path) -> Result<PathBuf, SnapshotError>;

    /// Release and clean up the snapshot from the operating system.
    fn cleanup_snapshot(&self, snapshot: &Snapshot) -> Result<(), SnapshotError>;
}

/// A fallback driver that simply returns the original path.
/// Used when elevated privileges are unavailable or snapshots are unsupported.
pub struct FallbackSnapshotDriver;

impl SnapshotDriver for FallbackSnapshotDriver {
    fn create_snapshot(&self, _volume_path: &Path) -> Result<Snapshot, SnapshotError> {
        Ok(Snapshot {
            id: "fallback".to_string(),
            mount_point: PathBuf::new(),
        })
    }

    fn resolve_snapshot_path(&self, _snapshot: &Snapshot, original_path: &Path) -> Result<PathBuf, SnapshotError> {
        Ok(original_path.to_path_buf())
    }

    fn cleanup_snapshot(&self, _snapshot: &Snapshot) -> Result<(), SnapshotError> {
        Ok(())
    }
}

// Platform specific module exports
#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod linux;

/// Factory function to get the appropriate driver for the current platform.
pub fn get_native_driver() -> Box<dyn SnapshotDriver> {
    #[cfg(target_os = "windows")]
    {
        Box::new(windows::VssSnapshotDriver::new())
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::ApfsSnapshotDriver::new())
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxSnapshotDriver::new())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Box::new(FallbackSnapshotDriver)
    }
}
