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

    pub(crate) fn install_no_clobber(
        &self,
        destination: &str,
        staging_directory: &str,
        bytes: &[u8],
    ) -> Result<(), MutationError> {
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
                    Ok(())
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    sync_dir(&destination.parent)?;
                    Ok(())
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
            names.push(name.to_string());
        }
        names.sort();
        Ok(names)
    }

    pub(crate) fn directory_entries(
        &self,
        relative_directory: &str,
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
        let temporary_target = Self {
            parent: self.parent.try_clone()?,
            leaf: temporary.clone(),
        };
        temporary_target.put_new(bytes)?;
        let result = self
            .parent
            .rename(&temporary, &self.parent, &self.leaf)
            .map_err(MutationError::from);
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
    let left = left.metadata()?;
    let right = right.metadata()?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
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
        directory.open_with(".", &options)?.sync_all()?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        directory.try_clone()?.into_std_file().sync_all()?;
        Ok(())
    }
}

#[allow(dead_code)]
fn _leaf_name_is_relative(leaf: &OsStr) -> bool {
    Path::new(leaf).components().count() == 1
}
