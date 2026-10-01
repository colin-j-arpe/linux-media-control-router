//! Public wire contract exercised through the executable on a private bus.
use futures_util::StreamExt;
use media_router::{
    config::{Preferences, Store},
    dbus_api::{Identity, NAME, PATH, Settings, State},
    player::selection::ApplicationId,
};
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::time::timeout;
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Bus {
    _process: Process,
    directory: std::path::PathBuf,
    address: String,
}
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
impl Bus {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "media-router-api-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let mut process = Process(
            Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let mut address = String::new();
        BufReader::new(process.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Self {
            _process: process,
            directory,
            address: address.trim().into(),
        }
    }
    async fn connect(&self) -> zbus::Connection {
        zbus::connection::Builder::address(self.address.as_str())
            .unwrap()
            .method_timeout(Duration::from_secs(3))
            .build()
            .await
            .unwrap()
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_media-router"));
        command
            .env("DBUS_SESSION_BUS_ADDRESS", &self.address)
            .env("XDG_SESSION_TYPE", "wayland")
            .env("XDG_CONFIG_HOME", &self.directory)
            .env("XDG_STATE_HOME", self.directory.join("state"))
            .env("GSETTINGS_BACKEND", "memory")
            .env("GIO_USE_VFS", "local")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }
    fn config(&self) -> std::path::PathBuf {
        self.directory.join("media-router")
    }
    fn preferences(&self) -> Preferences {
        serde_json::from_slice(&std::fs::read(self.config().join("config.json")).unwrap()).unwrap()
    }
    async fn start(&self) -> Process {
        self.start_with(&["--serve", "--interactive"]).await
    }
    async fn start_with(&self, args: &[&str]) -> Process {
        let process = Process(
            self.command()
                .args(args)
                .stdin(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let connection = self.connect().await;
        let bus = zbus::fdo::DBusProxy::new(&connection).await.unwrap();
        timeout(Duration::from_secs(3), async {
            while !bus.name_has_owner(NAME.try_into().unwrap()).await.unwrap() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        process
    }
}
async fn proxy(connection: &zbus::Connection) -> zbus::Proxy<'_> {
    zbus::Proxy::new(connection, NAME, PATH, NAME)
        .await
        .unwrap()
}
async fn state(proxy: &zbus::Proxy<'_>) -> State {
    proxy.call("GetState", &()).await.unwrap()
}
async fn until(proxy: &zbus::Proxy<'_>, check: impl Fn(&State) -> bool) -> State {
    timeout(Duration::from_secs(3), async {
        loop {
            let current = state(proxy).await;
            if check(&current) {
                return current;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[derive(Default)]
struct Gate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
struct Root {
    name: String,
    desktop: String,
    gate: Option<std::sync::Arc<Gate>>,
}
#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    #[zbus(property)]
    async fn identity(&self) -> &str {
        if let Some(gate) = &self.gate {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        &self.name
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        &self.desktop
    }
}
struct Player;
#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    #[zbus(property)]
    fn playback_status(&self) -> &str {
        "Stopped"
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
async fn player(bus: &Bus, service: &str, desktop: &str, name: &str) -> zbus::Connection {
    zbus::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .name(service)
        .unwrap()
        .serve_at(
            "/org/mpris/MediaPlayer2",
            Root {
                name: name.into(),
                desktop: desktop.into(),
                gate: None,
            },
        )
        .unwrap()
        .serve_at("/org/mpris/MediaPlayer2", Player)
        .unwrap()
        .build()
        .await
        .unwrap()
}
#[tokio::test]
async fn public_contract_selection_revisions_errors_and_exclusive_ownership() {
    let bus = Bus::new();
    let mut daemon = bus.start().await;
    let connection = bus.connect().await;
    let client = proxy(&connection).await;
    let mut signals = client.receive_signal("StateChanged").await.unwrap();
    let initial = state(&client).await;
    assert_eq!(initial.revision, 0);
    assert_eq!(initial.selected.kind, "none");
    assert_eq!(initial.capture.status, "disabled");
    let selected: State = client
        .call("SelectApplication", &("identity", ""))
        .await
        .unwrap();
    assert_eq!(selected.selected.kind, "identity");
    assert_eq!(selected.selected.value, "");
    assert!(!selected.selected.available);
    assert_eq!(selected.revision, 1);
    let message = timeout(Duration::from_secs(2), signals.next())
        .await
        .unwrap()
        .unwrap();
    let signaled: State = message.body().deserialize().unwrap();
    assert_eq!(signaled, selected);
    let same: State = client
        .call("SelectApplication", &("identity", ""))
        .await
        .unwrap();
    assert_eq!(same, selected);
    for (kind, value) in [("desktop-entry", ""), ("unknown", "x"), ("none", "")] {
        let result: zbus::Result<State> = client.call("SelectApplication", &(kind, value)).await;
        assert!(
            matches!(result, Err(zbus::Error::MethodError(ref name, _, _)) if name.as_str() == "org.mediarouter.MediaRouter1.Error.InvalidArgument")
        );
        assert_eq!(state(&client).await, selected);
    }
    let second_connection = bus.connect().await;
    let second = proxy(&second_connection).await;
    let cleared: State = second.call("ClearSelection", &()).await.unwrap();
    assert_eq!(cleared.selected.kind, "none");
    assert_eq!(cleared.revision, 2);
    assert_eq!(state(&client).await, cleared);
    let introspection = zbus::fdo::IntrospectableProxy::builder(&connection)
        .destination(NAME)
        .unwrap()
        .path(PATH)
        .unwrap()
        .build()
        .await
        .unwrap()
        .introspect()
        .await
        .unwrap();
    for method in [
        "GetState",
        "SelectApplication",
        "ClearSelection",
        "SetCaptureEnabled",
        "StateChanged",
        "GetSettings",
        "SetAutoSelectNew",
        "SetAutoSelectExclusions",
        "SettingsChanged",
    ] {
        assert!(introspection.contains(&format!("name=\"{method}\"")));
    }
    assert!(introspection.contains("(ta(sssu)(ssbss)(ss))"));
    assert!(introspection.contains("(tbba(ss)s)"));
    let duplicate = bus
        .command()
        .args(["--serve", "--capture"])
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(!duplicate.status.success());
    let error = String::from_utf8_lossy(&duplicate.stderr);
    assert!(error.contains("name already taken") || error.contains("configuration writer lock"));
    assert_eq!(state(&client).await, cleared);
    // Unsupported capture fails explicitly but does not kill the service.
    let failed: zbus::Result<State> = client.call("SetCaptureEnabled", &(true,)).await;
    assert!(
        matches!(failed, Err(zbus::Error::MethodError(ref name, _, _)) if name.as_str() == "org.mediarouter.MediaRouter1.Error.CaptureFailed")
    );
    assert_eq!(state(&client).await.capture.status, "faulted");
    let _: State = client
        .call("SelectApplication", &("desktop-entry", "absent"))
        .await
        .unwrap();
    assert_eq!(state(&client).await.selected.value, "absent");
    assert!(
        daemon.0.try_wait().unwrap().is_none(),
        "stdin EOF must not stop --serve"
    );
    assert!(
        Command::new("kill")
            .args(["-TERM", &daemon.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    timeout(Duration::from_secs(3), async {
        loop {
            if let Some(status) = daemon.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let _restarted = bus.start().await;
    let restarted = state(&client).await;
    assert_eq!(restarted.selected.kind, "desktop-entry");
    assert_eq!(restarted.selected.value, "absent");
    until(&client, |s| s.capture.status == "faulted").await;
    assert!(settings(&client).await.capture_enabled);
}
#[tokio::test]
async fn inventory_groups_instances_retains_absent_selection_and_tracks_owner_changes() {
    let bus = Bus::new();
    let _daemon = bus.start().await;
    let connection = bus.connect().await;
    let client = proxy(&connection).await;
    let first = player(&bus, "org.mpris.MediaPlayer2.z", "music", "Music").await;
    until(&client, |s| s.applications.len() == 1).await;
    let selected: State = client
        .call("SelectApplication", &("desktop-entry", "music"))
        .await
        .unwrap();
    assert!(selected.selected.available);
    let second = player(&bus, "org.mpris.MediaPlayer2.a", "music", "Another Label").await;
    let fallback = player(&bus, "org.mpris.MediaPlayer2.fallback", "", "music").await;
    let current = until(&client, |s| {
        s.applications.len() == 2 && s.applications[0].instances == 2
    })
    .await;
    assert_eq!(current.applications[0].name, "Another Label");
    assert_eq!(current.applications[1].kind, "identity");
    assert_eq!(current.selected.owner, selected.selected.owner);
    first
        .release_name("org.mpris.MediaPlayer2.z")
        .await
        .unwrap();
    let replaced = until(&client, |s| {
        s.selected.service == "org.mpris.MediaPlayer2.a"
    })
    .await;
    assert!(replaced.revision > current.revision);
    second
        .release_name("org.mpris.MediaPlayer2.a")
        .await
        .unwrap();
    let absent = until(&client, |s| !s.selected.available).await;
    assert_eq!(absent.selected.value, "music");
    assert_eq!(absent.applications.len(), 1);
    let _: State = client
        .call("SelectApplication", &("identity", "music"))
        .await
        .unwrap();
    assert_eq!(
        state(&client).await.selected.owner,
        fallback.unique_name().unwrap().as_str()
    );
    let _: State = client
        .call("SelectApplication", &("desktop-entry", "music"))
        .await
        .unwrap();
    let returned = player(&bus, "org.mpris.MediaPlayer2.returned", "music", "Music").await;
    let available = until(&client, |s| s.selected.available).await;
    assert_eq!(
        available.selected.owner,
        returned.unique_name().unwrap().as_str()
    );
}

async fn settings(proxy: &zbus::Proxy<'_>) -> Settings {
    proxy.call("GetSettings", &()).await.unwrap()
}
async fn stop(process: &mut Process) {
    assert!(
        Command::new("kill")
            .args(["-TERM", &process.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    timeout(Duration::from_secs(3), async {
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
fn id(kind: &str, value: &str) -> Identity {
    Identity {
        kind: kind.into(),
        value: value.into(),
    }
}

#[tokio::test]
async fn settings_signals_validate_exclusions_and_persist_cli_overrides_and_clear() {
    let bus = Bus::new();
    let mut daemon = bus.start_with(&["-s", "-I", "", "--no-capture"]).await;
    let connection = bus.connect().await;
    let client = proxy(&connection).await;
    assert_eq!(state(&client).await.selected.kind, "identity");
    assert_eq!(
        bus.preferences().selected,
        Some(ApplicationId::Identity(String::new()))
    );
    let mut signals = client.receive_signal("SettingsChanged").await.unwrap();
    let initial = settings(&client).await;
    assert_eq!(initial.revision, 0);
    let enabled: Settings = client.call("SetAutoSelectNew", &(true,)).await.unwrap();
    assert!(enabled.auto_select_new);
    assert_eq!(enabled.revision, 1);
    let message = timeout(Duration::from_secs(2), signals.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(message.body().deserialize::<Settings>().unwrap(), enabled);
    assert_eq!(state(&client).await.revision, 0);
    let same: Settings = client.call("SetAutoSelectNew", &(true,)).await.unwrap();
    assert_eq!(same, enabled);
    let exclusions = vec![
        id("identity", ""),
        id("desktop-entry", "browser"),
        id("desktop-entry", "browser"),
    ];
    let changed: Settings = client
        .call("SetAutoSelectExclusions", &(exclusions,))
        .await
        .unwrap();
    assert_eq!(
        changed.exclusions,
        vec![id("desktop-entry", "browser"), id("identity", "")]
    );
    for exclusion in [id("none", ""), id("desktop-entry", "")] {
        let result: zbus::Result<Settings> = client
            .call("SetAutoSelectExclusions", &(vec![exclusion],))
            .await;
        assert!(
            matches!(result, Err(zbus::Error::MethodError(ref name, _, _)) if name.as_str().ends_with(".InvalidArgument"))
        );
        assert_eq!(settings(&client).await, changed);
    }
    let _: State = client.call("ClearSelection", &()).await.unwrap();
    assert!(bus.preferences().selected.is_none());
    stop(&mut daemon).await;
    let mut daemon = bus.start().await;
    let restored = settings(&client).await;
    assert_eq!(restored.revision, 0);
    assert!(restored.auto_select_new);
    assert_eq!(restored.exclusions, changed.exclusions);
    assert_eq!(state(&client).await.selected.kind, "none");
    stop(&mut daemon).await;
    let mut daemon = bus.start_with(&["-scp", "spotify"]).await;
    until(&client, |s| s.capture.status == "faulted").await;
    assert!(settings(&client).await.capture_enabled);
    assert_eq!(
        bus.preferences().selected,
        Some(ApplicationId::DesktopEntry("spotify".into()))
    );
    stop(&mut daemon).await;
    let _daemon = bus.start_with(&["-s", "--no-capture"]).await;
    assert!(!settings(&client).await.capture_enabled);
    assert_eq!(state(&client).await.capture.status, "disabled");
    assert_eq!(state(&client).await.selected.value, "spotify");
}

#[tokio::test]
async fn mro_respects_startup_instances_exclusions_manual_selection_and_departure() {
    let bus = Bus::new();
    let preferences = Preferences {
        selected: Some(ApplicationId::DesktopEntry("saved".into())),
        auto_select_new: true,
        exclusions: vec![ApplicationId::DesktopEntry("browser".into())],
        ..Preferences::default()
    };
    Store::open(bus.config())
        .unwrap()
        .save(preferences)
        .unwrap();
    let _startup = player(&bus, "org.mpris.MediaPlayer2.startup", "startup", "Startup").await;
    let _daemon = bus.start().await;
    let connection = bus.connect().await;
    let client = proxy(&connection).await;
    let initial = until(&client, |s| s.applications.len() == 1).await;
    assert_eq!(initial.selected.value, "saved");
    let one = player(&bus, "org.mpris.MediaPlayer2.one", "music", "Music").await;
    let selected = until(&client, |s| s.selected.value == "music").await;
    assert_eq!(
        bus.preferences().selected,
        Some(ApplicationId::DesktopEntry("music".into()))
    );
    let two = player(&bus, "org.mpris.MediaPlayer2.two", "music", "Music").await;
    let current = until(&client, |s| s.applications.iter().any(|a| a.instances == 2)).await;
    assert_eq!(current.selected.owner, selected.selected.owner);
    let _: State = client
        .call("SelectApplication", &("desktop-entry", "manual"))
        .await
        .unwrap();
    let _browser = player(&bus, "org.mpris.MediaPlayer2.browser", "browser", "Browser").await;
    let current = until(&client, |s| s.applications.len() == 3).await;
    assert_eq!(current.selected.value, "manual");
    let _: State = client
        .call("SelectApplication", &("desktop-entry", "browser"))
        .await
        .unwrap();
    assert!(state(&client).await.selected.available);
    let _: State = client.call("ClearSelection", &()).await.unwrap();
    assert!(settings(&client).await.auto_select_new);
    let three = player(&bus, "org.mpris.MediaPlayer2.three", "music", "Music").await;
    let current = until(&client, |s| s.applications.iter().any(|a| a.instances == 3)).await;
    assert_eq!(current.selected.kind, "none");
    let fallback = player(&bus, "org.mpris.MediaPlayer2.fallback", "", "browser").await;
    let current = until(&client, |s| s.selected.kind == "identity").await;
    assert_eq!(current.selected.value, "browser");
    fallback
        .release_name("org.mpris.MediaPlayer2.fallback")
        .await
        .unwrap();
    let absent = until(&client, |s| !s.selected.available).await;
    assert_eq!(absent.selected.kind, "identity");
    assert_eq!(absent.selected.value, "browser");
    for (player, service) in [(one, "one"), (two, "two"), (three, "three")] {
        player
            .release_name(format!("org.mpris.MediaPlayer2.{service}"))
            .await
            .unwrap();
    }
    until(&client, |s| {
        !s.applications.iter().any(|a| a.value == "music")
    })
    .await;
    let _returned = player(&bus, "org.mpris.MediaPlayer2.returned", "music", "Music").await;
    until(&client, |s| s.selected.value == "music").await;
    let _: Settings = client.call("SetAutoSelectNew", &(false,)).await.unwrap();
    let _disabled = player(
        &bus,
        "org.mpris.MediaPlayer2.disabled",
        "disabled",
        "Disabled",
    )
    .await;
    let current = until(&client, |s| {
        s.applications.iter().any(|a| a.value == "disabled")
    })
    .await;
    assert_eq!(current.selected.value, "music");
    let _: Settings = client.call("SetAutoSelectNew", &(true,)).await.unwrap();
    assert_eq!(state(&client).await.selected.value, "music");
}

#[tokio::test]
async fn failed_saves_leave_runtime_unchanged_and_report_api_and_automatic_errors() {
    let bus = Bus::new();
    let _anchor = player(&bus, "org.mpris.MediaPlayer2.anchor", "anchor", "Anchor").await;
    let _daemon = bus.start().await;
    let connection = bus.connect().await;
    let client = proxy(&connection).await;
    until(&client, |s| !s.applications.is_empty()).await;
    let _: Settings = client.call("SetAutoSelectNew", &(true,)).await.unwrap();
    let before = state(&client).await;
    std::fs::create_dir(bus.config().join("config.tmp")).unwrap();
    let failed: zbus::Result<State> = client
        .call("SelectApplication", &("desktop-entry", "music"))
        .await;
    assert!(
        matches!(failed, Err(zbus::Error::MethodError(ref name, _, _)) if name.as_str().ends_with(".PersistenceFailed"))
    );
    assert_eq!(state(&client).await, before);
    assert!(!settings(&client).await.persistence_error.is_empty());
    let failed: zbus::Result<State> = client.call("SetCaptureEnabled", &(true,)).await;
    assert!(
        matches!(failed, Err(zbus::Error::MethodError(ref name, _, _)) if name.as_str().ends_with(".PersistenceFailed"))
    );
    assert_eq!(state(&client).await.capture.status, "disabled");
    assert!(!settings(&client).await.capture_enabled);
    let _music = player(&bus, "org.mpris.MediaPlayer2.music", "music", "Music").await;
    let after = until(&client, |s| s.applications.len() == 2).await;
    assert_eq!(after.selected.kind, "none");
    assert!(bus.preferences().selected.is_none());
    std::fs::remove_dir(bus.config().join("config.tmp")).unwrap();
    let _: State = client
        .call("SelectApplication", &("desktop-entry", "music"))
        .await
        .unwrap();
    assert!(settings(&client).await.persistence_error.is_empty());
    assert_eq!(
        bus.preferences().selected,
        Some(ApplicationId::DesktopEntry("music".into()))
    );
}

#[tokio::test]
async fn delayed_validation_does_not_turn_startup_or_pre_enable_players_into_arrivals() {
    for startup in [true, false] {
        let bus = Bus::new();
        let _anchor = player(&bus, "org.mpris.MediaPlayer2.anchor", "anchor", "Anchor").await;
        Store::open(bus.config())
            .unwrap()
            .save(Preferences {
                auto_select_new: startup,
                ..Preferences::default()
            })
            .unwrap();
        let delayed = bus.connect().await;
        let gate = std::sync::Arc::new(Gate::default());
        delayed
            .object_server()
            .at(
                "/org/mpris/MediaPlayer2",
                Root {
                    name: "Delayed".into(),
                    desktop: "delayed".into(),
                    gate: Some(gate.clone()),
                },
            )
            .await
            .unwrap();
        delayed
            .object_server()
            .at("/org/mpris/MediaPlayer2", Player)
            .await
            .unwrap();
        if startup {
            delayed
                .request_name("org.mpris.MediaPlayer2.delayed")
                .await
                .unwrap();
        }
        let _daemon = bus.start().await;
        let connection = bus.connect().await;
        let client = proxy(&connection).await;
        until(&client, |s| !s.applications.is_empty()).await;
        if !startup {
            delayed
                .request_name("org.mpris.MediaPlayer2.delayed")
                .await
                .unwrap();
        }
        timeout(Duration::from_secs(1), gate.entered.notified())
            .await
            .unwrap();
        let _: Settings = client.call("SetAutoSelectNew", &(true,)).await.unwrap();
        gate.release.notify_one();
        let current = until(&client, |s| s.applications.len() == 2).await;
        assert_eq!(current.selected.kind, "none");
        let _new = player(&bus, "org.mpris.MediaPlayer2.new", "new", "New").await;
        until(&client, |s| s.selected.value == "new").await;
    }
}

#[test]
fn corrupt_configuration_blocks_serving_but_diagnostics_ignore_preferences() {
    let bus = Bus::new();
    std::fs::create_dir(bus.config()).unwrap();
    let path = bus.config().join("config.json");
    std::fs::write(&path, b"{bad").unwrap();
    let failed = bus
        .command()
        .args(["-s", "-c"])
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("invalid configuration"));
    assert_eq!(std::fs::read(&path).unwrap(), b"{bad");
    let diagnostic = bus
        .command()
        .args(["-i", "-p", "temporary"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(diagnostic.status.success());
    assert_eq!(std::fs::read(path).unwrap(), b"{bad");
}
