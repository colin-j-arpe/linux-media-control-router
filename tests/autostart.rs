//! Exercise installation through the CLI without touching the login session.
use std::{fs, path::PathBuf, process::Command};

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cli_installs_and_removes_using_xdg_and_home_without_bus_access() {
    let directory = Directory(
        std::env::temp_dir().join(format!("media-router-autostart-cli-{}", std::process::id())),
    );
    fs::create_dir(&directory.0).unwrap();
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_media-router"));
    for xdg in [
        Some(directory.0.join("xdg")),
        Some(PathBuf::from("relative-ignored")),
        None,
    ] {
        let base = xdg
            .as_ref()
            .filter(|path| path.is_absolute())
            .cloned()
            .unwrap_or_else(|| directory.0.join(".config"));
        let command = || {
            let mut command = Command::new(&executable);
            command
                .env("HOME", &directory.0)
                .env("DBUS_SESSION_BUS_ADDRESS", "invalid:")
                .env_remove("DISPLAY")
                .env_remove("XDG_CONFIG_HOME");
            if let Some(xdg) = &xdg {
                command.env("XDG_CONFIG_HOME", xdg);
            }
            command
        };
        let installed = command()
            .arg("--install-autostart")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            installed.status.success(),
            "{}",
            String::from_utf8_lossy(&installed.stderr)
        );
        let entry = base.join("autostart/media-router.desktop");
        let keyfile = gio::glib::KeyFile::new();
        keyfile
            .load_from_file(&entry, gio::glib::KeyFileFlags::NONE)
            .unwrap();
        assert_eq!(
            keyfile.string("Desktop Entry", "TryExec").unwrap().as_str(),
            executable.to_str().unwrap()
        );
        assert_eq!(
            keyfile
                .string("Desktop Entry", "OnlyShowIn")
                .unwrap()
                .as_str(),
            "X-Cinnamon;"
        );
        assert!(
            keyfile
                .string("Desktop Entry", "Exec")
                .unwrap()
                .ends_with(" --serve")
        );
        assert!(!base.join("media-router/config.json").exists());
        let removed = command().arg("--remove-autostart").output().unwrap();
        assert!(removed.status.success());
        assert!(!entry.exists());
        assert!(
            command()
                .arg("--remove-autostart")
                .status()
                .unwrap()
                .success()
        );
    }
}
