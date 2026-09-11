//! Capability-scoped atomic writes using a random, no-follow, same-directory temporary
//! file. The selected parent directory is opened once and retained through publication,
//! so a path swap or preplanted symlink cannot redirect the write.

use crate::services::twin_events::AnchoredRoot;
use std::path::Path;

/// Write `contents` to `path` atomically.
///
/// Writes to a temporary file in the same directory as `path` (so the final `rename` is
/// same-volume and therefore atomic), fsyncs the temp file's contents to disk, then
/// renames it over the target. If the rename fails, the temp file is removed on a
/// best-effort basis so failed writes don't leave litter behind.
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "atomic target needs a filename",
        )
    })?;
    let root = AnchoredRoot::open_external_directory(dir).map_err(|error| {
        std::io::Error::other(format!("atomic parent acquisition failed: {error}"))
    })?;
    root.put_atomic_leaf(file_name, contents)
        .map_err(|error| std::io::Error::other(format!("atomic publication failed: {error}")))
}

/// Test helper: assert that `dir` contains no random atomic-write temp litter.
/// Used by per-store adoption tests to prove each store's public write API leaves
/// no temp files behind.
#[cfg(test)]
pub fn assert_no_tmp_siblings(dir: &Path) {
    let leftovers: Vec<_> = std::fs::read_dir(dir)
        .expect("read_dir for tmp-sibling check")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|p| is_atomic_temporary(p))
        .collect();
    assert!(
        leftovers.is_empty(),
        "expected no atomic temp files in {}, found: {:?}",
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
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|p| is_atomic_temporary(p))
            .collect()
    }

    #[cfg(unix)]
    fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    #[cfg(unix)]
    fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
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
        //
        // The guarantee under test is ATOMICITY, not that concurrent racers never
        // error: on Windows, a rename onto a target mid-replacement can exhaust
        // the bounded transient retry and surface PermissionDenied (seen rarely
        // under full-suite parallel load). One side failing that way is
        // acceptable — as long as at least one write wins and the file is intact.
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("concurrent.json");

        let payload_a = vec![b'a'; 64 * 1024];
        let payload_b = vec![b'b'; 64 * 1024];

        let (res_a, res_b) = std::thread::scope(|scope| {
            let path_a = path.clone();
            let path_b = path.clone();
            let a = &payload_a;
            let b = &payload_b;
            let ta = scope.spawn(move || write_atomic(&path_a, a));
            let tb = scope.spawn(move || write_atomic(&path_b, b));
            (ta.join().expect("thread a"), tb.join().expect("thread b"))
        });

        let transient_only_loss = |res: &std::io::Result<()>| match res {
            Ok(()) => true,
            Err(err) => err.kind() == std::io::ErrorKind::PermissionDenied,
        };
        assert!(
            res_a.is_ok() || res_b.is_ok(),
            "at least one concurrent write must succeed: a={res_a:?} b={res_b:?}"
        );
        assert!(
            transient_only_loss(&res_a) && transient_only_loss(&res_b),
            "a losing racer may only fail with retry-exhausted PermissionDenied: a={res_a:?} b={res_b:?}"
        );

        let content = std::fs::read(&path).expect("file should exist");
        assert!(
            content == payload_a || content == payload_b,
            "final content must be exactly one payload, intact"
        );
        assert!(tmp_siblings(&path).is_empty());
    }

    #[test]
    fn preplanted_exact_temp_and_final_symlinks_cannot_redirect_atomic_write() {
        let dir = tempdir().expect("tempdir");
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, b"private victim").unwrap();
        let path = dir.path().join("selected.png");
        let temporary_name = format!(".{}.tmp", uuid::Uuid::new_v4());
        let preplanted = dir.path().join(&temporary_name);
        if let Err(error) = symlink_file(&victim, &preplanted) {
            if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Unsupported
            ) || error.raw_os_error() == Some(1314)
            {
                return;
            }
            panic!("preplant symlink: {error}");
        }

        let root = AnchoredRoot::open_external_directory(dir.path()).unwrap();
        let error = root
            .put_atomic_leaf_with_temporary_for_test(
                path.file_name().unwrap(),
                std::ffi::OsStr::new(&temporary_name),
                b"first payload",
            )
            .unwrap_err();
        assert!(error.to_string().contains("temporary publication"));
        assert_eq!(std::fs::read(&victim).unwrap(), b"private victim");
        assert!(!path.exists());

        symlink_file(&victim, &path).unwrap();
        write_atomic(&path, b"replacement payload").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"private victim");
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement payload");
        assert!(!std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn intermediate_directory_reparse_is_rejected_when_supported() {
        let dir = tempdir().expect("tempdir");
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let redirect = dir.path().join("redirect");
        if let Err(error) = symlink_dir(&outside, &redirect) {
            if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Unsupported
            ) || error.raw_os_error() == Some(1314)
            {
                return;
            }
            panic!("intermediate symlink: {error}");
        }
        let redirected_target = redirect.join("selected.png");
        assert!(write_atomic(&redirected_target, b"private").is_err());
        assert!(!outside.join("selected.png").exists());
    }

    #[test]
    fn parent_traversal_is_unconditionally_rejected() {
        let dir = tempdir().expect("tempdir");
        let safe = dir.path().join("safe");
        std::fs::create_dir(&safe).unwrap();
        let traversing_target = safe.join("..").join("escaped.png");
        assert!(write_atomic(&traversing_target, b"private").is_err());
        assert!(!dir.path().join("escaped.png").exists());
    }
}

#[cfg(test)]
fn is_atomic_temporary(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix('.'))
        .and_then(|name| name.strip_suffix(".tmp"))
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .is_some_and(|value| value.get_version_num() == 4)
}
