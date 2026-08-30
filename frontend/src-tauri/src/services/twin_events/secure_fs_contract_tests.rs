use super::AnchoredRoot;
use fs2::FileExt;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::path::Path;
use tempfile::tempdir;

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(unix)]
fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[test]
fn retained_parent_capability_cannot_be_redirected_outside_anchor() {
    let temp = tempdir().unwrap();
    let anchor = temp.path().join("anchor");
    let outside = temp.path().join("outside");
    fs::create_dir_all(anchor.join("parent")).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let root = AnchoredRoot::open(&anchor).unwrap();
    let target = root.resolve_target("parent/note.md", true).unwrap();

    let displaced = anchor.join("displaced");
    if fs::rename(anchor.join("parent"), &displaced).is_err() {
        // Windows denies replacement while cap-std retains the no-share-delete
        // parent handle. That is the fail-closed outcome this test requires.
        assert!(!outside.join("note.md").exists());
        return;
    }
    if symlink_dir(&outside, &anchor.join("parent")).is_err() {
        return;
    }

    target.put_atomic(b"anchored").unwrap();
    assert_eq!(fs::read(displaced.join("note.md")).unwrap(), b"anchored");
    assert!(!outside.join("note.md").exists());
}

#[test]
fn final_symlink_is_replaced_or_removed_without_touching_its_target() {
    let temp = tempdir().unwrap();
    let anchor = temp.path().join("anchor");
    fs::create_dir(&anchor).unwrap();
    let outside = temp.path().join("outside.txt");
    fs::write(&outside, "outside").unwrap();
    let leaf = anchor.join("note.md");
    if symlink_file(&outside, &leaf).is_err() {
        return;
    }
    let root = AnchoredRoot::open(&anchor).unwrap();
    let target = root.resolve_target("note.md", false).unwrap();

    target.put_atomic(b"inside").unwrap();
    assert_eq!(fs::read(&outside).unwrap(), b"outside");
    assert_eq!(fs::read(&leaf).unwrap(), b"inside");

    fs::remove_file(&leaf).unwrap();
    symlink_file(&outside, &leaf).unwrap();
    target.delete().unwrap();
    assert_eq!(fs::read(&outside).unwrap(), b"outside");
    assert!(!leaf.exists());
}

#[test]
fn bounded_read_uses_the_single_opened_leaf_handle() {
    let temp = tempdir().unwrap();
    let anchor = temp.path().join("anchor");
    fs::create_dir(&anchor).unwrap();
    let leaf = anchor.join("lease.json");
    fs::write(&leaf, "before").unwrap();
    let root = AnchoredRoot::open(&anchor).unwrap();
    let target = root.resolve_target("lease.json", false).unwrap();

    let bytes = target
        .read_bounded_with_hook(6, || {
            fs::rename(&leaf, anchor.join("old.json")).unwrap();
            fs::write(&leaf, "after!").unwrap();
        })
        .unwrap()
        .unwrap();
    assert_eq!(bytes, b"before");
    assert_eq!(fs::read(&leaf).unwrap(), b"after!");
}

#[test]
fn exclusive_lock_retries_or_denies_leaf_replacement_and_holds_the_current_entry() {
    let temp = tempdir().unwrap();
    let anchor = temp.path().join("anchor");
    fs::create_dir_all(anchor.join("locks")).unwrap();
    let root = AnchoredRoot::open(&anchor).unwrap();
    let replaced = AtomicBool::new(false);
    let lock = root
        .lock_exclusive_with_hook("locks/mutation.lock", || {
            let leaf = anchor.join("locks/mutation.lock");
            if fs::rename(&leaf, anchor.join("locks/old.lock")).is_ok() {
                fs::write(&leaf, b"replacement").unwrap();
                replaced.store(true, Ordering::SeqCst);
            }
        })
        .unwrap();
    assert_eq!(lock.key(), "locks/mutation.lock");
    let current = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(anchor.join("locks/mutation.lock"))
        .unwrap();
    assert!(current.try_lock_exclusive().is_err());
    lock.unlock().unwrap();
    current.try_lock_exclusive().unwrap();
    FileExt::unlock(&current).unwrap();
    if replaced.load(Ordering::SeqCst) {
        assert_eq!(fs::read(anchor.join("locks/mutation.lock")).unwrap(), b"replacement");
    }
}

#[test]
fn exclusive_lock_parent_replacement_never_follows_outside_anchor() {
    let temp = tempdir().unwrap();
    let anchor = temp.path().join("anchor");
    let outside = temp.path().join("outside");
    fs::create_dir_all(anchor.join("locks")).unwrap();
    fs::create_dir(&outside).unwrap();
    let root = AnchoredRoot::open(&anchor).unwrap();
    let result = root.lock_exclusive_with_hook("locks/mutation.lock", || {
        if fs::rename(anchor.join("locks"), anchor.join("displaced-locks")).is_ok() {
            let _ = symlink_dir(&outside, &anchor.join("locks"));
        }
    });
    if let Ok(lock) = result {
        lock.unlock().unwrap();
    }
    assert!(!outside.join("mutation.lock").exists());
}
