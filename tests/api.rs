//! Public wire contract exercised through the executable on a private bus.
use futures_util::StreamExt;
use media_router::dbus_api::{NAME, PATH, State};
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
    address: String,
}
impl Bus {
    fn new() -> Self {
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
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }
    async fn start(&self) -> Process {
        let process = Process(
            self.command()
                .args(["--serve", "--interactive"])
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
struct Root {
    name: String,
    desktop: String,
}
#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    #[zbus(property)]
    fn identity(&self) -> &str {
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
    ] {
        assert!(introspection.contains(&format!("name=\"{method}\"")));
    }
    assert!(introspection.contains("(ta(sssu)(ssbss)(ss))"));
    let duplicate = bus
        .command()
        .args(["--serve", "--capture"])
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("name already taken"));
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
    assert_eq!(restarted.revision, 0);
    assert_eq!(restarted.selected.kind, "none");
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
