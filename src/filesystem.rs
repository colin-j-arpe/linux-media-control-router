//! Shared filesystem primitives for durable local files; callers own their locks.
use std::{
    fs::{self, File},
    io,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

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
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
