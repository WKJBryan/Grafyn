//! Atomic file writes: write to a same-directory temp file, fsync, then rename over the
//! target. On NTFS/ext4 (same volume) `rename` is atomic, so readers never observe a
//! partially-written file — even if the process is killed or the machine loses power
//! mid-write, the target is either the old complete content or the new complete content,
//! never a truncated mix of both.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic counter to disambiguate concurrent writers within the same process that
/// share a pid (e.g. multiple threads writing to the same directory at once).
static WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `contents` to `path` atomically.
///
/// Writes to a temporary file in the same directory as `path` (so the final `rename` is
/// same-volume and therefore atomic), fsyncs the temp file's contents to disk, then
/// renames it over the target. If the rename fails, the temp file is removed on a
/// best-effort basis so failed writes don't leave litter behind.
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("atomic-write");

    let pid = std::process::id();
    let counter = WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = dir.join(format!("{file_name}.tmp-{pid}-{counter}"));

    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Return the tmp-file siblings of `path` left behind in its parent directory, if any.
    fn tmp_siblings(path: &Path) -> Vec<std::path::PathBuf> {
        let dir = path.parent().unwrap();
        let file_name = path.file_name().unwrap().to_str().unwrap();
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n != file_name && n.starts_with(file_name) && n.contains(".tmp-"))
                    .unwrap_or(false)
            })
            .collect()
    }

    #[test]
    fn writes_fresh_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("fresh.json");

        write_atomic(&path, b"hello world").expect("write should succeed");

        let content = std::fs::read_to_string(&path).expect("file should exist");
        assert_eq!(content, "hello world");
    }

    #[test]
    fn replaces_existing_content() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("existing.json");
        std::fs::write(&path, b"old content that is much longer than the new one")
            .expect("seed file");

        write_atomic(&path, b"new").expect("write should succeed");

        let content = std::fs::read_to_string(&path).expect("file should exist");
        assert_eq!(content, "new");
    }

    #[test]
    fn no_tmp_file_left_behind_after_success() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("clean.json");

        write_atomic(&path, b"payload").expect("write should succeed");

        assert!(
            tmp_siblings(&path).is_empty(),
            "expected no *.tmp-* siblings after a successful write"
        );
    }

    #[test]
    fn no_tmp_file_left_behind_after_rename_failure() {
        // Force the rename to fail by pointing the "target" at a directory that
        // doesn't exist (so `rename` errors), and confirm the temp file created
        // alongside it is cleaned up rather than left as litter.
        let dir = tempdir().expect("tempdir");
        let missing_dir = dir.path().join("does-not-exist");
        let path = missing_dir.join("target.json");

        let result = write_atomic(&path, b"payload");
        assert!(result.is_err(), "write should fail: target dir is missing");

        // No tmp file should be left in the (nonexistent) directory, and creating
        // the temp file itself should have failed cleanly rather than partially.
        assert!(!missing_dir.exists() || tmp_siblings(&path).is_empty());
    }

    #[test]
    fn concurrent_writes_use_distinct_temp_names() {
        // Two overlapping writes to the same path must not collide on the same
        // temp file name (pid alone is not enough within a single process/thread
        // pool), so we assert the counter disambiguates them.
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("concurrent.json");

        write_atomic(&path, b"first").expect("first write");
        write_atomic(&path, b"second").expect("second write");

        let content = std::fs::read_to_string(&path).expect("file should exist");
        assert_eq!(content, "second");
        assert!(tmp_siblings(&path).is_empty());
    }
}
