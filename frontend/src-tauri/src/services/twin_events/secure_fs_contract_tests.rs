use super::{AnchoredRoot, NoClobberInstallOutcome};
use fs2::FileExt;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
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
        assert_eq!(
            fs::read(anchor.join("locks/mutation.lock")).unwrap(),
            b"replacement"
        );
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

#[test]
fn no_replace_rename_preserves_a_racing_regular_file_destination() {
    let temp = tempdir().unwrap();
    let anchor = temp.path().join("anchor");
    fs::create_dir_all(anchor.join("from")).unwrap();
    fs::create_dir_all(anchor.join("to")).unwrap();
    fs::write(anchor.join("from/source.json"), b"source").unwrap();
    let root = AnchoredRoot::open(&anchor).unwrap();

    let result =
        root.rename_no_replace_with_hook("from/source.json", "to/destination.json", false, || {
            fs::write(anchor.join("to/destination.json"), b"foreign").unwrap()
        });

    assert!(result.is_err());
    assert_eq!(
        fs::read(anchor.join("from/source.json")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(anchor.join("to/destination.json")).unwrap(),
        b"foreign"
    );
}

#[test]
fn no_replace_rename_preserves_a_racing_directory_destination() {
    let temp = tempdir().unwrap();
    let anchor = temp.path().join("anchor");
    fs::create_dir_all(anchor.join("from/source")).unwrap();
    fs::create_dir_all(anchor.join("to")).unwrap();
    fs::write(anchor.join("from/source/source.json"), b"source").unwrap();
    let root = AnchoredRoot::open(&anchor).unwrap();

    let result = root.rename_no_replace_with_hook("from/source", "to/destination", false, || {
        fs::create_dir(anchor.join("to/destination")).unwrap();
        fs::write(anchor.join("to/destination/foreign.json"), b"foreign").unwrap();
    });

    assert!(result.is_err());
    assert_eq!(
        fs::read(anchor.join("from/source/source.json")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(anchor.join("to/destination/foreign.json")).unwrap(),
        b"foreign"
    );
}

#[test]
fn no_replace_rename_remains_bound_to_the_resolved_destination_parent() {
    let temp = tempdir().unwrap();
    let anchor = temp.path().join("anchor");
    fs::create_dir_all(anchor.join("from")).unwrap();
    fs::create_dir_all(anchor.join("to")).unwrap();
    fs::write(anchor.join("from/source.json"), b"source").unwrap();
    let root = AnchoredRoot::open(&anchor).unwrap();
    let parent_replaced = AtomicBool::new(false);

    root.rename_no_replace_with_hook("from/source.json", "to/destination.json", false, || {
        if fs::rename(anchor.join("to"), anchor.join("resolved-to")).is_ok() {
            fs::create_dir(anchor.join("to")).unwrap();
            fs::write(anchor.join("to/replacement.json"), b"replacement").unwrap();
            parent_replaced.store(true, Ordering::SeqCst);
        }
    })
    .unwrap();

    assert!(parent_replaced.load(Ordering::SeqCst));
    assert_eq!(
        fs::read(anchor.join("resolved-to/destination.json")).unwrap(),
        b"source"
    );
    assert!(!anchor.join("to/destination.json").exists());
    assert_eq!(
        fs::read(anchor.join("to/replacement.json")).unwrap(),
        b"replacement"
    );
}

#[test]
fn no_clobber_install_reports_whether_it_created_the_destination() {
    let temp = tempdir().unwrap();
    let anchor = temp.path().join("anchor");
    fs::create_dir(&anchor).unwrap();
    let root = AnchoredRoot::open(&anchor).unwrap();

    assert_eq!(
        root.install_no_clobber_with_outcome("owned.json", "staging", b"owned")
            .unwrap(),
        NoClobberInstallOutcome::Installed
    );
    assert_eq!(
        root.install_no_clobber_with_outcome("owned.json", "staging", b"foreign")
            .unwrap(),
        NoClobberInstallOutcome::AlreadyExists
    );
    assert_eq!(fs::read(anchor.join("owned.json")).unwrap(), b"owned");
}

#[test]
fn hard_link_install_preserves_the_source_file_identity() {
    let temp = tempdir().unwrap();
    let anchor = temp.path().join("anchor");
    fs::create_dir(&anchor).unwrap();
    fs::write(anchor.join("witness.json"), b"owned").unwrap();
    let root = AnchoredRoot::open(&anchor).unwrap();

    assert_eq!(
        root.hard_link_no_clobber("witness.json", "descriptor.json", false)
            .unwrap(),
        NoClobberInstallOutcome::Installed
    );
    assert_eq!(
        root.hard_link_no_clobber("witness.json", "descriptor.json", false)
            .unwrap(),
        NoClobberInstallOutcome::AlreadyExists
    );
    assert!(root
        .same_regular_file("witness.json", "descriptor.json")
        .unwrap());
    assert_eq!(
        root.regular_file_identity("witness.json").unwrap(),
        root.regular_file_identity("descriptor.json").unwrap()
    );
}

#[test]
fn identity_bound_quarantine_removes_the_matching_regular_file_from_its_live_name() {
    let temp = tempdir().unwrap();
    let anchor = temp.path().join("anchor");
    fs::create_dir(&anchor).unwrap();
    fs::write(anchor.join("owned.json"), b"owned").unwrap();
    let root = AnchoredRoot::open(&anchor).unwrap();
    let identity = root.regular_file_identity("owned.json").unwrap().unwrap();

    assert!(root
        .quarantine_regular_file_if_identity("owned.json", identity)
        .unwrap());
    assert!(!anchor.join("owned.json").exists());
    let retained = fs::read_dir(&anchor)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(retained.len(), 1);
    assert!(retained[0].starts_with('.'));
    assert!(retained[0].ends_with(".delete"));
    assert_eq!(fs::read(anchor.join(&retained[0])).unwrap(), b"owned");
    assert_eq!(
        root.regular_file_identity(&retained[0]).unwrap(),
        Some(identity)
    );
}

#[test]
fn identity_bound_quarantine_preserves_a_racing_same_byte_replacement() {
    let temp = tempdir().unwrap();
    let anchor = temp.path().join("anchor");
    fs::create_dir(&anchor).unwrap();
    let target = anchor.join("owned.json");
    fs::write(&target, b"same bytes").unwrap();
    fs::hard_link(&target, anchor.join("original-witness.json")).unwrap();
    let root = AnchoredRoot::open(&anchor).unwrap();
    let original_identity = root.regular_file_identity("owned.json").unwrap().unwrap();

    let deleted = root
        .quarantine_regular_file_if_identity_with_hook("owned.json", original_identity, || {
            fs::remove_file(&target).unwrap();
            fs::write(&target, b"same bytes").unwrap();
        })
        .unwrap();

    assert!(!deleted);
    assert_eq!(fs::read(&target).unwrap(), b"same bytes");
    assert_ne!(
        root.regular_file_identity("owned.json").unwrap().unwrap(),
        original_identity
    );
    assert_eq!(
        root.regular_file_identity("original-witness.json")
            .unwrap()
            .unwrap(),
        original_identity
    );
}
