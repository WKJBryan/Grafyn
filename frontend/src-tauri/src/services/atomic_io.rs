//! Atomic file writes: write to a same-directory temp file, fsync, then rename over the
//! target. On NTFS/ext4 (same volume) `rename` is atomic, so readers never observe a
//! partially-written file: if the writing process is interrupted, the target is either
//! the old complete content or the new complete content, never a truncated mix of both.
//!
//! Durability note: the temp file's contents are fsynced (`File::sync_all`) before the
//! rename, but the parent directory entry is not fsynced afterwards. After a power loss
//! the rename itself may not have reached disk (the old content survives), but the
//! target is never left truncated.

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

/// Test helper: assert that `dir` contains no `*.tmp-*` litter from `write_atomic`.
/// Used by per-store adoption tests to prove each store's public write API leaves
/// no temp files behind.
#[cfg(test)]
pub fn assert_no_tmp_siblings(dir: &Path) {
    let leftovers: Vec<_> = std::fs::read_dir(dir)
        .expect("read_dir for tmp-sibling check")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains(".tmp-"))
                .unwrap_or(false)
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "expected no *.tmp-* files in {}, found: {:?}",
        dir.display(),
        leftovers
    );
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
        // Make the target path an existing non-empty DIRECTORY. Creating the temp
        // file next to it (same parent) succeeds, but renaming a file over a
        // non-empty directory fails on Windows and Unix alike — so this exercises
        // the rename-failure cleanup path specifically, not File::create failure.
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("target.json");
        std::fs::create_dir(&path).expect("create target as directory");
        std::fs::write(path.join("occupant.txt"), b"x").expect("make directory non-empty");

        let result = write_atomic(&path, b"payload");
        assert!(
            result.is_err(),
            "write should fail: cannot rename over a non-empty directory"
        );

        assert!(
            tmp_siblings(&path).is_empty(),
            "expected the temp file to be cleaned up after rename failure"
        );
    }

    #[test]
    fn concurrent_writes_leave_one_intact_payload() {
        // Two threads racing to write the same target must not collide on the
        // same temp name (pid alone is not enough within one process — the
        // counter disambiguates). Whichever rename lands last wins, and the final
        // file must be one of the two payloads intact, never interleaved.
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("concurrent.json");

        let payload_a = vec![b'a'; 64 * 1024];
        let payload_b = vec![b'b'; 64 * 1024];

        std::thread::scope(|scope| {
            let path_a = path.clone();
            let path_b = path.clone();
            let a = &payload_a;
            let b = &payload_b;
            let ta = scope.spawn(move || write_atomic(&path_a, a));
            let tb = scope.spawn(move || write_atomic(&path_b, b));
            ta.join().expect("thread a").expect("write a");
            tb.join().expect("thread b").expect("write b");
        });

        let content = std::fs::read(&path).expect("file should exist");
        assert!(
            content == payload_a || content == payload_b,
            "final content must be exactly one payload, intact"
        );
        assert!(tmp_siblings(&path).is_empty());
    }
}
