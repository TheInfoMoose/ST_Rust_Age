use simply_transfer_core::engine::{FileTransferStatus, TransferEngine, TransferEvent};
use simply_transfer_core::ssh::MockSshClient;
use simply_transfer_snapshots::FallbackSnapshotDriver;
use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::sync::mpsc;

/// Helper function to create a file of a specific size (in MB) filled with repeating data.
fn create_dummy_file(path: &std::path::Path, size_mb: usize) {
    let mut file = File::create(path).unwrap();
    let buffer = vec![0xAB; 1024 * 1024]; // 1 MB buffer
    for _ in 0..size_mb {
        file.write_all(&buffer).unwrap();
    }
}

/// Helper function to create thousands of small files.
fn create_many_files(dir: &std::path::Path, count: usize) {
    let buffer = b"small data block for quickbooks emulation";
    for i in 0..count {
        let path = dir.join(format!("qb_db_file_{}.dat", i));
        let mut file = File::create(&path).unwrap();
        file.write_all(buffer).unwrap();
    }
}

async fn run_engine_and_measure(source_dir: std::path::PathBuf, expected_success: usize) {
    let (tx, mut rx) = mpsc::channel(1000);

    let engine = TransferEngine::new(
        source_dir,
        "/remote/backup".to_string(),
        tx,
        Arc::new(MockSshClient::new()),
        Arc::new(FallbackSnapshotDriver),
        None,
    );

    let start_time = std::time::Instant::now();

    let handle = tokio::spawn(async move {
        engine.execute().await.unwrap();
    });

    let mut completed = 0;
    let mut validated = 0;
    let mut transfer_complete = false;

    while let Some(event) = rx.recv().await {
        match event {
            TransferEvent::FileStatusChanged(_, status) => match status {
                FileTransferStatus::Completed => completed += 1,
                FileTransferStatus::Validated => validated += 1,
                _ => {}
            },
            TransferEvent::TransferComplete { successful, failed } => {
                assert_eq!(successful, expected_success);
                assert_eq!(failed, 0);
                transfer_complete = true;
            }
            _ => {}
        }
    }

    handle.await.unwrap();

    let duration = start_time.elapsed();
    println!(
        "Test completed in {:?} (Completed: {}, Validated: {})",
        duration, completed, validated
    );

    assert_eq!(completed, expected_success);
    assert_eq!(validated, expected_success);
    assert!(transfer_complete);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_stress_numerous_files() {
    let dir = tempdir().unwrap();
    let count = 2000; // 2,000 files to simulate a large directory
    println!("Generating {} files...", count);
    create_many_files(dir.path(), count);
    println!("Executing engine...");
    run_engine_and_measure(dir.path().to_path_buf(), count).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_stress_large_file() {
    let dir = tempdir().unwrap();
    println!("Generating 500 MB file...");
    let file_path = dir.path().join("large_database.qbw");
    create_dummy_file(&file_path, 500); // 500 MB file
    println!("Executing engine...");
    run_engine_and_measure(dir.path().to_path_buf(), 1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_stress_numerous_large_files() {
    let dir = tempdir().unwrap();
    let count = 20; // 20 files of 25MB each = 500MB
    println!("Generating {} files of 25 MB each...", count);
    for i in 0..count {
        let file_path = dir.path().join(format!("db_part_{}.tlg", i));
        create_dummy_file(&file_path, 25);
    }
    println!("Executing engine...");
    run_engine_and_measure(dir.path().to_path_buf(), count).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_extreme_10000_files() {
    let dir = tempdir().unwrap();
    let count = 10000;
    println!("Generating {} files...", count);
    create_many_files(dir.path(), count);
    println!("Executing engine...");
    run_engine_and_measure(dir.path().to_path_buf(), count).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Takes 20+ minutes and requires 100GB sparse file support (fails on Windows CI)"]
async fn test_extreme_100gb_file() {
    let dir = tempdir().unwrap();
    println!("Generating 100 GB sparse file...");
    let file_path = dir.path().join("massive_database.qbw");

    // Create a sparse file to avoid physically writing 100GB to the disk
    let file = File::create(&file_path).unwrap();
    let size_100gb: u64 = 100 * 1024 * 1024 * 1024;
    file.set_len(size_100gb).unwrap();
    drop(file);

    println!("Executing engine against 100GB file (this will take a moment to stream and hash)...");
    run_engine_and_measure(dir.path().to_path_buf(), 1).await;
}
