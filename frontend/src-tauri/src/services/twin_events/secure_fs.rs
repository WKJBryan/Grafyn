use crate::services::twin_events::{validate_relative_key, MutationError};
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use fs2::FileExt;
use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(crate) struct AnchoredRoot {
    canonical_path: PathBuf,
    dir: Dir,
}

impl std::fmt::Debug for AnchoredRoot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AnchoredRoot")
            .field("canonical_path", &self.canonical_path)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnchoredEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoClobberInstallOutcome {
    Installed,
    AlreadyExists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RegularFileIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
}

pub(crate) struct AnchoredExclusiveLock {
    file: std::fs::File,
    root: AnchoredRoot,
    #[cfg(test)]
    key: String,
}

impl std::ops::Deref for AnchoredExclusiveLock {
    type Target = std::fs::File;

    fn deref(&self) -> &Self::Target {
        &self.file
    }
}

impl AnchoredExclusiveLock {
    pub(crate) fn unlock(self) -> io::Result<()> {
        FileExt::unlock(&self.file)
    }

    pub(crate) fn root_path(&self) -> &Path {
        self.root.canonical_path()
    }

    #[cfg(test)]
    pub(crate) fn key(&self) -> &str {
        &self.key
    }
}

impl AnchoredRoot {
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self, MutationError> {
        crate::services::twin_events::validate_real_directory(
            path.as_ref(),
            "capability filesystem root",
        )?;
        let canonical_path = std::fs::canonicalize(path.as_ref())?;
        let dir = Dir::open_ambient_dir(&canonical_path, ambient_authority())?;
        if !dir.dir_metadata()?.is_dir() {
            return Err(MutationError::Invalid(
                "capability filesystem root is not a directory".into(),
            ));
        }
        Ok(Self {
            canonical_path,
            dir,
        })
    }

    pub(crate) fn open_external_directory(path: &Path) -> Result<Self, MutationError> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        #[cfg(windows)]
        {
            let dir = open_external_windows_directory(&absolute)?;
            return Ok(Self {
                canonical_path: absolute,
                dir,
            });
        }
        #[cfg(not(windows))]
        {
            let mut anchor = PathBuf::new();
            let mut descendants = Vec::new();
            for component in absolute.components() {
                match component {
                    std::path::Component::Prefix(prefix) => anchor.push(prefix.as_os_str()),
                    std::path::Component::RootDir => anchor.push(component.as_os_str()),
                    std::path::Component::CurDir => {}
                    std::path::Component::ParentDir => {
                        return Err(MutationError::Invalid(
                            "external atomic directory cannot contain parent traversal".into(),
                        ))
                    }
                    std::path::Component::Normal(name) => descendants.push(name.to_os_string()),
                }
            }
            if anchor.as_os_str().is_empty() {
                return Err(MutationError::Invalid(
                    "external atomic directory must resolve from a filesystem root".into(),
                ));
            }
            let mut dir = Dir::open_ambient_dir(&anchor, ambient_authority())?;
            for descendant in descendants {
                dir = dir.open_dir_nofollow(&descendant).map_err(|error| {
                    MutationError::RecoveryConflict(format!(
                        "external atomic parent traversal failed at {}: {error}",
                        descendant.to_string_lossy()
                    ))
                })?;
            }
            if !dir.dir_metadata()?.is_dir() {
                return Err(MutationError::Invalid(
                    "external atomic parent is not a directory".into(),
                ));
            }
            let dir = reopen_external_dir_with_identity(&absolute, &dir)?;
            Ok(Self {
                canonical_path: absolute,
                dir,
            })
        }
    }

    pub(crate) fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    pub(crate) fn try_clone(&self) -> Result<Self, MutationError> {
        Ok(Self {
            canonical_path: self.canonical_path.clone(),
            dir: self.dir.try_clone()?,
        })
    }

    pub(crate) fn lock_exclusive(
        &self,
        relative_key: &str,
    ) -> Result<AnchoredExclusiveLock, MutationError> {
        self.lock_exclusive_inner(relative_key, || {})
    }

    #[cfg(test)]
    pub(crate) fn lock_exclusive_with_hook(
        &self,
        relative_key: &str,
        hook: impl FnOnce(),
    ) -> Result<AnchoredExclusiveLock, MutationError> {
        self.lock_exclusive_inner(relative_key, hook)
    }

    fn lock_exclusive_inner(
        &self,
        relative_key: &str,
        hook: impl FnOnce(),
    ) -> Result<AnchoredExclusiveLock, MutationError> {
        validate_relative_key(relative_key)?;
        let mut hook = Some(hook);
        for _ in 0..4 {
            let target = self.resolve_target(relative_key, true)?;
            let file = target.open_lock_file()?;
            file.lock_exclusive()?;
            if let Some(hook) = hook.take() {
                hook();
            }
            let current = self.resolve_target(relative_key, false)?.open_regular()?;
            let Some(current) = current else {
                FileExt::unlock(&file)?;
                continue;
            };
            if same_file(&file, &current)? {
                return Ok(AnchoredExclusiveLock {
                    file,
                    root: self.try_clone()?,
                    #[cfg(test)]
                    key: relative_key.to_string(),
                });
            }
            FileExt::unlock(&file)?;
        }
        Err(MutationError::RecoveryConflict(
            "capability lock entry changed during acquisition".into(),
        ))
    }

    pub(crate) fn resolve_target(
        &self,
        relative_key: &str,
        create_parents: bool,
    ) -> Result<AnchoredTarget, MutationError> {
        self.try_resolve_target(relative_key, create_parents)?
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound).into())
    }

    fn try_resolve_target(
        &self,
        relative_key: &str,
        create_parents: bool,
    ) -> Result<Option<AnchoredTarget>, MutationError> {
        validate_relative_key(relative_key)?;
        let mut components = relative_key.split('/').collect::<Vec<_>>();
        let leaf = components
            .pop()
            .ok_or_else(|| MutationError::Invalid("target key is empty".into()))?;
        let mut parent = self.dir.try_clone()?;
        for component in components {
            parent = match parent.open_dir_nofollow(component) {
                Ok(next) => next,
                Err(error) if error.kind() == io::ErrorKind::NotFound && create_parents => {
                    match parent.create_dir(component) {
                        Ok(()) => sync_dir(&parent)?,
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error.into()),
                    }
                    parent.open_dir_nofollow(component)?
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
            };
        }
        Ok(Some(AnchoredTarget {
            parent,
            leaf: OsString::from(leaf),
        }))
    }

    pub(crate) fn read_bounded(
        &self,
        relative_key: &str,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, MutationError> {
        match self.try_resolve_target(relative_key, false)? {
            Some(target) => target.read_bounded(limit),
            None => Ok(None),
        }
    }

    pub(crate) fn put_atomic(&self, relative_key: &str, bytes: &[u8]) -> Result<(), MutationError> {
        self.resolve_target(relative_key, true)?.put_atomic(bytes)
    }

    pub(crate) fn put_atomic_leaf(&self, leaf: &OsStr, bytes: &[u8]) -> Result<(), MutationError> {
        if !leaf_name_is_relative(leaf) {
            return Err(MutationError::Invalid(
                "capability atomic target must be one filename".into(),
            ));
        }
        AnchoredTarget {
            parent: self.dir.try_clone()?,
            leaf: leaf.to_os_string(),
        }
        .put_atomic(bytes)
    }

    #[cfg(test)]
    pub(crate) fn put_atomic_leaf_with_temporary_for_test(
        &self,
        leaf: &OsStr,
        temporary: &OsStr,
        bytes: &[u8],
    ) -> Result<(), MutationError> {
        if !leaf_name_is_relative(leaf) || !leaf_name_is_relative(temporary) {
            return Err(MutationError::Invalid(
                "capability atomic test target must be one filename".into(),
            ));
        }
        AnchoredTarget {
            parent: self.dir.try_clone()?,
            leaf: leaf.to_os_string(),
        }
        .put_atomic_with_temporary(temporary.to_os_string(), bytes)
    }

    pub(crate) fn delete(&self, relative_key: &str) -> Result<(), MutationError> {
        match self.try_resolve_target(relative_key, false)? {
            Some(target) => target.delete(),
            None => Ok(()),
        }
    }

    pub(crate) fn rename(
        &self,
        from: &str,
        to: &str,
        create_to_parents: bool,
    ) -> Result<(), MutationError> {
        let from = self.resolve_target(from, false)?;
        let to = self.resolve_target(to, create_to_parents)?;
        from.parent.rename(&from.leaf, &to.parent, &to.leaf)?;
        sync_dir(&from.parent)?;
        sync_dir(&to.parent)?;
        Ok(())
    }

    pub(crate) fn rename_no_replace(
        &self,
        from: &str,
        to: &str,
        create_to_parents: bool,
    ) -> Result<(), MutationError> {
        self.rename_no_replace_inner(from, to, create_to_parents, || {})
    }

    pub(crate) fn hard_link_no_clobber(
        &self,
        from: &str,
        to: &str,
        create_to_parents: bool,
    ) -> Result<NoClobberInstallOutcome, MutationError> {
        let from = self.resolve_target(from, false)?;
        let to = self.resolve_target(to, create_to_parents)?;
        if from.open_regular()?.is_none() {
            return Err(io::Error::from(io::ErrorKind::NotFound).into());
        }
        let outcome = match from.parent.hard_link(&from.leaf, &to.parent, &to.leaf) {
            Ok(()) => NoClobberInstallOutcome::Installed,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                NoClobberInstallOutcome::AlreadyExists
            }
            Err(error) => return Err(error.into()),
        };
        sync_dir(&to.parent)?;
        Ok(outcome)
    }

    #[allow(dead_code)]
    pub(crate) fn same_regular_file(&self, left: &str, right: &str) -> Result<bool, MutationError> {
        let left = self.resolve_target(left, false)?;
        let right = self.resolve_target(right, false)?;
        let (Some(left), Some(right)) = (left.open_regular()?, right.open_regular()?) else {
            return Ok(false);
        };
        same_cap_file(&left, &right)
    }

    pub(crate) fn regular_file_identity(
        &self,
        relative_key: &str,
    ) -> Result<Option<RegularFileIdentity>, MutationError> {
        let Some(target) = self.try_resolve_target(relative_key, false)? else {
            return Ok(None);
        };
        target
            .open_regular()?
            .map(|file| cap_file_identity(&file))
            .transpose()
    }

    pub(crate) fn quarantine_regular_file_if_identity(
        &self,
        relative_key: &str,
        expected: RegularFileIdentity,
    ) -> Result<bool, MutationError> {
        self.quarantine_regular_file_if_identity_inner(relative_key, expected, || {})
    }

    #[cfg(test)]
    pub(crate) fn quarantine_regular_file_if_identity_with_hook(
        &self,
        relative_key: &str,
        expected: RegularFileIdentity,
        hook: impl FnOnce(),
    ) -> Result<bool, MutationError> {
        self.quarantine_regular_file_if_identity_inner(relative_key, expected, hook)
    }

    fn quarantine_regular_file_if_identity_inner(
        &self,
        relative_key: &str,
        expected: RegularFileIdentity,
        hook: impl FnOnce(),
    ) -> Result<bool, MutationError> {
        if self.regular_file_identity(relative_key)? != Some(expected) {
            return Ok(false);
        }
        hook();

        let cleanup_key = random_sibling_key(relative_key)?;
        self.rename_no_replace(relative_key, &cleanup_key, false)?;
        let quarantined_identity = self.regular_file_identity(&cleanup_key)?.ok_or_else(|| {
            MutationError::RecoveryConflict(
                "identity-bound delete quarantine disappeared before validation".into(),
            )
        })?;
        if quarantined_identity != expected {
            self.rename_no_replace(&cleanup_key, relative_key, false)
                .map_err(|error| {
                    MutationError::RecoveryConflict(format!(
                        "identity-bound delete preserved a replacement in quarantine but could not restore its path: {error}"
                    ))
                })?;
            return Ok(false);
        }
        Ok(true)
    }

    #[allow(dead_code)]
    pub(crate) fn delete_if_same_regular_file(
        &self,
        reference: &str,
        target: &str,
    ) -> Result<bool, MutationError> {
        let Some(reference_identity) = self.regular_file_identity(reference)? else {
            return Ok(false);
        };
        self.quarantine_regular_file_if_identity(target, reference_identity)
    }

    #[cfg(test)]
    pub(crate) fn rename_no_replace_with_hook(
        &self,
        from: &str,
        to: &str,
        create_to_parents: bool,
        hook: impl FnOnce(),
    ) -> Result<(), MutationError> {
        self.rename_no_replace_inner(from, to, create_to_parents, hook)
    }

    fn rename_no_replace_inner(
        &self,
        from: &str,
        to: &str,
        create_to_parents: bool,
        hook: impl FnOnce(),
    ) -> Result<(), MutationError> {
        let from_target = self.resolve_target(from, false)?;
        let to_target = self.resolve_target(to, create_to_parents)?;
        if let Err(error) = rename_target_no_replace(from_target, to_target, hook) {
            return Err(MutationError::RecoveryConflict(format!(
                "capability no-replace rename failed at destination {to}: {error}"
            )));
        }
        Ok(())
    }

    pub(crate) fn install_no_clobber(
        &self,
        destination: &str,
        staging_directory: &str,
        bytes: &[u8],
    ) -> Result<(), MutationError> {
        self.install_no_clobber_with_outcome(destination, staging_directory, bytes)
            .map(|_| ())
    }

    pub(crate) fn install_no_clobber_with_outcome(
        &self,
        destination: &str,
        staging_directory: &str,
        bytes: &[u8],
    ) -> Result<NoClobberInstallOutcome, MutationError> {
        validate_relative_key(staging_directory)?;
        let temporary = format!("{staging_directory}/.{}.tmp", Uuid::new_v4());
        let staging = self.resolve_target(&temporary, true)?;
        let destination = self.resolve_target(destination, true)?;
        staging.put_new(bytes)?;
        let result =
            match staging
                .parent
                .hard_link(&staging.leaf, &destination.parent, &destination.leaf)
            {
                Ok(()) => {
                    sync_dir(&destination.parent)?;
                    Ok(NoClobberInstallOutcome::Installed)
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    sync_dir(&destination.parent)?;
                    Ok(NoClobberInstallOutcome::AlreadyExists)
                }
                Err(error) => Err(error.into()),
            };
        let cleanup = staging.parent.remove_file_or_symlink(&staging.leaf);
        if let Err(error) = cleanup {
            if error.kind() != io::ErrorKind::NotFound {
                return Err(error.into());
            }
        }
        sync_dir(&staging.parent)?;
        result
    }

    pub(crate) fn regular_file_names(
        &self,
        relative_directory: &str,
    ) -> Result<Vec<String>, MutationError> {
        self.regular_file_names_bounded(relative_directory, usize::MAX)
    }

    pub(crate) fn regular_file_names_bounded(
        &self,
        relative_directory: &str,
        max_entries: usize,
    ) -> Result<Vec<String>, MutationError> {
        validate_relative_key(relative_directory)?;
        let directory = self.open_directory(relative_directory, false)?;
        let mut names = Vec::new();
        for entry in directory.entries()? {
            let entry = entry?;
            let name = entry.file_name();
            let metadata = directory.symlink_metadata(&name)?;
            if metadata.is_symlink() || !metadata.is_file() {
                return Err(MutationError::Invalid(format!(
                    "capability directory contains a non-regular entry: {}",
                    name.to_string_lossy()
                )));
            }
            let name = name
                .to_str()
                .ok_or_else(|| MutationError::Invalid("filename is not UTF-8".into()))?;
            if names.len() >= max_entries {
                return Err(MutationError::Invalid(format!(
                    "capability directory exceeds its {max_entries}-entry limit"
                )));
            }
            names.push(name.to_string());
        }
        names.sort();
        Ok(names)
    }

    pub(crate) fn directory_entries(
        &self,
        relative_directory: &str,
    ) -> Result<Vec<(String, AnchoredEntryKind)>, MutationError> {
        self.directory_entries_bounded(relative_directory, usize::MAX)
    }

    pub(crate) fn directory_entries_bounded(
        &self,
        relative_directory: &str,
        max_entries: usize,
    ) -> Result<Vec<(String, AnchoredEntryKind)>, MutationError> {
        validate_relative_key(relative_directory)?;
        let directory = self.open_directory(relative_directory, false)?;
        let mut entries = Vec::new();
        for entry in directory.entries()? {
            let entry = entry?;
            let name = entry.file_name();
            let metadata = directory.symlink_metadata(&name)?;
            if metadata.is_symlink() {
                return Err(MutationError::Invalid(format!(
                    "capability directory contains a symlink: {}",
                    name.to_string_lossy()
                )));
            }
            let kind = if metadata.is_file() {
                AnchoredEntryKind::File
            } else if metadata.is_dir() {
                AnchoredEntryKind::Directory
            } else {
                return Err(MutationError::Invalid(format!(
                    "capability directory contains an unsupported entry: {}",
                    name.to_string_lossy()
                )));
            };
            let name = name
                .to_str()
                .ok_or_else(|| MutationError::Invalid("filename is not UTF-8".into()))?;
            validate_relative_key(name)?;
            if entries.len() >= max_entries {
                return Err(MutationError::Invalid(format!(
                    "capability directory exceeds its {max_entries}-entry limit"
                )));
            }
            entries.push((name.to_string(), kind));
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(entries)
    }

    pub(crate) fn open_directory(
        &self,
        relative_directory: &str,
        create: bool,
    ) -> Result<Dir, MutationError> {
        validate_relative_key(relative_directory)?;
        let mut directory = self.dir.try_clone()?;
        for component in relative_directory.split('/') {
            directory = match directory.open_dir_nofollow(component) {
                Ok(next) => next,
                Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
                    match directory.create_dir(component) {
                        Ok(()) => sync_dir(&directory)?,
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error.into()),
                    }
                    directory.open_dir_nofollow(component)?
                }
                Err(error) => return Err(error.into()),
            };
        }
        Ok(directory)
    }

    pub(crate) fn directory_exists(&self, relative_directory: &str) -> Result<bool, MutationError> {
        validate_relative_key(relative_directory)?;
        let mut directory = self.dir.try_clone()?;
        for component in relative_directory.split('/') {
            directory = match directory.open_dir_nofollow(component) {
                Ok(next) => next,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error.into()),
            };
        }
        Ok(true)
    }
}

pub(crate) struct AnchoredTarget {
    parent: Dir,
    leaf: OsString,
}

#[cfg(unix)]
fn rename_target_replace(from: AnchoredTarget, to: AnchoredTarget) -> Result<(), MutationError> {
    rename_replace_with_retry(&from.parent, &from.leaf, &to.parent, &to.leaf)?;
    sync_dir(&from.parent)?;
    sync_dir(&to.parent)
}

#[cfg(windows)]
fn rename_target_replace(from: AnchoredTarget, to: AnchoredTarget) -> Result<(), MutationError> {
    rename_target_windows(from, to, true, || {})
}

#[cfg(unix)]
fn rename_target_no_replace(
    from: AnchoredTarget,
    to: AnchoredTarget,
    hook: impl FnOnce(),
) -> Result<(), MutationError> {
    use rustix::fs::{renameat_with, RenameFlags};

    hook();
    renameat_with(
        &from.parent,
        from.leaf.as_os_str(),
        &to.parent,
        to.leaf.as_os_str(),
        RenameFlags::NOREPLACE,
    )
    .map_err(io::Error::from)?;
    sync_dir(&from.parent)?;
    sync_dir(&to.parent)
}

#[cfg(windows)]
fn rename_target_no_replace(
    from: AnchoredTarget,
    to: AnchoredTarget,
    hook: impl FnOnce(),
) -> Result<(), MutationError> {
    rename_target_windows(from, to, false, hook)
}

#[cfg(windows)]
fn rename_target_windows(
    from: AnchoredTarget,
    to: AnchoredTarget,
    replace_if_exists: bool,
    hook: impl FnOnce(),
) -> Result<(), MutationError> {
    use cap_std::fs::OpenOptionsExt;
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Wdk::Storage::FileSystem::{
        FileRenameInformation, NtSetInformationFile, FILE_RENAME_INFORMATION,
    };
    use windows_sys::Win32::Foundation::RtlNtStatusToDosError;
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE, FILE_WRITE_DATA, SYNCHRONIZE,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

    let metadata = from.parent.symlink_metadata(&from.leaf)?;
    if metadata.is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capability rename source is not a real file or directory",
        )
        .into());
    }
    let mut options = OpenOptions::new();
    options
        .access_mode(DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .follow(FollowSymlinks::No);
    let source = from
        .parent
        .open_with(&from.leaf, &options)
        .map_err(|error| MutationError::Io(format!("atomic rename source open failed: {error}")))?;
    let mut destination_options = OpenOptions::new();
    destination_options
        .access_mode(FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .follow(FollowSymlinks::No);
    let destination_parent = to
        .parent
        .open_with(".", &destination_options)
        .map_err(|error| {
            MutationError::Io(format!("atomic rename destination open failed: {error}"))
        })?;
    let from_metadata = from.parent.dir_metadata()?;
    let to_metadata = to.parent.dir_metadata()?;
    let destination_sync =
        if from_metadata.dev() == to_metadata.dev() && from_metadata.ino() == to_metadata.ino() {
            None
        } else {
            let mut destination_sync_options = OpenOptions::new();
            destination_sync_options
                .access_mode(FILE_WRITE_DATA | SYNCHRONIZE)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .follow(FollowSymlinks::No);
            Some(to.parent.open_with(".", &destination_sync_options)?)
        };
    let destination_name = to.leaf.encode_wide().collect::<Vec<_>>();
    let name_bytes = destination_name
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|length| u32::try_from(length).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "rename name is too long"))?;
    let buffer_bytes = size_of::<FILE_RENAME_INFORMATION>()
        .checked_add(name_bytes as usize)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "rename buffer overflow"))?;
    let mut buffer = vec![0_u64; buffer_bytes.div_ceil(size_of::<u64>())];
    let information = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    unsafe {
        (*information).Anonymous.ReplaceIfExists = replace_if_exists;
        (*information).RootDirectory = destination_parent.as_raw_handle();
        (*information).FileNameLength = name_bytes;
        std::ptr::copy_nonoverlapping(
            destination_name.as_ptr(),
            std::ptr::addr_of_mut!((*information).FileName).cast::<u16>(),
            destination_name.len(),
        );
    }

    drop(to.parent);
    hook();
    retry_windows_replace(
        replace_if_exists,
        std::time::Duration::from_millis(10),
        || {
            let mut io_status = IO_STATUS_BLOCK::default();
            let status = unsafe {
                NtSetInformationFile(
                    source.as_raw_handle(),
                    &mut io_status,
                    information.cast_const().cast(),
                    u32::try_from(buffer_bytes).map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "rename buffer is too large")
                    })?,
                    FileRenameInformation,
                )
            };
            if status >= 0 {
                return Ok(());
            }
            let error =
                io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32);
            Err(io::Error::new(
                error.kind(),
                format!("handle-relative rename failed: {error}"),
            ))
        },
    )?;
    sync_dir(&from.parent)?;
    if let Some(destination_sync) = destination_sync {
        destination_sync.sync_all().map_err(|error| {
            MutationError::Io(format!(
                "destination directory durability flush failed: {error}"
            ))
        })?;
    }
    Ok(())
}

#[cfg(windows)]
fn retry_windows_replace(
    replace_if_exists: bool,
    mut backoff: std::time::Duration,
    mut operation: impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    const ATTEMPTS: u32 = 8;
    const SHARING_VIOLATION: i32 = 32;
    let mut last_error = None;
    for attempt in 0..ATTEMPTS {
        match operation() {
            Ok(()) => return Ok(()),
            Err(error)
                if replace_if_exists
                    && (error.kind() == io::ErrorKind::PermissionDenied
                        || error.raw_os_error() == Some(SHARING_VIOLATION)) =>
            {
                last_error = Some(error);
                if attempt + 1 < ATTEMPTS {
                    std::thread::sleep(backoff);
                    backoff *= 2;
                }
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.expect("bounded Windows rename retry records every transient error"))
}

impl AnchoredTarget {
    fn open_regular(&self) -> Result<Option<cap_std::fs::File>, MutationError> {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        match self.parent.open_with(&self.leaf, &options) {
            Ok(file) => {
                if !file.metadata()?.is_file() {
                    return Err(MutationError::Invalid(
                        "capability target is not a regular file".into(),
                    ));
                }
                Ok(Some(file))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn open_lock_file(&self) -> Result<std::fs::File, MutationError> {
        let mut create = OpenOptions::new();
        create
            .create_new(true)
            .read(true)
            .write(true)
            .follow(FollowSymlinks::No);
        configure_lock_sharing(&mut create);
        match self.parent.open_with(&self.leaf, &create) {
            Ok(file) => {
                file.sync_all()?;
                sync_dir(&self.parent)?;
                Ok(file.into_std())
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let mut existing = OpenOptions::new();
                existing.read(true).write(true).follow(FollowSymlinks::No);
                configure_lock_sharing(&mut existing);
                let file = self.parent.open_with(&self.leaf, &existing)?;
                if !file.metadata()?.is_file() {
                    return Err(MutationError::Invalid(
                        "capability lock target is not a regular file".into(),
                    ));
                }
                Ok(file.into_std())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn read_bounded(&self, limit: usize) -> Result<Option<Vec<u8>>, MutationError> {
        self.read_bounded_inner(limit, || {})
    }

    #[cfg(test)]
    pub(crate) fn read_bounded_with_hook(
        &self,
        limit: usize,
        hook: impl FnOnce(),
    ) -> Result<Option<Vec<u8>>, MutationError> {
        self.read_bounded_inner(limit, hook)
    }

    fn read_bounded_inner(
        &self,
        limit: usize,
        hook: impl FnOnce(),
    ) -> Result<Option<Vec<u8>>, MutationError> {
        let Some(mut file) = self.open_regular()? else {
            return Ok(None);
        };
        hook();
        let take_limit = u64::try_from(limit)
            .map_err(|_| MutationError::Invalid("bounded read limit overflow".into()))?
            .checked_add(1)
            .ok_or_else(|| MutationError::Invalid("bounded read limit overflow".into()))?;
        let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
        Read::by_ref(&mut file)
            .take(take_limit)
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(MutationError::Invalid(format!(
                "capability target exceeds its {limit}-byte limit"
            )));
        }
        Ok(Some(bytes))
    }

    pub(crate) fn put_atomic(&self, bytes: &[u8]) -> Result<(), MutationError> {
        let temporary = OsString::from(format!(".{}.tmp", Uuid::new_v4()));
        self.put_atomic_with_temporary(temporary, bytes)
    }

    fn put_atomic_with_temporary(
        &self,
        temporary: OsString,
        bytes: &[u8],
    ) -> Result<(), MutationError> {
        let temporary_target = Self {
            parent: self.parent.try_clone()?,
            leaf: temporary.clone(),
        };
        temporary_target.put_new(bytes).map_err(|error| {
            MutationError::RecoveryConflict(format!(
                "capability atomic temporary publication failed: {error}"
            ))
        })?;
        let result = rename_target_replace(
            temporary_target,
            Self {
                parent: self.parent.try_clone()?,
                leaf: self.leaf.clone(),
            },
        );
        if result.is_err() {
            let _ = self.parent.remove_file_or_symlink(&temporary);
        }
        result?;
        sync_dir(&self.parent)
    }

    fn put_new(&self, bytes: &[u8]) -> Result<(), MutationError> {
        let mut options = OpenOptions::new();
        options
            .create_new(true)
            .write(true)
            .follow(FollowSymlinks::No);
        let mut file = self.parent.open_with(&self.leaf, &options)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        sync_dir(&self.parent)
    }

    pub(crate) fn delete(&self) -> Result<(), MutationError> {
        match self.parent.symlink_metadata(&self.leaf) {
            Ok(metadata) if metadata.is_file() || metadata.is_symlink() => {
                self.parent.remove_file_or_symlink(&self.leaf)?;
                sync_dir(&self.parent)
            }
            Ok(_) => Err(MutationError::Invalid(
                "capability target is not a regular file or symlink".into(),
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => sync_dir(&self.parent),
            Err(error) => Err(error.into()),
        }
    }
}

fn same_file(left: &std::fs::File, right: &cap_std::fs::File) -> Result<bool, MutationError> {
    let left = cap_std::fs::File::from_std(left.try_clone()?);
    same_cap_file(&left, right)
}

fn same_cap_file(
    left: &cap_std::fs::File,
    right: &cap_std::fs::File,
) -> Result<bool, MutationError> {
    Ok(cap_file_identity(left)? == cap_file_identity(right)?)
}

fn cap_file_identity(file: &cap_std::fs::File) -> Result<RegularFileIdentity, MutationError> {
    let metadata = file.metadata()?;
    Ok(RegularFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

fn random_sibling_key(relative_key: &str) -> Result<String, MutationError> {
    validate_relative_key(relative_key)?;
    let temporary = format!(".{}.delete", Uuid::new_v4());
    Ok(match relative_key.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{temporary}"),
        None => temporary,
    })
}

#[cfg(windows)]
fn configure_lock_sharing(options: &mut OpenOptions) {
    use cap_std::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
}

#[cfg(not(windows))]
fn configure_lock_sharing(_options: &mut OpenOptions) {}

fn sync_dir(directory: &Dir) -> Result<(), MutationError> {
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .follow(FollowSymlinks::No);
        directory
            .open_with(".", &options)
            .map_err(|error| {
                MutationError::Io(format!("directory durability open failed: {error}"))
            })?
            .sync_all()
            .map_err(|error| {
                MutationError::Io(format!("directory durability flush failed: {error}"))
            })?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        directory.try_clone()?.into_std_file().sync_all()?;
        Ok(())
    }
}

#[cfg(unix)]
fn rename_replace_with_retry(
    from_dir: &Dir,
    from: &OsStr,
    to_dir: &Dir,
    to: &OsStr,
) -> io::Result<()> {
    const ATTEMPTS: u32 = 5;
    const SHARING_VIOLATION: i32 = 32;
    let mut backoff = std::time::Duration::from_millis(10);
    let mut last_error = None;
    for attempt in 0..ATTEMPTS {
        match from_dir.rename(from, to_dir, to) {
            Ok(()) => return Ok(()),
            Err(error)
                if error.kind() == io::ErrorKind::PermissionDenied
                    || error.raw_os_error() == Some(SHARING_VIOLATION) =>
            {
                last_error = Some(error);
                if attempt + 1 < ATTEMPTS {
                    std::thread::sleep(backoff);
                    backoff *= 2;
                }
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.expect("bounded rename retry records every transient error"))
}

#[cfg(windows)]
fn open_external_windows_directory(path: &Path) -> Result<Dir, MutationError> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt as _};
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFinalPathNameByHandleW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FILE_TRAVERSE, SYNCHRONIZE, VOLUME_NAME_DOS,
    };

    let mut anchor = PathBuf::new();
    let mut descendants = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => anchor.push(prefix.as_os_str()),
            std::path::Component::RootDir => anchor.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                return Err(MutationError::Invalid(
                    "external atomic directory cannot contain parent traversal".into(),
                ))
            }
            std::path::Component::Normal(name) => descendants.push(name.to_os_string()),
        }
    }
    if anchor.as_os_str().is_empty() {
        return Err(MutationError::Invalid(
            "external atomic directory must resolve from a filesystem root".into(),
        ));
    }
    let mut current = anchor;
    let mut pinned_ancestors = Vec::new();
    for descendant in descendants {
        current.push(&descendant);
        let metadata = std::fs::symlink_metadata(&current)?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(MutationError::Invalid(format!(
                "external atomic directory contains a reparse point at {}",
                descendant.to_string_lossy()
            )));
        }
        let opened = std::fs::OpenOptions::new()
            .access_mode(FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&current);
        match opened {
            Ok(file) => pinned_ancestors.push(file),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                // Some Windows profile ancestors deny opening the directory itself
                // while permitting traversal. The final retained handle path check
                // below still proves that no unchecked ancestor redirected it.
            }
            Err(error) => return Err(error.into()),
        }
    }
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&current)
        .map_err(|error| {
            MutationError::Io(format!("external parent no-follow open failed: {error}"))
        })?;
    let metadata = file.metadata()?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(MutationError::Invalid(
            "external atomic parent must be a real directory".into(),
        ));
    }
    let required = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
        )
    };
    if required == 0 {
        return Err(io::Error::last_os_error().into());
    }
    let mut buffer = vec![0_u16; required as usize + 1];
    let written = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
        )
    };
    if written == 0 || written as usize >= buffer.len() {
        return Err(io::Error::last_os_error().into());
    }
    let retained = String::from_utf16(&buffer[..written as usize])
        .map_err(|_| MutationError::Invalid("external retained path is invalid UTF-16".into()))?;
    if normalize_windows_retained_path(&retained)
        != normalize_windows_retained_path(&current.to_string_lossy())
    {
        return Err(MutationError::RecoveryConflict(
            "external atomic parent was redirected during capability acquisition".into(),
        ));
    }
    drop(pinned_ancestors);
    Ok(Dir::from_std_file(file))
}

#[cfg(windows)]
fn normalize_windows_retained_path(path: &str) -> String {
    let path = path.replace('/', "\\");
    let path = path
        .strip_prefix(r"\\?\UNC\")
        .map(|path| format!(r"\\{path}"))
        .or_else(|| path.strip_prefix(r"\\?\").map(ToOwned::to_owned))
        .unwrap_or(path);
    path.trim_end_matches('\\').to_lowercase()
}

#[cfg(not(windows))]
fn reopen_external_dir_with_identity(path: &Path, expected: &Dir) -> Result<Dir, MutationError> {
    let reopened = Dir::open_ambient_dir(path, ambient_authority())
        .map_err(|error| MutationError::Io(format!("external parent reopen failed: {error}")))?;
    let expected_metadata = expected.dir_metadata().map_err(|error| {
        MutationError::Io(format!(
            "external traversed parent metadata failed: {error}"
        ))
    })?;
    let reopened_metadata = reopened.dir_metadata().map_err(|error| {
        MutationError::Io(format!("external reopened parent metadata failed: {error}"))
    })?;
    if expected_metadata.dev() != reopened_metadata.dev()
        || expected_metadata.ino() != reopened_metadata.ino()
    {
        return Err(MutationError::RecoveryConflict(
            "external atomic parent changed during capability acquisition".into(),
        ));
    }
    Ok(reopened)
}

fn leaf_name_is_relative(leaf: &OsStr) -> bool {
    let mut components = Path::new(leaf).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

#[cfg(all(test, windows))]
mod external_windows_path_tests {
    use super::{normalize_windows_retained_path, retry_windows_replace};

    #[test]
    fn retained_dos_and_unc_paths_normalize_to_their_lexical_forms() {
        assert_eq!(
            normalize_windows_retained_path(r"\\?\C:\Users\Bryan\Preview\\"),
            r"c:\users\bryan\preview"
        );
        assert_eq!(
            normalize_windows_retained_path(r"\\?\UNC\Server\Share\Preview"),
            r"\\server\share\preview"
        );
    }

    #[test]
    fn transient_replace_retries_are_bounded_and_reach_the_eighth_attempt() {
        let mut eventual_attempts = 0;
        retry_windows_replace(true, std::time::Duration::ZERO, || {
            eventual_attempts += 1;
            if eventual_attempts < 8 {
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(eventual_attempts, 8);

        let mut exhausted_attempts = 0;
        let error = retry_windows_replace(true, std::time::Duration::ZERO, || {
            exhausted_attempts += 1;
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        })
        .unwrap_err();
        assert_eq!(exhausted_attempts, 8);
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }
}
