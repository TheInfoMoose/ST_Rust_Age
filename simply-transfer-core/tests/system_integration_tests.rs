use simply_transfer_core::engine::TransferEngine;
use simply_transfer_core::ssh::MockSshClient;
use simply_transfer_snapshots::FallbackSnapshotDriver;
use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_cross_platform_pathing_normalization() {
    let dir = tempdir().unwrap();
    let source_dir = dir.path();

    // Simulate a nested directory structure
    let nested_dir = source_dir.join("subfolder");
    std::fs::create_dir(&nested_dir).unwrap();

    let file_path = nested_dir.join("test_file.txt");
    let mut file = File::create(&file_path).unwrap();
    writeln!(file, "cross platform test").unwrap();

    let (tx, _rx) = mpsc::channel(100);

    let ssh_client = Arc::new(MockSshClient::new());
    
    let remote_dir = tempdir().unwrap();
    let remote_dest = remote_dir.path().to_string_lossy().to_string().replace('\\', "/");

    let engine = TransferEngine::new(
        source_dir.to_path_buf(),
        remote_dest.clone(),
        tx,
        ssh_client.clone(),
        Arc::new(FallbackSnapshotDriver),
        None,
    );

    engine.execute().await.unwrap();

    // Verify the remote path sent to SshClient was normalized to POSIX format
    let uploaded_paths = ssh_client.uploaded_paths.lock().unwrap();
    assert_eq!(uploaded_paths.len(), 1);

    let path = &uploaded_paths[0];

    // Check that it starts with the destination prefix
    assert!(path.starts_with(&format!("{}/", remote_dest)));

    // Check that it contains forward slashes for the subfolder
    assert!(path.contains("/subfolder/test_file.txt"));

    // Check that it does NOT contain backslashes, even if run on Windows
    assert!(!path.contains('\\'));
}
