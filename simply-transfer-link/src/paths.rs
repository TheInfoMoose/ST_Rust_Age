use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::LinkError;

#[derive(Debug, Clone)]
pub struct ShareRoot {
    pub id: String,
    pub path: PathBuf,
    pub read: bool,
    pub write: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SharePolicy {
    roots: HashMap<String, Vec<ShareRoot>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

impl SharePolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn grant(&mut self, fp: &str, root: ShareRoot) {
        self.roots.entry(fp.to_string()).or_default().push(root);
    }

    pub fn roots_for(&self, fp: &str) -> &[ShareRoot] {
        self.roots.get(fp).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn resolve(
        &self,
        fp: &str,
        root_id: &str,
        rel: &str,
        access: Access,
    ) -> Result<PathBuf, LinkError> {
        // 2a. Find root for fp + root_id and check read/write flag.
        let root = self
            .roots_for(fp)
            .iter()
            .find(|r| r.id == root_id)
            .ok_or_else(|| LinkError::Path("unknown root".to_string()))?;

        match access {
            Access::Read => {
                if !root.read {
                    return Err(LinkError::Path("read not permitted".to_string()));
                }
            }
            Access::Write => {
                if !root.write {
                    return Err(LinkError::Path("write not permitted".to_string()));
                }
            }
        }

        // 2b. Validate rel.
        if rel.is_empty() && access == Access::Write {
            return Err(LinkError::Path("invalid path".to_string()));
        }
        if rel.contains('\0') || rel.starts_with('/') || rel.starts_with('\\') {
            return Err(LinkError::Path("invalid path".to_string()));
        }
        let bytes = rel.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return Err(LinkError::Path("invalid path".to_string()));
        }

        let mut parts: Vec<&str> = Vec::new();
        for comp in rel.split(['/', '\\']) {
            if comp == ".." {
                return Err(LinkError::Path("invalid path".to_string()));
            }
            if !comp.is_empty() && comp != "." {
                parts.push(comp);
            }
        }

        // 2c. Canonicalize root and walk up to deepest existing ancestor via lstat.
        let canonical_root = std::fs::canonicalize(&root.path)
            .map_err(|_| LinkError::Path("root unavailable".to_string()))?;

        let mut candidate = canonical_root.clone();
        for part in &parts {
            candidate.push(part);
        }

        let mut ancestor = candidate;
        let mut remaining_count = 0usize;
        while std::fs::symlink_metadata(&ancestor).is_err() {
            if remaining_count >= parts.len() {
                return Err(LinkError::Path("root unavailable".to_string()));
            }
            if !ancestor.pop() {
                return Err(LinkError::Path("root unavailable".to_string()));
            }
            remaining_count += 1;
        }

        let canonical_ancestor = std::fs::canonicalize(&ancestor)
            .map_err(|_| LinkError::Path("escapes root".to_string()))?;

        if !canonical_ancestor.starts_with(&canonical_root) {
            return Err(LinkError::Path("escapes root".to_string()));
        }

        // 2d. Return joined path (canonical ancestor + remaining components).
        let mut resolved = canonical_ancestor;
        for part in &parts[parts.len() - remaining_count..] {
            resolved.push(part);
        }
        Ok(resolved)
    }
}

pub fn open_for_write(path: &Path, offset: u64) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    if offset == 0 {
        opts.write(true).create(true).truncate(true);
    } else {
        opts.write(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = opts.open(path)?;
    if offset > 0 {
        file.seek(SeekFrom::Start(offset))?;
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_FP: &str = "SHA256:testpeer";

    fn policy_with_root(path: PathBuf, read: bool, write: bool) -> SharePolicy {
        let mut policy = SharePolicy::new();
        policy.grant(
            TEST_FP,
            ShareRoot {
                id: "share".to_string(),
                path,
                read,
                write,
            },
        );
        policy
    }

    #[test]
    fn resolve_relative_subpath_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let policy = policy_with_root(tmp.path().to_path_buf(), true, true);

        let resolved = policy
            .resolve(TEST_FP, "share", "a/b.txt", Access::Write)
            .unwrap();
        let expected = std::fs::canonicalize(tmp.path())
            .unwrap()
            .join("a")
            .join("b.txt");
        assert_eq!(resolved, expected);

        let root_read = policy.resolve(TEST_FP, "share", "", Access::Read).unwrap();
        assert_eq!(root_read, std::fs::canonicalize(tmp.path()).unwrap());

        let empty_write = policy
            .resolve(TEST_FP, "share", "", Access::Write)
            .unwrap_err();
        assert!(matches!(empty_write, LinkError::Path(_)));
    }

    #[test]
    fn resolve_rejects_traversal_and_absolute_or_drive_or_unc_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let policy = policy_with_root(tmp.path().to_path_buf(), true, true);

        for bad in ["../x", "a/../../x", "/etc/passwd", "C:\\x", "\\\\srv\\s"] {
            for access in [Access::Read, Access::Write] {
                let err = policy.resolve(TEST_FP, "share", bad, access).unwrap_err();
                assert!(
                    matches!(err, LinkError::Path(_)),
                    "expected LinkError::Path for {bad:?} ({access:?})"
                );
            }
        }
    }

    #[test]
    fn resolve_rejects_nul_byte() {
        let tmp = tempfile::tempdir().unwrap();
        let policy = policy_with_root(tmp.path().to_path_buf(), true, true);

        let err = policy
            .resolve(TEST_FP, "share", "a/b\0c.txt", Access::Read)
            .unwrap_err();
        assert!(matches!(err, LinkError::Path(_)));
    }

    #[test]
    fn resolve_rejects_write_on_read_only_root() {
        let tmp = tempfile::tempdir().unwrap();
        let policy = policy_with_root(tmp.path().to_path_buf(), true, false);

        let err = policy
            .resolve(TEST_FP, "share", "a/b.txt", Access::Write)
            .unwrap_err();
        assert!(matches!(err, LinkError::Path(_)));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_rejects_symlink_pointing_outside_root() {
        let root_dir = tempfile::tempdir().unwrap();
        let outside_dir = tempfile::tempdir().unwrap();
        std::fs::write(outside_dir.path().join("file"), b"secret").unwrap();

        std::os::unix::fs::symlink(outside_dir.path(), root_dir.path().join("link")).unwrap();

        let policy = policy_with_root(root_dir.path().to_path_buf(), true, true);

        for access in [Access::Read, Access::Write] {
            let err = policy
                .resolve(TEST_FP, "share", "link/file", access)
                .unwrap_err();
            assert!(matches!(err, LinkError::Path(_)));
        }
    }

    #[cfg(unix)]
    #[test]
    fn resolve_rejects_dangling_symlink_pointing_outside_root() {
        let root_dir = tempfile::tempdir().unwrap();
        let outside_dir = tempfile::tempdir().unwrap();
        let nonexistent_outside = outside_dir.path().join("nonexistent_dir");

        std::os::unix::fs::symlink(&nonexistent_outside, root_dir.path().join("link")).unwrap();

        let policy = policy_with_root(root_dir.path().to_path_buf(), true, true);

        let err = policy
            .resolve(TEST_FP, "share", "link/new.txt", Access::Write)
            .unwrap_err();
        assert!(matches!(err, LinkError::Path(_)));
    }

    #[cfg(unix)]
    #[test]
    fn open_for_write_on_symlink_fails() {
        use std::io::Write;

        let tmp = tempfile::tempdir().unwrap();
        let real_file = tmp.path().join("real.txt");
        std::fs::write(&real_file, b"hello").unwrap();

        let link_file = tmp.path().join("link.txt");
        std::os::unix::fs::symlink(&real_file, &link_file).unwrap();

        assert!(open_for_write(&link_file, 0).is_err());
        assert!(open_for_write(&link_file, 2).is_err());

        // Also verify normal create (offset == 0) and resume (offset > 0) on a regular file.
        let regular = tmp.path().join("regular.txt");
        {
            let mut f = open_for_write(&regular, 0).unwrap();
            f.write_all(b"abcdef").unwrap();
        }
        {
            let mut f = open_for_write(&regular, 3).unwrap();
            f.write_all(b"XYZ").unwrap();
        }
        assert_eq!(std::fs::read(&regular).unwrap(), b"abcXYZ");
    }
}
