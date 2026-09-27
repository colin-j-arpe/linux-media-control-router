//! Real D-Bus transport tests on isolated buses; never use the desktop session bus.
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};

use media_router::player::{
    PlaybackStatus,
    discovery::{self, DiscoveryEvent},
};
use tokio::{
    sync::{Notify, mpsc},
    task::JoinHandle,
    time::timeout,
};
use zbus::Connection;

const PATH: &str = "/org/mpris/MediaPlayer2";
const ANCHOR: &str = "org.mpris.MediaPlayer2.anchor";

struct PrivateBus {
    child: Child,
    address: String,
}

impl PrivateBus {
    fn start() -> Self {
        let child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("integration tests require dbus-daemon and permission to create local sockets");
        let mut bus = Self {
            child,
            address: String::new(),
        };
        BufReader::new(bus.child.stdout.take().unwrap())
            .read_line(&mut bus.address)
            .unwrap();
        bus.address = bus.address.trim().to_owned();
        assert!(
            !bus.address.is_empty(),
            "private D-Bus daemon did not start"
        );
        bus
    }

    async fn connect(&self) -> Connection {
        zbus::connection::Builder::address(self.address.as_str())
            .unwrap()
            .method_timeout(Duration::from_secs(2))
            .build()
            .await
            .unwrap()
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Root;

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    #[zbus(property)]
    fn identity(&self) -> &str {
        "Test Player"
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        "test-player"
    }
}

struct RootWithoutDesktop;

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl RootWithoutDesktop {
    #[zbus(property)]
    fn identity(&self) -> &str {
        "Anonymous Player"
    }
}

struct MockPlayer {
    controllable: bool,
    stall: Option<Arc<Notify>>,
}

impl Default for MockPlayer {
    fn default() -> Self {
        Self {
            controllable: true,
            stall: None,
        }
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl MockPlayer {
    #[zbus(property)]
    async fn can_control(&self) -> bool {
        if let Some(started) = &self.stall {
            started.notify_one();
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
        self.controllable
    }
    #[zbus(property)]
    fn playback_status(&self) -> &str {
        "Paused"
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
        false
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

async fn export(connection: &Connection, player: MockPlayer) {
    connection.object_server().at(PATH, Root).await.unwrap();
    connection.object_server().at(PATH, player).await.unwrap();
}

async fn player(bus: &PrivateBus, name: &str) -> Connection {
    let connection = bus.connect().await;
    export(&connection, MockPlayer::default()).await;
    connection.request_name(name).await.unwrap();
    connection
}

struct Monitor {
    task: JoinHandle<zbus::Result<()>>,
    events: mpsc::UnboundedReceiver<DiscoveryEvent>,
}

impl Monitor {
    async fn start(bus: &PrivateBus) -> Self {
        let connection = bus.connect().await;
        let (sender, events) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            discovery::watch(&connection, |event| {
                let _ = sender.send(event);
            })
            .await
        });
        Self { task, events }
    }

    async fn next(&mut self) -> DiscoveryEvent {
        // Five two-second probes plus retry delays can take nearly twelve seconds
        // for a name whose owner never responds at all.
        timeout(Duration::from_secs(15), self.events.recv())
            .await
            .expect("timed out waiting for discovery event")
            .expect("watcher ended")
    }

    async fn quiet(&mut self) {
        assert!(
            timeout(Duration::from_millis(150), self.events.recv())
                .await
                .is_err(),
            "unexpected extra discovery event"
        );
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn ready_monitor(bus: &PrivateBus) -> (Connection, Monitor) {
    let anchor = player(bus, ANCHOR).await;
    let mut monitor = Monitor::start(bus).await;
    assert!(matches!(monitor.next().await, DiscoveryEvent::Added(p) if p.service == ANCHOR));
    (anchor, monitor)
}

#[tokio::test]
async fn discovers_existing_instances_and_optional_desktop_entry() {
    let bus = PrivateBus::start();
    let first = player(&bus, "org.mpris.MediaPlayer2.test.one").await;
    let second = player(&bus, "org.mpris.MediaPlayer2.test.two").await;
    let anonymous = bus.connect().await;
    anonymous
        .object_server()
        .at(PATH, RootWithoutDesktop)
        .await
        .unwrap();
    anonymous
        .object_server()
        .at(PATH, MockPlayer::default())
        .await
        .unwrap();
    anonymous
        .request_name("org.mpris.MediaPlayer2.anonymous")
        .await
        .unwrap();
    let unrelated = bus.connect().await;
    unrelated
        .request_name("org.example.Unrelated")
        .await
        .unwrap();

    let mut monitor = Monitor::start(&bus).await;
    let mut discovered = Vec::new();
    for _ in 0..3 {
        let DiscoveryEvent::Added(p) = monitor.next().await else {
            panic!("expected player")
        };
        assert_eq!(p.playback_status, PlaybackStatus::Paused);
        assert!(!p.capabilities.can_go_next);
        discovered.push(p);
    }
    assert_eq!(
        discovered
            .iter()
            .filter(|p| p.desktop_entry.as_deref() == Some("test-player"))
            .count(),
        2
    );
    assert!(
        discovered
            .iter()
            .any(|p| p.desktop_entry.is_none() && p.identity == "Anonymous Player")
    );
    assert!(
        discovered
            .iter()
            .any(|p| p.owner == first.unique_name().unwrap().as_str())
    );
    assert!(
        discovered
            .iter()
            .any(|p| p.owner == second.unique_name().unwrap().as_str())
    );
    monitor.quiet().await;
}

#[tokio::test]
async fn retries_initializing_player_then_reports_departure_and_return() {
    let bus = PrivateBus::start();
    let (_anchor, mut monitor) = ready_monitor(&bus).await;
    let name = "org.mpris.MediaPlayer2.delayed";
    let connection = bus.connect().await;
    connection.request_name(name).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    export(&connection, MockPlayer::default()).await;
    assert!(matches!(monitor.next().await, DiscoveryEvent::Added(p) if p.service == name));
    connection.release_name(name).await.unwrap();
    assert!(matches!(monitor.next().await, DiscoveryEvent::Removed(p) if p.service == name));
    let replacement = player(&bus, name).await;
    assert!(matches!(monitor.next().await, DiscoveryEvent::Added(p)
        if p.owner == replacement.unique_name().unwrap().as_str()));
    monitor.quiet().await;
}

#[tokio::test]
async fn rejects_phantom_and_uncontrollable_services() {
    let bus = PrivateBus::start();
    let (_anchor, mut monitor) = ready_monitor(&bus).await;
    let phantom = bus.connect().await;
    phantom
        .request_name("org.mpris.MediaPlayer2.phantom")
        .await
        .unwrap();
    let disabled = bus.connect().await;
    export(
        &disabled,
        MockPlayer {
            controllable: false,
            stall: None,
        },
    )
    .await;
    disabled
        .request_name("org.mpris.MediaPlayer2.disabled")
        .await
        .unwrap();
    let mut reasons = Vec::new();
    for _ in 0..2 {
        let DiscoveryEvent::Skipped { reason, .. } = monitor.next().await else {
            panic!("invalid player admitted")
        };
        reasons.push(reason);
    }
    assert!(reasons.iter().any(|reason| reason == "CanControl is false"));
    assert!(
        reasons
            .iter()
            .any(|reason| reason.contains("after 5 attempts"))
    );
    monitor.quiet().await;
}

#[tokio::test]
async fn pending_validation_does_not_block_other_players_and_cannot_survive_owner_loss() {
    let bus = PrivateBus::start();
    let (_anchor, mut monitor) = ready_monitor(&bus).await;
    let name = "org.mpris.MediaPlayer2.slow";
    let slow = bus.connect().await;
    let started = Arc::new(Notify::new());
    export(
        &slow,
        MockPlayer {
            controllable: true,
            stall: Some(started.clone()),
        },
    )
    .await;
    slow.request_name(name).await.unwrap();
    timeout(Duration::from_secs(2), started.notified())
        .await
        .unwrap();
    let other = player(&bus, "org.mpris.MediaPlayer2.other").await;
    let event = timeout(Duration::from_secs(1), monitor.next())
        .await
        .unwrap();
    assert!(
        matches!(event, DiscoveryEvent::Added(p) if p.owner == other.unique_name().unwrap().as_str())
    );
    slow.release_name(name).await.unwrap();
    let replacement = player(&bus, name).await;
    assert!(matches!(monitor.next().await, DiscoveryEvent::Added(p)
        if p.owner == replacement.unique_name().unwrap().as_str()));
    monitor.quiet().await;
}

#[tokio::test]
async fn owner_replacement_removes_old_instance_before_adding_new_one() {
    let bus = PrivateBus::start();
    let (_anchor, mut monitor) = ready_monitor(&bus).await;
    let name = "org.mpris.MediaPlayer2.replaceable";
    let old = zbus::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .serve_at(PATH, Root)
        .unwrap()
        .serve_at(PATH, MockPlayer::default())
        .unwrap()
        .allow_name_replacements(true)
        .name(name)
        .unwrap()
        .build()
        .await
        .unwrap();
    assert!(
        matches!(monitor.next().await, DiscoveryEvent::Added(p) if p.owner == old.unique_name().unwrap().as_str())
    );
    let new = zbus::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .serve_at(PATH, Root)
        .unwrap()
        .serve_at(PATH, MockPlayer::default())
        .unwrap()
        .replace_existing_names(true)
        .name(name)
        .unwrap()
        .build()
        .await
        .unwrap();
    assert!(
        matches!(monitor.next().await, DiscoveryEvent::Removed(p) if p.owner == old.unique_name().unwrap().as_str())
    );
    assert!(
        matches!(monitor.next().await, DiscoveryEvent::Added(p) if p.owner == new.unique_name().unwrap().as_str())
    );
    monitor.quiet().await;
}

#[tokio::test]
async fn loss_of_bus_ends_discovery_with_an_error() {
    let mut bus = PrivateBus::start();
    let (_anchor, mut monitor) = ready_monitor(&bus).await;
    bus.child.kill().unwrap();
    let result = timeout(Duration::from_secs(3), &mut monitor.task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
}
