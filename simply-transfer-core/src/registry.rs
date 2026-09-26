use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents a single file in the transfer registry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileEntry {
    /// The normalized relative path of the file.
    pub relative_path: String,
    /// The size of the file in bytes.
    pub size: u64,
    /// The SHA-256 hash of the file.
    pub hash: String,
}

/// The registry sent from the source to the destination in Phase 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry {
    pub files: HashMap<String, FileEntry>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    pub fn add_file(&mut self, entry: FileEntry) {
        self.files.insert(entry.relative_path.clone(), entry);
    }
}

/// The manifest returned by the destination detailing which files to transfer or skip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferManifest {
    /// Files that are missing or differ and must be transferred.
    pub to_transfer: Vec<String>,
    /// Files that are identical and can be skipped.
    pub to_skip: Vec<String>,
}

impl TransferManifest {
    /// Compare a source registry against a destination registry to generate a manifest.
    pub fn compare(source: &Registry, destination: &Registry) -> Self {
        let mut to_transfer = Vec::new();
        let mut to_skip = Vec::new();

        for (path, src_entry) in &source.files {
            if let Some(dest_entry) = destination.files.get(path) {
                if src_entry.hash == dest_entry.hash && src_entry.size == dest_entry.size {
                    to_skip.push(path.clone());
                } else {
                    to_transfer.push(path.clone());
                }
            } else {
                to_transfer.push(path.clone());
            }
        }

        Self {
            to_transfer,
            to_skip,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transfer_manifest_compare() {
        let mut source = Registry::new();
        let mut dest = Registry::new();

        // 1. Identical file -> should be skipped
        let file1 = FileEntry {
            relative_path: "file1.txt".to_string(),
            size: 100,
            hash: "abc".to_string(),
        };
        source.add_file(file1.clone());
        dest.add_file(file1.clone());

        // 2. Modified file (different hash) -> should be transferred
        let file2_src = FileEntry {
            relative_path: "file2.txt".to_string(),
            size: 200,
            hash: "def_new".to_string(),
        };
        let file2_dest = FileEntry {
            relative_path: "file2.txt".to_string(),
            size: 200,
            hash: "def_old".to_string(),
        };
        source.add_file(file2_src.clone());
        dest.add_file(file2_dest.clone());

        // 3. New file (in source, missing in dest) -> should be transferred
        let file3_src = FileEntry {
            relative_path: "new_file.txt".to_string(),
            size: 300,
            hash: "ghi".to_string(),
        };
        source.add_file(file3_src.clone());

        let manifest = TransferManifest::compare(&source, &dest);

        assert_eq!(manifest.to_skip.len(), 1);
        assert!(manifest.to_skip.contains(&"file1.txt".to_string()));

        assert_eq!(manifest.to_transfer.len(), 2);
        assert!(manifest.to_transfer.contains(&"file2.txt".to_string()));
        assert!(manifest.to_transfer.contains(&"new_file.txt".to_string()));
    }
}
