//! Opt-in, per-user XDG autostart entry management. Never starts a process.
use crate::filesystem::{TemporaryFile, create_private_directory, remove_file_if_exists};
use gio::glib::{KeyFile, KeyFileFlags};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

const ENTRY: &str = "media-router.desktop";
const GROUP: &str = "Desktop Entry";
const MANAGED: &str = "X-MediaRouter-Managed";

pub fn directory() -> io::Result<PathBuf> {
    Ok(crate::config::directory()?.with_file_name("autostart"))
}

/// Register an explicitly chosen installed executable, not the calling build.
pub fn install(directory: &Path, executable: &Path) -> io::Result<PathBuf> {
    let contents = entry(executable)?;
    create_private_directory(directory)?;
    let _lock = lock(directory)?;
    let path = directory.join(ENTRY);
    check_managed(&path)?;
    let temporary = directory.join(".media-router.desktop.tmp");
    remove_file_if_exists(&temporary)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    let cleanup = TemporaryFile(temporary);
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    fs::rename(&cleanup.0, &path)?;
    File::open(directory)?.sync_all().map_err(|error| io::Error::other(format!(
        "autostart entry was installed, but directory durability could not be confirmed: {error}"
    )))?;
    Ok(path)
}

/// Remove only this tool's personal entry. Running daemons and preferences stay intact.
pub fn remove(directory: &Path) -> io::Result<bool> {
    match fs::metadata(directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
        Ok(_) => (),
    }
    let _lock = lock(directory)?;
    let path = directory.join(ENTRY);
    if !check_managed(&path)? {
        return Ok(false);
    }
    fs::remove_file(path)?;
    File::open(directory)?.sync_all().map_err(|error| {
        io::Error::other(format!(
            "autostart entry was removed, but directory durability could not be confirmed: {error}"
        ))
    })?;
    Ok(true)
}

fn lock(directory: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(directory.join(".media-router.lock"))?;
    file.try_lock().map_err(|error| {
        io::Error::other(format!(
            "another autostart operation holds the lock: {error}"
        ))
    })?;
    Ok(file)
}

fn check_managed(path: &Path) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
        Ok(metadata) => metadata,
    };
    if metadata.is_file() {
        let data = fs::read_to_string(path)?;
        let keyfile = KeyFile::new();
        if keyfile.load_from_data(&data, KeyFileFlags::NONE).is_ok()
            && keyfile.boolean(GROUP, MANAGED).unwrap_or(false)
        {
            return Ok(true);
        }
    }
    Err(io::Error::other(format!(
        "refusing to replace or remove an unrecognized autostart entry: {}; move it aside or manage it manually",
        path.display()
    )))
}

fn entry(executable: &Path) -> io::Result<String> {
    let invalid = |message| io::Error::new(io::ErrorKind::InvalidInput, message);
    if !executable.is_absolute() {
        return Err(invalid("autostart requires an absolute executable path"));
    }
    let path = executable
        .to_str()
        .ok_or_else(|| invalid("the desktop entry requires a UTF-8 executable path"))?;
    if path.chars().any(|c| c.is_control() || c == '=') {
        return Err(invalid(
            "the executable path must not contain control characters or '='",
        ));
    }
    let metadata = fs::metadata(executable)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(invalid("the autostart target must be an executable file"));
    }
    // Desktop Exec quoting is not shell execution. First quote the argument;
    // KeyFile then handles the separate desktop-string escaping layer.
    let mut quoted = String::from("\"");
    for character in path.chars() {
        match character {
            '%' => quoted.push_str("%%"),
            '"' | '`' | '$' | '\\' => {
                quoted.push('\\');
                quoted.push(character);
            }
            _ => quoted.push(character),
        }
    }
    quoted.push_str("\" --serve");
    let keyfile = KeyFile::new();
    keyfile.set_string(GROUP, "Type", "Application");
    keyfile.set_string(GROUP, "Name", "Media Router");
    keyfile.set_string(
        GROUP,
        "Comment",
        "Route media transport keys to the selected application",
    );
    keyfile.set_string(GROUP, "Exec", &quoted);
    keyfile.set_string(GROUP, "TryExec", path);
    keyfile.set_string(GROUP, "OnlyShowIn", "X-Cinnamon;");
    keyfile.set_boolean(GROUP, "Terminal", false);
    keyfile.set_boolean(GROUP, MANAGED, true);
    Ok(keyfile.to_data().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "media-router-autostart-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn executable(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, b"test executable; never launched").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            path
        }
        fn autostart(&self) -> PathBuf {
            self.0.join("autostart")
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn entry_quotes_paths_and_sets_cinnamon_and_serving_fields() {
        let directory = Directory::new();
        let executable = directory.executable("Media 音楽 'quoted' \" $`\\%file");
        let keyfile = KeyFile::new();
        keyfile
            .load_from_data(&entry(&executable).unwrap(), KeyFileFlags::NONE)
            .unwrap();
        let exec = keyfile.string(GROUP, "Exec").unwrap();
        // Native GLib parsing checks the command quoting after KeyFile decoding.
        let args = gio::glib::shell_parse_argv(exec.replace("%%", "%")).unwrap();
        assert_eq!(
            args,
            [executable.as_os_str(), std::ffi::OsStr::new("--serve")]
        );
        assert_eq!(
            keyfile.string(GROUP, "TryExec").unwrap().as_str(),
            executable.to_str().unwrap()
        );
        assert_eq!(
            keyfile.string(GROUP, "OnlyShowIn").unwrap().as_str(),
            "X-Cinnamon;"
        );
        assert!(!keyfile.boolean(GROUP, "Terminal").unwrap());
    }

    #[test]
    fn installs_replaces_and_removes_only_managed_entries() {
        let directory = Directory::new();
        let executable = directory.executable("media-router");
        assert!(!remove(&directory.autostart()).unwrap());
        assert!(!directory.autostart().exists());
        let path = install(&directory.autostart(), &executable).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            entry(&executable).unwrap()
        );
        let new_executable = directory.executable("new-router");
        install(&directory.autostart(), &new_executable).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            entry(&new_executable).unwrap()
        );
        assert!(remove(&directory.autostart()).unwrap());
        assert!(!remove(&directory.autostart()).unwrap());
        fs::write(&path, b"[Desktop Entry]\nType=Application\nName=Other\n").unwrap();
        let before = fs::read(&path).unwrap();
        assert!(install(&directory.autostart(), &executable).is_err());
        assert!(remove(&directory.autostart()).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn rejects_invalid_targets_before_creating_autostart_files() {
        let directory = Directory::new();
        let plain = directory.executable("not-executable");
        fs::set_permissions(&plain, fs::Permissions::from_mode(0o600)).unwrap();
        for path in [
            PathBuf::from("relative"),
            directory.0.join("missing"),
            directory.0.clone(),
            plain,
            directory.executable("bad=name"),
            directory.executable("bad\nname"),
        ] {
            assert!(
                install(&directory.autostart(), &path).is_err(),
                "accepted {path:?}"
            );
            assert!(!directory.autostart().exists());
        }
    }

    #[test]
    fn lock_and_temporary_failure_preserve_installed_entry() {
        let directory = Directory::new();
        let executable = directory.executable("media-router");
        let path = install(&directory.autostart(), &executable).unwrap();
        let before = fs::read(&path).unwrap();
        let held = lock(&directory.autostart()).unwrap();
        assert!(install(&directory.autostart(), &executable).is_err());
        assert!(remove(&directory.autostart()).is_err());
        drop(held);
        fs::create_dir(directory.autostart().join(".media-router.desktop.tmp")).unwrap();
        assert!(install(&directory.autostart(), &executable).is_err());
        assert_eq!(fs::read(path).unwrap(), before);
    }

    #[test]
    fn refuses_symlink_entries_without_touching_their_targets() {
        let directory = Directory::new();
        let executable = directory.executable("media-router");
        fs::create_dir(directory.autostart()).unwrap();
        let target = directory.0.join("other.desktop");
        fs::write(&target, entry(&executable).unwrap()).unwrap();
        std::os::unix::fs::symlink(&target, directory.autostart().join(ENTRY)).unwrap();
        assert!(install(&directory.autostart(), &executable).is_err());
        assert!(remove(&directory.autostart()).is_err());
        assert!(target.exists());
    }
}
