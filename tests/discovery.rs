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

struct Cli {
    child: Child,
    lines: mpsc::UnboundedReceiver<String>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Cli {
    fn start(bus: &PrivateBus, arguments: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_media-router"))
            .args(arguments)
            .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let output = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::unbounded_channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            lines,
            reader: Some(reader),
        }
    }

    async fn line(&mut self) -> String {
        timeout(Duration::from_secs(3), self.lines.recv())
            .await
            .expect("CLI output timed out")
            .expect("CLI exited unexpectedly")
    }

    async fn next_selection(&mut self) -> Vec<String> {
        loop {
            let header = self.line().await;
            match header.as_str() {
                "SELECTION NONE" => return vec![header],
                "SELECTION UNAVAILABLE" => return vec![header, self.line().await],
                "SELECTION AVAILABLE" => {
                    return vec![
                        header,
                        self.line().await,
                        self.line().await,
                        self.line().await,
                    ];
                }
                _ => {}
            }
        }
    }

    async fn observe_player_without_selection_change(&mut self, service: &str) {
        loop {
            let line = self.line().await;
            assert!(
                !line.starts_with("SELECTION "),
                "unexpected selection change: {line}"
            );
            if line == format!("  service: {service}") {
                return;
            }
        }
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = self.reader.take().unwrap().join();
    }
}

async fn selected_player(bus: &PrivateBus, name: &str, fallback: bool) -> Connection {
    let connection = bus.connect().await;
    if fallback {
        connection
            .object_server()
            .at(PATH, RootWithoutDesktop)
            .await
            .unwrap();
    } else {
        connection.object_server().at(PATH, Root).await.unwrap();
    }
    connection
        .object_server()
        .at(PATH, MockPlayer::default())
        .await
        .unwrap();
    connection.request_name(name).await.unwrap();
    connection
}

#[tokio::test]
async fn cli_retains_desktop_and_fallback_selections_across_instance_lifecycles() {
    for fallback in [false, true] {
        let bus = PrivateBus::start();
        let other_name = "org.mpris.MediaPlayer2.other";
        let _other = selected_player(&bus, other_name, !fallback).await;
        let (arguments, label) = if fallback {
            (
                ["--select-identity", "Anonymous Player"],
                "identity \"Anonymous Player\"",
            )
        } else {
            (["--select", "test-player"], "desktop-entry \"test-player\"")
        };
        let mut cli = Cli::start(&bus, &arguments);
        let unavailable = vec![
            "SELECTION UNAVAILABLE".to_owned(),
            format!("  application: {label}"),
        ];
        assert_eq!(cli.next_selection().await, unavailable);
        cli.observe_player_without_selection_change(other_name)
            .await;

        let first_name = "org.mpris.MediaPlayer2.z";
        let first = selected_player(&bus, first_name, fallback).await;
        let available = |name: &str, connection: &Connection| {
            vec![
                "SELECTION AVAILABLE".to_owned(),
                format!("  application: {label}"),
                format!("  service: {name}"),
                format!("  owner: {}", connection.unique_name().unwrap()),
            ]
        };
        assert_eq!(cli.next_selection().await, available(first_name, &first));

        let second_name = "org.mpris.MediaPlayer2.a";
        let second = selected_player(&bus, second_name, fallback).await;
        cli.observe_player_without_selection_change(second_name)
            .await;
        let third_name = "org.mpris.MediaPlayer2.b";
        let third = selected_player(&bus, third_name, fallback).await;
        cli.observe_player_without_selection_change(third_name)
            .await;

        first.release_name(first_name).await.unwrap();
        assert_eq!(cli.next_selection().await, available(second_name, &second));
        second.release_name(second_name).await.unwrap();
        assert_eq!(cli.next_selection().await, available(third_name, &third));
        third.release_name(third_name).await.unwrap();
        assert_eq!(cli.next_selection().await, unavailable);

        let returned_name = "org.mpris.MediaPlayer2.returned";
        let returned = selected_player(&bus, returned_name, fallback).await;
        assert_eq!(
            cli.next_selection().await,
            available(returned_name, &returned)
        );
        drop(cli);
        let mut restarted = Cli::start(&bus, &[]);
        assert_eq!(restarted.next_selection().await, vec!["SELECTION NONE"]);
        restarted
            .observe_player_without_selection_change(returned_name)
            .await;
    }
}

#[test]
fn cli_help_and_argument_errors_do_not_require_a_session_bus() {
    for (arguments, exit_code, expected) in [
        (vec!["--help"], 0, "--select-identity"),
        (vec!["--select"], 2, "expected no arguments"),
        (
            vec!["--select", "a", "--select-identity", "b"],
            2,
            "expected no arguments",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_media-router"))
            .args(arguments)
            .env("DBUS_SESSION_BUS_ADDRESS", "invalid:")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(exit_code));
        let text = if exit_code == 0 {
            output.stdout
        } else {
            output.stderr
        };
        assert!(String::from_utf8(text).unwrap().contains(expected));
    }
}
