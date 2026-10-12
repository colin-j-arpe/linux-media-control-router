//! Shared filesystem primitives for durable local files; callers own their locks.
use std::{
    fs::{self, File},
    io,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

/// Creates a directory tree with private permissions for new directories.
///
/// # Arguments
///
/// * `directory` — Directory tree to create; existing permissions are preserved.
///
/// # Returns
///
/// Returns Ok(()) after creating the tree and synchronizing its parent directories.
///
/// # Errors
///
/// Returns a filesystem error if creation or parent synchronization fails; directories may already
/// exist when synchronization fails.
pub(crate) fn create_private_directory(directory: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    // Sync parent entries too, including any newly created ancestors.
    for parent in directory
        .ancestors()
        .skip(1)
        .filter(|p| !p.as_os_str().is_empty())
    {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// Removes a file while treating an absent path as success.
///
/// # Arguments
///
/// * `path` — File or symlink to remove.
///
/// # Returns
///
/// Returns Ok(()) if removal succeeds or the path is absent.
///
/// # Errors
///
/// Returns any removal error other than NotFound.
pub(crate) fn remove_file_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Only construct after successfully creating the temporary file under a lock.
pub(crate) struct TemporaryFile(pub PathBuf);
impl Drop for TemporaryFile {
    /// Attempts to remove the temporary file when its guard is dropped.
    ///
    /// Cleanup failures are ignored; the caller must construct the guard only after creating the
    /// file under a lock.
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
