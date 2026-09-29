use super::{Error, Result};
use gio::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

pub(crate) const BINDINGS: [(&str, &str); 5] = [
    ("play", "XF86AudioPlay"),
    ("pause", "XF86AudioPause"),
    ("stop", "XF86AudioStop"),
    ("previous", "XF86AudioPrev"),
    ("next", "XF86AudioNext"),
];
const SCHEMA: &str = "org.cinnamon.desktop.keybindings.media-keys";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Value {
    effective: Vec<String>,
    user: Option<Vec<String>>,
}

pub(crate) trait Settings {
    fn read(&self, key: &str) -> Result<Value>;
    fn write(&self, key: &str, value: Option<&[String]>) -> Result<()>;
}

pub(crate) struct Cinnamon(gio::Settings);
impl Cinnamon {
    pub fn new() -> Result<Self> {
        let schema = gio::SettingsSchemaSource::default()
            .and_then(|source| source.lookup(SCHEMA, true))
            .ok_or("Cinnamon media-key settings schema is not installed")?;
        for (key, _) in BINDINGS {
            if !schema.has_key(key) || schema.key(key).value_type().as_str() != "as" {
                return Err(format!("missing or incompatible Cinnamon setting: {key}").into());
            }
        }
        Ok(Self(gio::Settings::new_full(
            &schema,
            None::<&gio::SettingsBackend>,
            None,
        )))
    }
}
fn pump() {
    let context = gio::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}
impl Settings for Cinnamon {
    fn read(&self, key: &str) -> Result<Value> {
        pump();
        Ok(Value {
            effective: self.0.strv(key).iter().map(|s| s.to_string()).collect(),
            user: self
                .0
                .user_value(key)
                .map(|v| v.get::<Vec<String>>().ok_or("invalid shortcut array"))
                .transpose()?,
        })
    }
    fn write(&self, key: &str, value: Option<&[String]>) -> Result<()> {
        if !self.0.is_writable(key) {
            return Err(format!("Cinnamon setting is not writable: {key}").into());
        }
        match value {
            Some(value) => self.0.set_strv(key, value)?,
            None => self.0.reset(key),
        }
        gio::Settings::sync();
        pump();
        if self.read(key)?.user.as_deref() != value {
            return Err(format!("could not verify Cinnamon setting write: {key}").into());
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    key: String,
    original: Value,
    temporary: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    entries: Vec<Entry>,
}

/// Owns the lock for the entire settings transaction, including recovery.
/// The journal is durable before the first desktop write; Drop is only a fallback.
pub(crate) struct Lease<S: Settings> {
    settings: S,
    directory: PathBuf,
    _lock: File,
}
impl<S: Settings> Lease<S> {
    pub fn open(settings: S, directory: PathBuf) -> Result<Self> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(directory.join("bindings.lock"))?;
        lock.try_lock().map_err(|e| {
            format!("another capture/recovery process holds the settings lock: {e}")
        })?;
        Ok(Self {
            settings,
            directory,
            _lock: lock,
        })
    }
    fn path(&self) -> PathBuf {
        self.directory.join("bindings.json")
    }
    fn load(&self) -> Result<Option<Journal>> {
        let bytes = match fs::read(self.path()) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let journal: Journal = serde_json::from_slice(&bytes)?;
        if journal.version != 1 || journal.entries.len() > BINDINGS.len() {
            return Err("unsupported recovery journal".into());
        }
        let mut seen = std::collections::HashSet::new();
        for entry in &journal.entries {
            let accelerator = BINDINGS
                .iter()
                .find(|(key, _)| *key == entry.key)
                .ok_or("unknown recovery key")?
                .1;
            let expected = without(&entry.original.effective, accelerator);
            if !seen.insert(&entry.key)
                || expected != entry.temporary
                || expected == entry.original.effective
            {
                return Err("inconsistent recovery journal; refusing settings changes".into());
            }
        }
        Ok(Some(journal))
    }
    pub fn acquire(&self) -> Result<()> {
        self.restore()?;
        let mut entries = Vec::new();
        for (key, accelerator) in BINDINGS {
            let original = self.settings.read(key)?;
            let temporary = without(&original.effective, accelerator);
            if temporary != original.effective {
                entries.push(Entry {
                    key: key.into(),
                    original,
                    temporary,
                });
            }
        }
        let journal = Journal {
            version: 1,
            entries,
        };
        self.save(&journal)?;
        for entry in &journal.entries {
            // A user edit between snapshot and write aborts acquisition.
            if self.settings.read(&entry.key)? != entry.original {
                return Err(
                    format!("setting changed during capture startup: {}", entry.key).into(),
                );
            }
            self.settings.write(&entry.key, Some(&entry.temporary))?;
        }
        Ok(())
    }
    fn save(&self, journal: &Journal) -> Result<()> {
        let temporary = self.directory.join("bindings.tmp");
        // Only a stale, incomplete temporary file may exist while this lock is held.
        match fs::remove_file(&temporary) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(journal)?)?;
        file.sync_all()?;
        fs::rename(temporary, self.path())?;
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
    pub fn unchanged(&self) -> Result<bool> {
        let journal = self.load()?.ok_or("capture recovery journal disappeared")?;
        for entry in journal.entries {
            let current = self.settings.read(&entry.key)?;
            if current.user.as_ref() != Some(&entry.temporary)
                || current.effective != entry.temporary
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub fn restore(&self) -> Result<()> {
        let Some(journal) = self.load()? else {
            return Ok(());
        };
        let mut errors = Vec::new();
        let mut conflicts = false;
        for entry in &journal.entries {
            let result = (|| -> Result<()> {
                let current = self.settings.read(&entry.key)?;
                // Handles recovery interrupted partway through restoration as well.
                if current.user == entry.original.user {
                    return Ok(());
                }
                if current.user.as_ref() == Some(&entry.temporary) {
                    self.settings
                        .write(&entry.key, entry.original.user.as_deref())?;
                } else {
                    conflicts = true;
                    eprintln!(
                        "BINDING PRESERVED: {} was changed externally; keeping the user's value",
                        entry.key
                    );
                }
                Ok(())
            })();
            if let Err(e) = result {
                errors.push(e.to_string());
            }
        }
        if !errors.is_empty() {
            return Err(errors.join("; ").into());
        }
        if conflicts {
            // Keep the original values for manual inspection, but never replay
            // this completed/conflicted transaction on subsequent startup.
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            fs::rename(
                self.path(),
                self.directory
                    .join(format!("bindings-conflict-{stamp}.json")),
            )?;
        } else {
            fs::remove_file(self.path())?;
        }
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}
impl<S: Settings> Drop for Lease<S> {
    fn drop(&mut self) {
        if let Err(e) = self.restore() {
            eprintln!("BINDING RECOVERY REQUIRED: {e}; run media-router --restore-bindings");
        }
    }
}
fn without(values: &[String], accelerator: &str) -> Vec<String> {
    values
        .iter()
        .filter(|s| s.as_str() != accelerator)
        .cloned()
        .collect()
}
pub(crate) fn state_directory() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_STATE_HOME").filter(|p| Path::new(p).is_absolute()) {
        return Ok(PathBuf::from(path).join("media-router"));
    }
    let home = std::env::var_os("HOME")
        .filter(|p| Path::new(p).is_absolute())
        .ok_or::<Error>("an absolute HOME or XDG_STATE_HOME is required".into())?;
    Ok(PathBuf::from(home).join(".local/state/media-router"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap,
        rc::Rc,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "media-router-settings-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[derive(Clone)]
    struct Fake {
        values: Rc<RefCell<BTreeMap<String, Value>>>,
        fail: Rc<Cell<Option<usize>>>,
        writes: Rc<Cell<usize>>,
        journal: PathBuf,
    }
    impl Fake {
        fn new(directory: &Directory) -> Self {
            Self {
                values: Rc::new(RefCell::new(
                    BINDINGS
                        .iter()
                        .map(|(key, accelerator)| {
                            (
                                key.to_string(),
                                Value {
                                    effective: vec![accelerator.to_string()],
                                    user: None,
                                },
                            )
                        })
                        .collect(),
                )),
                fail: Rc::new(Cell::new(None)),
                writes: Rc::new(Cell::new(0)),
                journal: directory.0.join("bindings.json"),
            }
        }
    }
    impl Settings for Fake {
        fn read(&self, key: &str) -> Result<Value> {
            Ok(self.values.borrow()[key].clone())
        }
        fn write(&self, key: &str, value: Option<&[String]>) -> Result<()> {
            assert!(
                self.journal.exists(),
                "settings changed before recovery record existed"
            );
            self.writes.set(self.writes.get() + 1);
            if self.fail.get() == Some(self.writes.get()) {
                return Err("simulated write failure".into());
            }
            let default = BINDINGS.iter().find(|(k, _)| *k == key).unwrap().1;
            self.values.borrow_mut().insert(
                key.into(),
                Value {
                    effective: value
                        .map(<[String]>::to_vec)
                        .unwrap_or_else(|| vec![default.into()]),
                    user: value.map(<[String]>::to_vec),
                },
            );
            Ok(())
        }
    }
    #[test]
    fn preserves_custom_arrays_explicit_defaults_and_inherited_values() {
        let directory = Directory::new();
        let fake = Fake::new(&directory);
        let custom = vec!["<Control><Alt>p".into(), "XF86AudioPlay".into(), "".into()];
        fake.values.borrow_mut().insert(
            "play".into(),
            Value {
                effective: custom.clone(),
                user: Some(custom),
            },
        );
        fake.values.borrow_mut().get_mut("stop").unwrap().user = Some(vec!["XF86AudioStop".into()]);
        let original = fake.values.borrow().clone();
        let lease = Lease::open(fake.clone(), directory.0.clone()).unwrap();
        lease.acquire().unwrap();
        assert_eq!(
            fake.read("play").unwrap().effective,
            ["<Control><Alt>p", ""]
        );
        assert!(lease.unchanged().unwrap());
        lease.restore().unwrap();
        assert_eq!(*fake.values.borrow(), original);
        assert!(!lease.path().exists());
    }
    #[test]
    fn partial_acquisition_rolls_back_and_lock_excludes_other_processes() {
        let directory = Directory::new();
        let fake = Fake::new(&directory);
        let original = fake.values.borrow().clone();
        let lease = Lease::open(fake.clone(), directory.0.clone()).unwrap();
        assert!(Lease::open(fake.clone(), directory.0.clone()).is_err());
        fake.fail.set(Some(3));
        assert!(lease.acquire().is_err());
        drop(lease);
        assert_eq!(*fake.values.borrow(), original);
        assert!(Lease::open(fake, directory.0.clone()).is_ok());
    }
    #[test]
    fn recovery_is_repeatable_after_an_interrupted_restore() {
        let directory = Directory::new();
        let fake = Fake::new(&directory);
        let original = fake.values.borrow().clone();
        let lease = Lease::open(fake.clone(), directory.0.clone()).unwrap();
        lease.acquire().unwrap();
        fake.fail.set(Some(7));
        assert!(lease.restore().is_err());
        assert!(lease.path().exists());
        lease.restore().unwrap();
        assert_eq!(*fake.values.borrow(), original);
    }
    #[test]
    fn external_edit_is_preserved_and_original_backup_archived() {
        let directory = Directory::new();
        let fake = Fake::new(&directory);
        let lease = Lease::open(fake.clone(), directory.0.clone()).unwrap();
        lease.acquire().unwrap();
        fake.write("next", Some(&["<Super>n".into()])).unwrap();
        assert!(!lease.unchanged().unwrap());
        lease.restore().unwrap();
        assert_eq!(fake.read("next").unwrap().effective, ["<Super>n"]);
        assert_eq!(fake.read("play").unwrap().user, None);
        assert!(fs::read_dir(&directory.0).unwrap().any(|p| {
            p.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("bindings-conflict-")
        }));
    }
    #[test]
    fn corrupted_or_unrecognized_journal_never_writes_settings() {
        let directory = Directory::new();
        let fake = Fake::new(&directory);
        let lease = Lease::open(fake.clone(), directory.0.clone()).unwrap();
        for data in [
            "{",
            r#"{"version":99,"entries":[]}"#,
            r#"{"version":1,"entries":[{"key":"volume-up","original":{"effective":["XF86AudioPlay"],"user":null},"temporary":[]}]}"#,
        ] {
            fs::write(lease.path(), data).unwrap();
            assert!(lease.restore().is_err());
            assert!(lease.acquire().is_err());
        }
        assert_eq!(fake.writes.get(), 0);
    }
    #[test]
    fn native_gio_preserves_user_value_distinction_with_memory_backend() {
        let schema = gio::SettingsSchemaSource::default()
            .unwrap()
            .lookup(SCHEMA, true)
            .unwrap();
        let backend = gio::memory_settings_backend_new();
        let settings = Cinnamon(gio::Settings::new_full(&schema, Some(&backend), None));
        assert_eq!(settings.read("play").unwrap().user, None);
        settings.write("play", Some(&[])).unwrap();
        assert_eq!(settings.read("play").unwrap().user, Some(vec![]));
        settings.write("play", None).unwrap();
        assert_eq!(settings.read("play").unwrap().effective, ["XF86AudioPlay"]);
        assert_eq!(settings.read("play").unwrap().user, None);
    }
}
