//! Real daemon and mock Cinnamon service on a private bus; no live desktop input.
use futures_util::StreamExt;
use media_router::dbus_api::{NAME, PATH, Settings, State};
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use zbus::{Connection, Proxy};

const CSD: &str = "org.gnome.SettingsDaemon";
const KEY_PATH: &str = "/org/gnome/SettingsDaemon/MediaKeys";
const KEY_IFACE: &str = "org.gnome.SettingsDaemon.MediaKeys";
const LOCKER: &str = "org.cinnamon.ScreenSaver";
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Sandbox {
    bus: Process,
    address: String,
    directory: std::path::PathBuf,
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
impl Sandbox {
    fn new() -> Self {
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
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "media-router-cinnamon-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(directory.join("glib-2.0/settings")).unwrap();
        Self {
            bus,
            address: address.trim().into(),
            directory,
        }
    }
    async fn connect(&self) -> Connection {
        zbus::connection::Builder::address(self.address.as_str())
            .unwrap()
            .method_timeout(Duration::from_secs(3))
            .build()
            .await
            .unwrap()
    }
    fn daemon(&self) -> Process {
        Process(
            Command::new(env!("CARGO_BIN_EXE_media-router"))
                .args(["--serve", "--no-capture"])
                .env("DBUS_SESSION_BUS_ADDRESS", &self.address)
                .env("XDG_SESSION_TYPE", "x11")
                .env("XDG_CURRENT_DESKTOP", "X-Cinnamon")
                .env("GSETTINGS_BACKEND", "keyfile")
                .env("GIO_USE_VFS", "local")
                .env("XDG_CONFIG_HOME", &self.directory)
                .env("XDG_STATE_HOME", self.directory.join("state"))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }
}
#[derive(Default)]
struct Registrations {
    listeners: Vec<(String, String)>,
    count: usize,
    fail: bool,
}
struct MediaKeys(Arc<Mutex<Registrations>>);
#[zbus::interface(name = "org.gnome.SettingsDaemon.MediaKeys")]
impl MediaKeys {
    fn grab_media_player_keys(
        &self,
        application: &str,
        _time: u32,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let mut state = self.0.lock().unwrap();
        if state.fail {
            return Err(zbus::fdo::Error::Failed(
                "injected registration failure".into(),
            ));
        }
        state.count += 1;
        state.listeners.retain(|(name, _)| name != application);
        state
            .listeners
            .push((application.into(), header.sender().unwrap().to_string()));
        Ok(())
    }
    fn release_media_player_keys(&self, application: &str) {
        self.0
            .lock()
            .unwrap()
            .listeners
            .retain(|(name, _)| name != application);
    }
}
struct Screen;
#[zbus::interface(name = "org.cinnamon.ScreenSaver")]
impl Screen {
    fn get_active(&self) -> bool {
        false
    }
}
struct Root(&'static str);
#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    #[zbus(property)]
    fn identity(&self) -> &str {
        self.0
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        self.0
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
async fn player(
    sandbox: &Sandbox,
    id: &'static str,
    calls: Arc<Mutex<Vec<&'static str>>>,
) -> Connection {
    zbus::connection::Builder::address(sandbox.address.as_str())
        .unwrap()
        .name(format!("org.mpris.MediaPlayer2.{id}"))
        .unwrap()
        .serve_at("/org/mpris/MediaPlayer2", Root(id))
        .unwrap()
        .serve_at("/org/mpris/MediaPlayer2", Player(calls))
        .unwrap()
        .build()
        .await
        .unwrap()
}
async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("condition not reached");
}
async fn state_until(proxy: &Proxy<'_>, predicate: impl Fn(&State) -> bool) -> State {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let state: State = proxy.call("GetState", &()).await.unwrap();
            if predicate(&state) {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
async fn key(service: &Connection, state: &Mutex<Registrations>, key: &str) {
    let (application, owner) = state.lock().unwrap().listeners.last().unwrap().clone();
    service
        .emit_signal(
            Some(owner.as_str()),
            KEY_PATH,
            KEY_IFACE,
            "MediaPlayerKeyPressed",
            &(application, key),
        )
        .await
        .unwrap();
}
async fn competitor(connection: &Connection) {
    let proxy = Proxy::new(connection, CSD, KEY_PATH, KEY_IFACE)
        .await
        .unwrap();
    proxy
        .call::<_, _, ()>("GrabMediaPlayerKeys", &("competitor", 0u32))
        .await
        .unwrap();
}
fn router_wins(state: &Mutex<Registrations>) -> bool {
    state
        .lock()
        .unwrap()
        .listeners
        .last()
        .is_some_and(|(app, _)| app == "media-router")
}

#[tokio::test(flavor = "current_thread")]
async fn routing_reclaim_events_priority_faults_and_cleanup() {
    let sandbox = Sandbox::new();
    let registrations = Arc::new(Mutex::new(Registrations::default()));
    let service = zbus::connection::Builder::address(sandbox.address.as_str())
        .unwrap()
        .name(CSD)
        .unwrap()
        .serve_at(KEY_PATH, MediaKeys(registrations.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    let first_calls = Arc::new(Mutex::new(Vec::new()));
    let second_calls = Arc::new(Mutex::new(Vec::new()));
    let first = player(&sandbox, "first", first_calls.clone()).await;
    let _second = player(&sandbox, "second", second_calls.clone()).await;
    let client = sandbox.connect().await;
    let mut daemon = sandbox.daemon();
    let bus = zbus::fdo::DBusProxy::new(&client).await.unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        while !bus.name_has_owner(NAME.try_into().unwrap()).await.unwrap() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let api = Proxy::new(&client, NAME, PATH, NAME).await.unwrap();
    state_until(&api, |s| s.applications.len() == 2).await;
    assert!(
        api.call::<_, _, State>("ReclaimMediaKeys", &())
            .await
            .is_err()
    );
    let _: State = api
        .call("SelectApplication", &("desktop-entry", "first"))
        .await
        .unwrap();
    let enabled: State = api.call("SetCaptureEnabled", &(true,)).await.unwrap();
    assert_eq!(enabled.capture.status, "enabled");
    assert!(router_wins(&registrations));
    for event in ["Play", "Pause", "Stop", "Previous", "Next"] {
        key(&service, &registrations, event).await;
    }
    until(|| first_calls.lock().unwrap().len() == 5).await;
    assert_eq!(
        *first_calls.lock().unwrap(),
        ["play-pause", "play-pause", "stop", "previous", "next"]
    );
    assert!(second_calls.lock().unwrap().is_empty());
    for event in ["VolumeUp", "Mute", "Seek"] {
        key(&service, &registrations, event).await;
    }
    // A subsequent transport acknowledgement proves the earlier messages were processed.
    key(&service, &registrations, "Play").await;
    until(|| first_calls.lock().unwrap().len() == 6).await;

    let contender = sandbox.connect().await;
    let contender_proxy = Proxy::new(&contender, CSD, KEY_PATH, KEY_IFACE)
        .await
        .unwrap();
    let mut contender_events = contender_proxy
        .receive_signal("MediaPlayerKeyPressed")
        .await
        .unwrap();
    competitor(&contender).await;
    key(&service, &registrations, "Play").await;
    tokio::time::timeout(Duration::from_secs(2), contender_events.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first_calls.lock().unwrap().len(), 6);
    let before_settings: Settings = api.call("GetSettings", &()).await.unwrap();
    let before_file = std::fs::read(sandbox.directory.join("media-router/config.json")).unwrap();
    let _: State = api.call("ReclaimMediaKeys", &()).await.unwrap();
    assert!(router_wins(&registrations));
    let after_settings: Settings = api.call("GetSettings", &()).await.unwrap();
    assert_eq!(before_settings, after_settings);
    assert_eq!(
        before_file,
        std::fs::read(sandbox.directory.join("media-router/config.json")).unwrap()
    );

    competitor(&contender).await;
    let _: State = api
        .call("SelectApplication", &("desktop-entry", "second"))
        .await
        .unwrap();
    assert!(router_wins(&registrations));
    key(&service, &registrations, "Next").await;
    until(|| second_calls.lock().unwrap().len() == 1).await;
    competitor(&contender).await;
    let _third = player(&sandbox, "third", Arc::new(Mutex::new(Vec::new()))).await;
    state_until(&api, |s| s.applications.len() == 3).await;
    until(|| router_wins(&registrations)).await;

    competitor(&contender).await;
    let screen = zbus::connection::Builder::address(sandbox.address.as_str())
        .unwrap()
        .name(LOCKER)
        .unwrap()
        .serve_at("/org/cinnamon/ScreenSaver", Screen)
        .unwrap()
        .build()
        .await
        .unwrap();
    // Allow owner tracking to bind the new screensaver before its signal.
    tokio::time::sleep(Duration::from_millis(100)).await;
    screen
        .emit_signal(
            None::<&str>,
            "/org/cinnamon/ScreenSaver",
            LOCKER,
            "ActiveChanged",
            &(true,),
        )
        .await
        .unwrap();
    until(|| router_wins(&registrations)).await;
    key(&service, &registrations, "Stop").await;
    until(|| second_calls.lock().unwrap().len() == 2).await;
    screen.release_name(LOCKER).await.unwrap();
    let screen2 = zbus::connection::Builder::address(sandbox.address.as_str())
        .unwrap()
        .name(LOCKER)
        .unwrap()
        .build()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    competitor(&contender).await;
    screen2
        .emit_signal(
            None::<&str>,
            "/org/cinnamon/ScreenSaver",
            LOCKER,
            "ActiveChanged",
            &(true,),
        )
        .await
        .unwrap();
    until(|| router_wins(&registrations)).await;

    let _: State = api
        .call("SelectApplication", &("desktop-entry", "first"))
        .await
        .unwrap();
    first.close().await.unwrap();
    state_until(&api, |s| !s.selected.available).await;
    key(&service, &registrations, "Play").await;
    // Clearing selection synchronizes the daemon loop after the unavailable input.
    let _: State = api.call("ClearSelection", &()).await.unwrap();
    assert_eq!(second_calls.lock().unwrap().len(), 2);
    registrations.lock().unwrap().fail = true;
    assert!(
        api.call::<_, _, State>("ReclaimMediaKeys", &())
            .await
            .is_err()
    );
    state_until(&api, |s| s.capture.status == "faulted").await;
    let settings: Settings = api.call("GetSettings", &()).await.unwrap();
    assert!(settings.capture_enabled);
    assert!(
        !registrations
            .lock()
            .unwrap()
            .listeners
            .iter()
            .any(|(app, _)| app == "media-router")
    );
    registrations.lock().unwrap().fail = false;
    let _: State = api.call("SetCaptureEnabled", &(true,)).await.unwrap();
    assert!(router_wins(&registrations));
    registrations.lock().unwrap().fail = true;
    screen2
        .emit_signal(
            None::<&str>,
            "/org/cinnamon/ScreenSaver",
            LOCKER,
            "ActiveChanged",
            &(true,),
        )
        .await
        .unwrap();
    state_until(&api, |s| s.capture.status == "faulted").await;
    let still_enabled: Settings = api.call("GetSettings", &()).await.unwrap();
    assert!(still_enabled.capture_enabled);
    registrations.lock().unwrap().fail = false;
    let _: State = api.call("SetCaptureEnabled", &(true,)).await.unwrap();
    service.release_name(CSD).await.unwrap();
    state_until(&api, |s| s.capture.status == "faulted").await;
    service.request_name(CSD).await.unwrap();
    let _: State = api.call("SetCaptureEnabled", &(true,)).await.unwrap();
    let _: State = api.call("SetCaptureEnabled", &(false,)).await.unwrap();
    assert!(
        !registrations
            .lock()
            .unwrap()
            .listeners
            .iter()
            .any(|(app, _)| app == "media-router")
    );
    let count = registrations.lock().unwrap().count;
    let _: State = api
        .call("SelectApplication", &("desktop-entry", "second"))
        .await
        .unwrap();
    assert_eq!(registrations.lock().unwrap().count, count);
    let _: State = api.call("SetCaptureEnabled", &(true,)).await.unwrap();
    Command::new("kill")
        .args(["-TERM", &daemon.0.id().to_string()])
        .status()
        .unwrap();
    until(|| daemon.0.try_wait().unwrap().is_some()).await;
    assert!(
        !registrations
            .lock()
            .unwrap()
            .listeners
            .iter()
            .any(|(app, _)| app == "media-router")
    );
    assert!(
        !sandbox
            .directory
            .join("state/media-router/bindings.json")
            .exists()
    );
    let _ = &sandbox.bus; // Keep the private bus alive through all checks.
}

#[tokio::test(flavor = "current_thread")]
async fn custom_shortcuts_are_preserved_and_external_edits_fault_capture() {
    use gio::prelude::*;
    let sandbox = Sandbox::new();
    let schema = gio::SettingsSchemaSource::default()
        .unwrap()
        .lookup("org.cinnamon.desktop.keybindings.media-keys", true)
        .unwrap();
    let backend = gio::keyfile_settings_backend_new(
        sandbox
            .directory
            .join("glib-2.0/settings/keyfile")
            .to_str()
            .unwrap(),
        "/",
        None,
    );
    let settings = gio::Settings::new_full(&schema, Some(&backend), None);
    settings
        .set_strv("play", ["XF86AudioPlay", "<Control><Alt>p"])
        .unwrap();
    gio::Settings::sync();
    let registrations = Arc::new(Mutex::new(Registrations::default()));
    let _service = zbus::connection::Builder::address(sandbox.address.as_str())
        .unwrap()
        .name(CSD)
        .unwrap()
        .serve_at(KEY_PATH, MediaKeys(registrations.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    let _daemon = sandbox.daemon();
    let client = sandbox.connect().await;
    let bus = zbus::fdo::DBusProxy::new(&client).await.unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        while !bus.name_has_owner(NAME.try_into().unwrap()).await.unwrap() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let api = Proxy::new(&client, NAME, PATH, NAME).await.unwrap();
    assert!(
        api.call::<_, _, State>("SetCaptureEnabled", &(true,))
            .await
            .is_err()
    );
    let fault = state_until(&api, |s| s.capture.status == "faulted").await;
    assert!(fault.capture.error.contains("custom/disabled"));
    assert_eq!(registrations.lock().unwrap().count, 0);
    assert_eq!(settings.strv("play").len(), 2);
    settings.set_strv("play", ["XF86AudioPlay"]).unwrap();
    gio::Settings::sync();
    let _: State = api.call("SetCaptureEnabled", &(true,)).await.unwrap();
    settings
        .set_strv("previous", ["XF86AudioPrev", "<Control>p"])
        .unwrap();
    gio::Settings::sync();
    state_until(&api, |s| s.capture.status == "faulted").await;
    assert_eq!(settings.strv("previous").len(), 2);
    assert!(!router_wins(&registrations));
    let settings_state: Settings = api.call("GetSettings", &()).await.unwrap();
    assert!(settings_state.capture_enabled);
    let _: State = api.call("SetCaptureEnabled", &(false,)).await.unwrap();
    assert_eq!(settings.strv("previous").len(), 2);
    assert!(
        !sandbox
            .directory
            .join("state/media-router/bindings.json")
            .exists()
    );
}
