//! Durable user preferences. Runtime inventories and capture recovery belong elsewhere.
//!
//! A store owns its file lock until dropped. Callers must save user intent before
//! applying it to runtime state; capture itself is a separate external operation.
use crate::filesystem::{TemporaryFile, create_private_directory, remove_file_if_exists};
use crate::player::selection::ApplicationId;
use serde::{Deserialize, Serialize};
use std::{
    env, fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

/// Versioned on-disk format. Only logical identities are durable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub selected: Option<ApplicationId>,
    pub capture_enabled: bool,
    pub auto_select_new: bool,
    pub exclusions: Vec<ApplicationId>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            selected: None,
            capture_enabled: false,
            auto_select_new: false,
            exclusions: Vec::new(),
        }
    }
}

impl Preferences {
    fn validate(&self) -> io::Result<()> {
        if self.version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported configuration version: {}", self.version),
            ));
        }
        for id in self.selected.iter().chain(&self.exclusions) {
            if matches!(id, ApplicationId::DesktopEntry(value) if value.is_empty()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "desktop-entry identifiers must not be empty",
                ));
            }
        }
        Ok(())
    }
}

/// Resolve XDG configuration without creating files or touching desktop settings.
pub fn directory() -> io::Result<PathBuf> {
    configuration_directory(
        env::var_os("XDG_CONFIG_HOME").as_deref().map(Path::new),
        env::var_os("HOME").as_deref().map(Path::new),
    )
}

fn configuration_directory(xdg: Option<&Path>, home: Option<&Path>) -> io::Result<PathBuf> {
    // XDG paths must be absolute; ignore empty or relative environment values.
    let base = match xdg.filter(|path| path.is_absolute()) {
        Some(path) => path.to_path_buf(),
        None => home
            .filter(|path| path.is_absolute())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "configuration requires an absolute XDG_CONFIG_HOME or HOME",
                )
            })?
            .join(".config"),
    };
    Ok(base.join("media-router"))
}

/// A failed directory sync after rename is different from a failed replacement:
/// the new preferences are visible, but their survival across a crash is uncertain.
#[derive(Debug)]
pub enum SaveError {
    NotSaved(io::Error),
    DurabilityUncertain(io::Error),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSaved(error) => write!(f, "configuration was not saved: {error}"),
            Self::DurabilityUncertain(error) => write!(
                f,
                "configuration was replaced, but its durability could not be confirmed: {error}"
            ),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::NotSaved(error) | Self::DurabilityUncertain(error) => error,
        })
    }
}

/// Single writer for one configuration directory. Do not delete the lock file:
/// processes must keep locking the same inode, even across configuration renames.
pub struct Store {
    directory: PathBuf,
    preferences: Preferences,
    _lock: File,
}

impl Store {
    pub fn open(directory: PathBuf) -> io::Result<Self> {
        create_private_directory(&directory)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(directory.join("config.lock"))?;
        lock.try_lock().map_err(|error| {
            io::Error::other(format!(
                "could not acquire configuration writer lock: {error}"
            ))
        })?;
        let path = directory.join("config.json");
        let preferences = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Preferences>(&bytes).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid configuration {}: {error}", path.display()),
                )
            })?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Preferences::default(),
            Err(error) => return Err(error),
        };
        preferences.validate()?;
        Ok(Self {
            directory,
            preferences,
            _lock: lock,
        })
    }

    pub fn preferences(&self) -> &Preferences {
        &self.preferences
    }

    /// Save before acknowledging a preference change. On DurabilityUncertain,
    /// preferences() reflects the replacement, and callers must report the error.
    pub fn save(&mut self, preferences: Preferences) -> Result<(), SaveError> {
        self.save_with_directory_sync(preferences, |path| File::open(path)?.sync_all())
    }

    fn save_with_directory_sync(
        &mut self,
        preferences: Preferences,
        sync_directory: impl FnOnce(&Path) -> io::Result<()>,
    ) -> Result<(), SaveError> {
        let replace = || -> io::Result<()> {
            preferences.validate()?;
            let mut bytes = serde_json::to_vec_pretty(&preferences)?;
            bytes.push(b'\n');
            let temporary = self.directory.join("config.tmp");
            // The lock excludes other writers; an existing temporary file is
            // an incomplete previous save, never a configuration to load.
            remove_file_if_exists(&temporary)?;
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&temporary)?;
            let cleanup = TemporaryFile(temporary);
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&cleanup.0, self.directory.join("config.json"))?;
            Ok(())
        };
        replace().map_err(SaveError::NotSaved)?;
        self.preferences = preferences;
        sync_directory(&self.directory).map_err(SaveError::DurabilityUncertain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::PermissionsExt,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = env::temp_dir().join(format!(
                "media-router-config-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn store_path(&self) -> PathBuf {
            self.0.join("media-router")
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn xdg_paths_use_absolute_values_and_do_not_require_unicode() {
        assert_eq!(
            configuration_directory(Some(Path::new("/config")), None).unwrap(),
            Path::new("/config/media-router")
        );
        for invalid in [None, Some(Path::new("")), Some(Path::new("relative"))] {
            assert_eq!(
                configuration_directory(invalid, Some(Path::new("/home/user"))).unwrap(),
                Path::new("/home/user/.config/media-router")
            );
            assert!(configuration_directory(invalid, Some(Path::new("relative"))).is_err());
        }
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new(std::ffi::OsStr::from_bytes(b"/config/\xff"));
        assert_eq!(
            configuration_directory(Some(path), None).unwrap(),
            path.join("media-router")
        );
    }

    #[test]
    fn missing_configuration_defaults_without_writing_a_configuration_file() {
        let directory = Directory::new();
        let store = Store::open(directory.store_path()).unwrap();
        assert_eq!(store.preferences(), &Preferences::default());
        assert!(!directory.store_path().join("config.json").exists());
        assert_eq!(
            fs::metadata(directory.store_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn preferences_round_trip_exact_identities_and_clear_selection() {
        let directory = Directory::new();
        let mut store = Store::open(directory.store_path()).unwrap();
        for selected in [
            Some(ApplicationId::DesktopEntry("spotify".into())),
            Some(ApplicationId::Identity(" 日本語 Player \n".into())),
            Some(ApplicationId::Identity(String::new())),
            None,
        ] {
            let preferences = Preferences {
                selected,
                capture_enabled: true,
                auto_select_new: true,
                exclusions: vec![
                    ApplicationId::DesktopEntry("browser".into()),
                    ApplicationId::Identity("browser".into()),
                ],
                ..Preferences::default()
            };
            store.save(preferences.clone()).unwrap();
            drop(store);
            store = Store::open(directory.store_path()).unwrap();
            assert_eq!(store.preferences(), &preferences);
        }
        let path = directory.store_path().join("config.json");
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn malformed_or_unsupported_files_are_preserved_and_release_the_lock() {
        let directory = Directory::new();
        fs::create_dir(directory.store_path()).unwrap();
        let good = serde_json::to_value(Preferences::default()).unwrap();
        let mut invalid = vec!["{".into(), "{}".into()];
        for (key, value) in [
            ("version", serde_json::json!(2)),
            ("mrp_enabled", serde_json::json!(true)),
            (
                "selected",
                serde_json::json!({"kind":"desktop-entry", "value":""}),
            ),
            (
                "selected",
                serde_json::json!({"kind":"unknown", "value":"test"}),
            ),
            (
                "selected",
                serde_json::json!({"kind":"identity", "value":"test", "owner":":1.1"}),
            ),
            (
                "exclusions",
                serde_json::json!([{"kind":"desktop-entry", "value":""}]),
            ),
        ] {
            let mut document = good.clone();
            document[key] = value;
            invalid.push(document.to_string());
        }
        let path = directory.store_path().join("config.json");
        for bytes in invalid {
            fs::write(&path, &bytes).unwrap();
            assert!(
                Store::open(directory.store_path()).is_err(),
                "accepted {bytes}"
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
        }
        fs::write(path, good.to_string()).unwrap();
        assert!(Store::open(directory.store_path()).is_ok());
    }

    #[test]
    fn writer_lock_survives_replacement_and_is_released_on_drop() {
        let directory = Directory::new();
        let mut store = Store::open(directory.store_path()).unwrap();
        assert!(Store::open(directory.store_path()).is_err());
        store.save(Preferences::default()).unwrap();
        assert!(Store::open(directory.store_path()).is_err());
        drop(store);
        assert!(Store::open(directory.store_path()).is_ok());
    }

    #[test]
    fn stale_temporary_files_are_ignored_and_replaced_on_the_next_save() {
        let directory = Directory::new();
        let mut store = Store::open(directory.store_path()).unwrap();
        store.save(Preferences::default()).unwrap();
        drop(store);
        let temporary = directory.store_path().join("config.tmp");
        fs::write(&temporary, b"{interrupted write").unwrap();
        let mut store = Store::open(directory.store_path()).unwrap();
        assert_eq!(store.preferences(), &Preferences::default());
        let next = Preferences {
            capture_enabled: true,
            ..Preferences::default()
        };
        store.save(next.clone()).unwrap();
        assert!(!temporary.exists());
        drop(store);
        assert_eq!(
            Store::open(directory.store_path()).unwrap().preferences(),
            &next
        );
    }

    #[test]
    fn failures_before_replacement_preserve_file_and_memory() {
        let directory = Directory::new();
        let mut store = Store::open(directory.store_path()).unwrap();
        store.save(Preferences::default()).unwrap();
        let before = fs::read(directory.store_path().join("config.json")).unwrap();
        let bad = Preferences {
            selected: Some(ApplicationId::DesktopEntry(String::new())),
            ..Preferences::default()
        };
        assert!(matches!(store.save(bad), Err(SaveError::NotSaved(_))));
        // A directory at the temporary-file path gives a deterministic filesystem
        // failure, including when tests are run with elevated permissions.
        fs::create_dir(directory.store_path().join("config.tmp")).unwrap();
        let next = Preferences {
            capture_enabled: true,
            ..Preferences::default()
        };
        assert!(matches!(store.save(next), Err(SaveError::NotSaved(_))));
        assert_eq!(
            fs::read(directory.store_path().join("config.json")).unwrap(),
            before
        );
        assert_eq!(store.preferences(), &Preferences::default());
    }

    #[test]
    fn failed_directory_sync_reports_visible_replacement_and_allows_retry() {
        let directory = Directory::new();
        let mut store = Store::open(directory.store_path()).unwrap();
        store.save(Preferences::default()).unwrap();
        let next = Preferences {
            capture_enabled: true,
            ..Preferences::default()
        };
        let error = store.save_with_directory_sync(next.clone(), |_| {
            Err(io::Error::other("injected directory sync failure"))
        });
        assert!(matches!(error, Err(SaveError::DurabilityUncertain(_))));
        assert_eq!(store.preferences(), &next);
        let on_disk: Preferences =
            serde_json::from_slice(&fs::read(directory.store_path().join("config.json")).unwrap())
                .unwrap();
        assert_eq!(on_disk, next);
        store.save(next).unwrap();
    }
}
