//! Private X server, settings file, session bus, and recording player.
//! No keyboard event or settings write reaches the desktop session.
use gio::prelude::*;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{sync::mpsc, time::timeout};
use x11rb::{
    connection::Connection as _,
    protocol::{
        xproto::{AutoRepeatMode, ChangeKeyboardControlAux, ConnectionExt, GrabMode, ModMask},
        xtest::ConnectionExt as _,
    },
    rust_connection::RustConnection,
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
const SCHEMA: &str = "org.cinnamon.desktop.keybindings.media-keys";
const SYMBOLS: [u32; 5] = [0x1008ff14, 0x1008ff31, 0x1008ff15, 0x1008ff16, 0x1008ff17];
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Desktop {
    _xserver: Process,
    _bus: Process,
    directory: PathBuf,
    address: String,
    display: String,
    x: RustConnection,
    codes: [u8; 5],
    settings: gio::Settings,
}
impl Desktop {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "media-router-input-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(directory.join("config/glib-2.0/settings")).unwrap();
        let mut xserver = Process(
            Command::new("Xvfb")
                .args([
                    "-displayfd",
                    "1",
                    "-screen",
                    "0",
                    "800x600x24",
                    "-nolisten",
                    "tcp",
                    "-ac",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("Xvfb is required"),
        );
        let mut number = String::new();
        BufReader::new(xserver.0.stdout.take().unwrap())
            .read_line(&mut number)
            .unwrap();
        assert!(!number.trim().is_empty(), "Xvfb failed to start");
        let display = format!(":{}", number.trim());
        let (x, _) = x11rb::connect(Some(&display)).unwrap();
        let setup = x.setup();
        let mapping = x
            .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)
            .unwrap()
            .reply()
            .unwrap();
        let codes = SYMBOLS.map(|symbol| {
            setup.min_keycode
                + mapping
                    .keysyms
                    .chunks(usize::from(mapping.keysyms_per_keycode))
                    .position(|symbols| symbols[0] == symbol)
                    .expect("Xvfb transport keys") as u8
        });
        let mut bus = Process(
            Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let mut address = String::new();
        BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let schema = gio::SettingsSchemaSource::default()
            .unwrap()
            .lookup(SCHEMA, true)
            .unwrap();
        let backend = gio::keyfile_settings_backend_new(
            directory
                .join("config/glib-2.0/settings/keyfile")
                .to_str()
                .unwrap(),
            "/",
            None,
        );
        let settings = gio::Settings::new_full(&schema, Some(&backend), None);
        settings
            .set_strv("play", ["XF86AudioPlay", "<Control><Alt>p"])
            .unwrap();
        settings.set_strv("stop", ["XF86AudioStop"]).unwrap();
        settings
            .set_strv("volume-up", ["XF86AudioRaiseVolume", "<Super>u"])
            .unwrap();
        gio::Settings::sync();
        Self {
            _xserver: xserver,
            _bus: bus,
            directory,
            address: address.trim().into(),
            display,
            x,
            codes,
            settings,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_media-router"));
        command
            .env("DISPLAY", &self.display)
            .env("DBUS_SESSION_BUS_ADDRESS", &self.address)
            .env("XDG_SESSION_TYPE", "x11")
            .env("XDG_CURRENT_DESKTOP", "X-Cinnamon")
            .env("GSETTINGS_BACKEND", "keyfile")
            .env("GIO_USE_VFS", "local")
            .env("XDG_CONFIG_HOME", self.directory.join("config"))
            .env("XDG_STATE_HOME", self.directory.join("state"));
        command
    }
    fn code(&self, symbol: u32) -> u8 {
        let setup = self.x.setup();
        let mapping = self
            .x
            .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)
            .unwrap()
            .reply()
            .unwrap();
        setup.min_keycode
            + mapping
                .keysyms
                .chunks(usize::from(mapping.keysyms_per_keycode))
                .position(|row| row[0] == symbol)
                .unwrap() as u8
    }
    fn journal(&self) -> PathBuf {
        self.directory.join("state/media-router/bindings.json")
    }
    fn key(&self, code: u8, down: bool) {
        self.x
            .xtest_fake_input(if down { 2 } else { 3 }, code, 0, 0, 0, 0, 0)
            .unwrap()
            .check()
            .unwrap();
    }
    fn tap(&self, code: u8) {
        self.key(code, true);
        self.key(code, false);
    }
    async fn value(&self, key: &str, expected: &[&str]) {
        timeout(Duration::from_secs(3), async {
            loop {
                let context = gio::glib::MainContext::default();
                while context.pending() {
                    context.iteration(false);
                }
                if self
                    .settings
                    .strv(key)
                    .iter()
                    .map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    == expected
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("setting {key} did not become {expected:?}"));
    }
    async fn restored(&self) {
        self.value("play", &["XF86AudioPlay", "<Control><Alt>p"])
            .await;
        self.value("pause", &["XF86AudioPause"]).await;
        self.value("stop", &["XF86AudioStop"]).await;
        self.value("volume-up", &["XF86AudioRaiseVolume", "<Super>u"])
            .await;
        assert!(self.settings.user_value("pause").is_none());
        assert!(self.settings.user_value("stop").is_some());
        assert!(!self.journal().exists());
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
struct Router {
    process: Process,
    lines: mpsc::UnboundedReceiver<String>,
    pending: Vec<String>,
}
impl Router {
    fn start(desktop: &Desktop) -> Self {
        Self::with_args(desktop, &["--capture", "--select", "selected"])
    }
    fn with_args(desktop: &Desktop, args: &[&str]) -> Self {
        let mut command = desktop.command();
        command.args(args);
        Self::with_command(command)
    }
    fn with_command(mut command: Command) -> Self {
        let mut process = Process(
            command
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let (send, lines) = mpsc::unbounded_channel();
        let stdout = process.0.stdout.take().unwrap();
        let stderr = process.0.stderr.take().unwrap();
        let second = send.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if send.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                if second.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        Self {
            process,
            lines,
            pending: Vec::new(),
        }
    }
    async fn expect(&mut self, text: &str) {
        // stdout and stderr readers can deliver startup lines in either order.
        // Retain unmatched output so a later expectation cannot lose it.
        if let Some(index) = self.pending.iter().position(|line| line.contains(text)) {
            self.pending.remove(index);
            return;
        }
        timeout(Duration::from_secs(5), async {
            while let Some(line) = self.lines.recv().await {
                if line.contains(text) {
                    return;
                }
                self.pending.push(line);
            }
            panic!("router exited before {text}: {:?}", self.pending);
        })
        .await
        .unwrap_or_else(|_| panic!("router did not report {text}: {:?}", self.pending));
    }
    async fn exit(&mut self, success: bool) {
        timeout(Duration::from_secs(5), async {
            loop {
                if let Some(status) = self.process.0.try_wait().unwrap() {
                    assert_eq!(status.success(), success);
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("router did not exit");
    }
    fn signal(&self, signal: &str) {
        assert!(
            Command::new("kill")
                .args([signal, &self.process.0.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
    }
}
struct Root;
#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    #[zbus(property)]
    fn identity(&self) -> &str {
        "selected"
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        "selected"
    }
}
struct Player(Arc<Mutex<Vec<&'static str>>>);
#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn play_pause(&self) {
        self.0.lock().unwrap().push("play-pause");
    }
    fn stop(&self) {
        self.0.lock().unwrap().push("stop");
    }
    fn previous(&self) {
        self.0.lock().unwrap().push("previous");
    }
    fn next(&self) {
        self.0.lock().unwrap().push("next");
    }
    #[zbus(property)]
    fn playback_status(&self) -> &str {
        "Paused"
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        false
    }
}
#[tokio::test]
async fn hardware_routes_all_actions_suppresses_repeat_and_restores_on_sigterm() {
    let desktop = Desktop::new();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let _player = zbus::connection::Builder::address(desktop.address.as_str())
        .unwrap()
        .name("org.mpris.MediaPlayer2.selected")
        .unwrap()
        .serve_at("/org/mpris/MediaPlayer2", Root)
        .unwrap()
        .serve_at("/org/mpris/MediaPlayer2", Player(calls.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    let mut router = Router::start(&desktop);
    router.expect("CAPTURE READY").await;
    router.expect("SELECTION AVAILABLE").await;
    desktop.value("play", &["<Control><Alt>p"]).await;
    desktop
        .value("volume-up", &["XF86AudioRaiseVolume", "<Super>u"])
        .await;
    for (code, action) in
        desktop
            .codes
            .into_iter()
            .zip(["play-pause", "play-pause", "stop", "previous", "next"])
    {
        desktop.tap(code);
        router
            .expect(&format!("COMMAND ACKNOWLEDGED: {action}"))
            .await;
    }
    desktop
        .x
        .change_keyboard_control(
            &ChangeKeyboardControlAux::new()
                .key(u32::from(desktop.codes[4]))
                .auto_repeat_mode(AutoRepeatMode::ON),
        )
        .unwrap()
        .check()
        .unwrap();
    desktop.key(desktop.codes[4], true);
    router.expect("COMMAND ACKNOWLEDGED: next").await;
    tokio::time::sleep(Duration::from_millis(900)).await;
    desktop.key(desktop.codes[4], false);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        *calls.lock().unwrap(),
        [
            "play-pause",
            "play-pause",
            "stop",
            "previous",
            "next",
            "next"
        ]
    );
    // Lock modifiers still capture, while volume and modified transport
    // accelerators remain available for another client to grab.
    for symbol in [0xffe5, 0xff7f, 0xff14] {
        let lock = desktop.code(symbol);
        desktop.tap(lock);
        desktop.tap(desktop.codes[2]);
        router.expect("COMMAND ACKNOWLEDGED: stop").await;
        desktop.tap(lock);
    }
    let root = desktop.x.setup().roots[0].root;
    for (code, mask) in [
        (desktop.code(0x1008ff13), ModMask::ANY),
        (desktop.codes[0], ModMask::CONTROL),
    ] {
        desktop
            .x
            .grab_key(false, root, mask, code, GrabMode::ASYNC, GrabMode::ASYNC)
            .unwrap()
            .check()
            .unwrap();
        desktop
            .x
            .ungrab_key(code, root, mask)
            .unwrap()
            .check()
            .unwrap();
    }
    router.signal("-TERM");
    router.exit(true).await;
    desktop.restored().await;
}
#[tokio::test]
async fn sigkill_journal_recovers_manually_and_before_next_capture() {
    let desktop = Desktop::new();
    for manual in [true, false] {
        let mut router = Router::start(&desktop);
        router.expect("CAPTURE READY").await;
        assert!(desktop.journal().exists());
        router.process.0.kill().unwrap();
        router.exit(false).await;
        desktop.value("play", &["<Control><Alt>p"]).await;
        if manual {
            let output = desktop
                .command()
                .arg("--restore-bindings")
                .env_remove("DISPLAY")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        } else {
            let mut replacement = Router::start(&desktop);
            replacement.expect("CAPTURE READY").await;
            replacement.signal("-INT");
            replacement.exit(true).await;
        }
        desktop.restored().await;
    }
}
#[tokio::test]
async fn grab_conflict_rolls_back_and_external_edits_are_preserved() {
    let desktop = Desktop::new();
    let root = desktop.x.setup().roots[0].root;
    desktop
        .x
        .grab_key(
            false,
            root,
            ModMask::default(),
            desktop.codes[4],
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )
        .unwrap()
        .check()
        .unwrap();
    let mut failed = Router::start(&desktop);
    failed.exit(false).await;
    desktop.restored().await;
    desktop
        .x
        .ungrab_key(desktop.codes[4], root, ModMask::default())
        .unwrap()
        .check()
        .unwrap();
    let mut router = Router::start(&desktop);
    router.expect("CAPTURE READY").await;
    let output = desktop
        .command()
        .arg("--restore-bindings")
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "a second process must not restore active capture settings"
    );
    desktop.settings.set_strv("next", ["<Super>n"]).unwrap();
    gio::Settings::sync();
    router.exit(false).await;
    desktop.value("next", &["<Super>n"]).await;
    desktop.restored().await;
}

#[tokio::test]
async fn unavailable_inputs_are_dropped_and_mapping_changes_restore_bindings() {
    let desktop = Desktop::new();
    let mut router = Router::start(&desktop);
    router.expect("CAPTURE READY").await;
    router.expect("SELECTION UNAVAILABLE").await;
    desktop.tap(desktop.codes[0]);
    router
        .expect("COMMAND SKIPPED: play-pause: Unavailable")
        .await;
    desktop
        .x
        .change_keyboard_mapping(1, desktop.codes[0], 1, &[0x1008ff13])
        .unwrap()
        .check()
        .unwrap();
    router.exit(false).await;
    desktop.restored().await;
}

#[tokio::test]
async fn api_controls_capture_routes_runtime_selection_and_keeps_faults_recoverable() {
    use media_router::dbus_api::{NAME, PATH, State};
    let desktop = Desktop::new();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let connection = zbus::connection::Builder::address(desktop.address.as_str())
        .unwrap()
        .name("org.mpris.MediaPlayer2.selected")
        .unwrap()
        .serve_at("/org/mpris/MediaPlayer2", Root)
        .unwrap()
        .serve_at("/org/mpris/MediaPlayer2", Player(calls.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    let mut router = Router::with_args(&desktop, &["--serve"]);
    router.expect("API READY").await;
    let api = zbus::Proxy::new(&connection, NAME, PATH, NAME)
        .await
        .unwrap();
    router.expect("PLAYER ADDED").await;
    let initial: State = api.call("GetState", &()).await.unwrap();
    assert_eq!(initial.capture.status, "disabled");
    assert!(!desktop.journal().exists());
    let selected: State = api
        .call("SelectApplication", &("desktop-entry", "selected"))
        .await
        .unwrap();
    assert!(selected.selected.available);
    let enabled: State = api.call("SetCaptureEnabled", &(true,)).await.unwrap();
    assert_eq!(enabled.capture.status, "enabled");
    // Clean shutdown restores Cinnamon but retains desired capture and selection.
    router.signal("-TERM");
    router.exit(true).await;
    desktop.restored().await;
    router = Router::with_args(&desktop, &["--serve"]);
    router.expect("API READY").await;
    router.expect("SELECTION AVAILABLE").await;
    let restored: State = api.call("GetState", &()).await.unwrap();
    assert_eq!(restored.capture.status, "enabled");
    assert_eq!(restored.selected.value, "selected");
    desktop.value("play", &["<Control><Alt>p"]).await;
    desktop.tap(desktop.codes[4]);
    router.expect("COMMAND ACKNOWLEDGED: next").await;
    let _: State = api
        .call("SelectApplication", &("desktop-entry", "absent"))
        .await
        .unwrap();
    desktop.tap(desktop.codes[4]);
    router.expect("COMMAND SKIPPED: next: Unavailable").await;
    assert_eq!(*calls.lock().unwrap(), ["next"]);
    let disabled: State = api.call("SetCaptureEnabled", &(false,)).await.unwrap();
    assert_eq!(disabled.capture.status, "disabled");
    assert_eq!(disabled.selected.value, "absent");
    desktop.restored().await;
    let _: State = api.call("SetCaptureEnabled", &(true,)).await.unwrap();
    // A malformed recovery file must not be reported as a successful disable.
    let journal = fs::read(desktop.journal()).unwrap();
    fs::write(desktop.journal(), b"{").unwrap();
    let failure: zbus::Result<State> = api.call("SetCaptureEnabled", &(false,)).await;
    assert!(failure.is_err());
    let fault: State = api.call("GetState", &()).await.unwrap();
    assert_eq!(fault.capture.status, "faulted");
    assert!(!fault.capture.error.is_empty());
    assert!(desktop.journal().exists());
    fs::write(desktop.journal(), journal).unwrap();
    let recovered: State = api.call("SetCaptureEnabled", &(false,)).await.unwrap();
    assert_eq!(recovered.capture.status, "disabled");
    desktop.restored().await;
    // Input backend failure ends capture, while the API and selection survive.
    let _: State = api.call("SetCaptureEnabled", &(true,)).await.unwrap();
    desktop.settings.set_strv("next", ["<Super>n"]).unwrap();
    gio::Settings::sync();
    router.expect("CAPTURE FAULT").await;
    let fault: State = api.call("GetState", &()).await.unwrap();
    assert_eq!(fault.capture.status, "faulted");
    assert_eq!(fault.selected.value, "absent");
    desktop.value("next", &["<Super>n"]).await;
    let _: State = api.call("ClearSelection", &()).await.unwrap();
    let _: State = api.call("SetCaptureEnabled", &(false,)).await.unwrap();
    desktop.restored().await;
    router.signal("-TERM");
    router.exit(true).await;
}

#[tokio::test]
async fn serving_recovers_crashed_capture_even_when_capture_is_disabled_and_x_is_absent() {
    use media_router::dbus_api::{NAME, PATH, Settings, State};
    for saved_capture in [false, true] {
        let desktop = Desktop::new();
        let args = if saved_capture {
            vec!["--serve", "--capture"]
        } else {
            vec!["--capture"]
        };
        let mut crashed = Router::with_args(&desktop, &args);
        crashed.expect("CAPTURE READY").await;
        crashed.process.0.kill().unwrap();
        crashed.exit(false).await;
        assert!(desktop.journal().exists());
        desktop.value("play", &["<Control><Alt>p"]).await;
        let mut command = desktop.command();
        command
            .arg("--serve")
            .env_remove("DISPLAY")
            .env("XDG_SESSION_TYPE", "wayland");
        if saved_capture {
            command.arg("--no-capture");
        }
        let mut serving = Router::with_command(command);
        serving.expect("API READY").await;
        desktop.restored().await;
        let connection = zbus::connection::Builder::address(desktop.address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap();
        let api = zbus::Proxy::new(&connection, NAME, PATH, NAME)
            .await
            .unwrap();
        let state: State = api.call("GetState", &()).await.unwrap();
        let settings: Settings = api.call("GetSettings", &()).await.unwrap();
        assert_eq!(state.capture.status, "disabled");
        assert!(!settings.capture_enabled);
        serving.signal("-TERM");
        serving.exit(true).await;
    }
}

#[tokio::test]
async fn malformed_startup_journal_reports_fault_and_retains_intent_until_recovery_retry() {
    use media_router::dbus_api::{NAME, PATH, Settings, State};
    let desktop = Desktop::new();
    let mut crashed = Router::with_args(&desktop, &["--serve", "--capture"]);
    crashed.expect("CAPTURE READY").await;
    crashed.process.0.kill().unwrap();
    crashed.exit(false).await;
    let journal = fs::read(desktop.journal()).unwrap();
    fs::write(desktop.journal(), b"{bad").unwrap();
    let mut serving = Router::with_args(&desktop, &["--serve"]);
    serving.expect("API READY").await;
    let connection = zbus::connection::Builder::address(desktop.address.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    let api = zbus::Proxy::new(&connection, NAME, PATH, NAME)
        .await
        .unwrap();
    let state: State = api.call("GetState", &()).await.unwrap();
    assert_eq!(state.capture.status, "faulted");
    assert!(!state.capture.error.is_empty());
    let settings: Settings = api.call("GetSettings", &()).await.unwrap();
    assert!(settings.capture_enabled);
    assert_eq!(fs::read(desktop.journal()).unwrap(), b"{bad");
    desktop.value("play", &["<Control><Alt>p"]).await;
    // Fault does not stop selection/API access or discard the recovery record.
    let _: State = api
        .call("SelectApplication", &("identity", "absent"))
        .await
        .unwrap();
    fs::write(desktop.journal(), journal).unwrap();
    let recovered: State = api.call("SetCaptureEnabled", &(false,)).await.unwrap();
    assert_eq!(recovered.capture.status, "disabled");
    desktop.restored().await;
    serving.signal("-TERM");
    serving.exit(true).await;
}

#[tokio::test]
async fn serving_does_not_recover_bindings_owned_by_a_live_diagnostic_capturer() {
    use media_router::dbus_api::{NAME, PATH, State};
    let desktop = Desktop::new();
    let mut capturer = Router::start(&desktop);
    capturer.expect("CAPTURE READY").await;
    let journal = fs::read(desktop.journal()).unwrap();
    let mut serving = Router::with_args(&desktop, &["--serve"]);
    serving.expect("API READY").await;
    let connection = zbus::connection::Builder::address(desktop.address.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    let api = zbus::Proxy::new(&connection, NAME, PATH, NAME)
        .await
        .unwrap();
    let fault: State = api.call("GetState", &()).await.unwrap();
    assert_eq!(fault.capture.status, "faulted");
    assert!(fault.capture.error.contains("settings lock"));
    assert_eq!(fs::read(desktop.journal()).unwrap(), journal);
    desktop.value("play", &["<Control><Alt>p"]).await;
    assert!(capturer.process.0.try_wait().unwrap().is_none());
    capturer.signal("-TERM");
    capturer.exit(true).await;
    desktop.restored().await;
    let recovered: State = api.call("SetCaptureEnabled", &(false,)).await.unwrap();
    assert_eq!(recovered.capture.status, "disabled");
    serving.signal("-TERM");
    serving.exit(true).await;
}
