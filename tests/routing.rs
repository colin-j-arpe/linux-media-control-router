//! Transport and executable tests against private buses and recording players.
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use media_router::player::{
    routing::{self, RouteOutcome, SkipReason, TransportAction as Action},
    selection::{ApplicationId, InstanceId, SelectionState},
};
use tokio::{
    sync::{Notify, mpsc, watch},
    time::timeout,
};
use zbus::Connection;

const PATH: &str = "/org/mpris/MediaPlayer2";

struct Bus {
    child: Child,
    address: String,
}

impl Bus {
    fn start() -> Self {
        let child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("tests require dbus-daemon and local socket permissions");
        let mut bus = Self {
            child,
            address: String::new(),
        };
        BufReader::new(bus.child.stdout.take().unwrap())
            .read_line(&mut bus.address)
            .unwrap();
        bus.address = bus.address.trim().into();
        assert!(!bus.address.is_empty(), "private D-Bus failed to start");
        bus
    }

    async fn connect(&self) -> Connection {
        zbus::connection::Builder::address(self.address.as_str())
            .unwrap()
            .method_timeout(Duration::from_secs(3))
            .build()
            .await
            .unwrap()
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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

struct LiveProperties {
    control: bool,
    play: bool,
    pause: bool,
    next: bool,
    previous: bool,
    status: String,
}

impl Default for LiveProperties {
    fn default() -> Self {
        Self {
            control: true,
            play: true,
            pause: true,
            next: true,
            previous: true,
            status: "Paused".into(),
        }
    }
}

#[derive(Default)]
struct State {
    properties: Mutex<LiveProperties>,
    calls: Mutex<Vec<Action>>,
    block_property: AtomicBool,
    property_started: Notify,
    property_continue: Notify,
    fail_property: AtomicBool,
    fail_method: AtomicBool,
    block_method: AtomicBool,
}

impl State {
    async fn record(&self, action: Action) -> zbus::fdo::Result<()> {
        self.calls.lock().unwrap().push(action);
        if self.block_method.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
        if self.fail_method.load(Ordering::SeqCst) {
            return Err(zbus::fdo::Error::Failed("simulated player error".into()));
        }
        Ok(())
    }
}

struct Player(Arc<State>);
#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    async fn play_pause(&self) -> zbus::fdo::Result<()> {
        self.0.record(Action::PlayPause).await
    }
    async fn stop(&self) -> zbus::fdo::Result<()> {
        self.0.record(Action::Stop).await
    }
    async fn previous(&self) -> zbus::fdo::Result<()> {
        self.0.record(Action::Previous).await
    }
    async fn next(&self) -> zbus::fdo::Result<()> {
        self.0.record(Action::Next).await
    }
    #[zbus(property)]
    async fn can_control(&self) -> zbus::fdo::Result<bool> {
        if self.0.block_property.load(Ordering::SeqCst) {
            self.0.property_started.notify_one();
            self.0.property_continue.notified().await;
        }
        if self.0.fail_property.load(Ordering::SeqCst) {
            return Err(zbus::fdo::Error::Failed("property unavailable".into()));
        }
        Ok(self.0.properties.lock().unwrap().control)
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.0.properties.lock().unwrap().play
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.0.properties.lock().unwrap().pause
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.0.properties.lock().unwrap().next
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.0.properties.lock().unwrap().previous
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.0.properties.lock().unwrap().status.clone()
    }
}

struct Fixture {
    connection: Connection,
    state: Arc<State>,
    target: SelectionState,
}

async fn fixture(bus: &Bus, name: &str, application: &'static str) -> Fixture {
    let connection = bus.connect().await;
    let state = Arc::new(State::default());
    connection
        .object_server()
        .at(PATH, Root(application))
        .await
        .unwrap();
    connection
        .object_server()
        .at(PATH, Player(state.clone()))
        .await
        .unwrap();
    connection.request_name(name).await.unwrap();
    let target = SelectionState::Available {
        application: ApplicationId::DesktopEntry(application.into()),
        instance: InstanceId {
            service: name.into(),
            owner: connection.unique_name().unwrap().to_string(),
        },
    };
    Fixture {
        connection,
        state,
        target,
    }
}

#[tokio::test]
async fn all_four_actions_only_reach_the_selected_owner() {
    let bus = Bus::start();
    let selected = fixture(&bus, "org.mpris.MediaPlayer2.selected", "selected").await;
    let other = fixture(&bus, "org.mpris.MediaPlayer2.other", "other").await;
    let connection = bus.connect().await;
    let (_sender, current) = watch::channel(selected.target.clone());
    let actions = [
        Action::PlayPause,
        Action::Stop,
        Action::Previous,
        Action::Next,
    ];
    for action in actions {
        assert_eq!(
            routing::dispatch(&connection, &current, &selected.target, action).await,
            RouteOutcome::Acknowledged
        );
    }
    assert_eq!(*selected.state.calls.lock().unwrap(), actions);
    assert!(other.state.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reads_live_capabilities_and_current_playback_state_without_signals() {
    let bus = Bus::start();
    let selected = fixture(&bus, "org.mpris.MediaPlayer2.selected", "selected").await;
    let connection = bus.connect().await;
    let (_sender, current) = watch::channel(selected.target.clone());
    assert_eq!(
        routing::dispatch(&connection, &current, &selected.target, Action::Next).await,
        RouteOutcome::Acknowledged
    );
    selected.state.properties.lock().unwrap().next = false;
    assert_eq!(
        routing::dispatch(&connection, &current, &selected.target, Action::Next).await,
        RouteOutcome::Skipped(SkipReason::Unsupported("CanGoNext"))
    );
    selected.state.properties.lock().unwrap().previous = false;
    assert_eq!(
        routing::dispatch(&connection, &current, &selected.target, Action::Previous).await,
        RouteOutcome::Skipped(SkipReason::Unsupported("CanGoPrevious"))
    );
    selected.state.properties.lock().unwrap().play = false;
    assert_eq!(
        routing::dispatch(&connection, &current, &selected.target, Action::PlayPause).await,
        RouteOutcome::Skipped(SkipReason::Unsupported("CanPlay"))
    );
    selected.state.properties.lock().unwrap().status = "Playing".into();
    assert_eq!(
        routing::dispatch(&connection, &current, &selected.target, Action::PlayPause).await,
        RouteOutcome::Acknowledged
    );
    selected.state.properties.lock().unwrap().pause = false;
    assert_eq!(
        routing::dispatch(&connection, &current, &selected.target, Action::PlayPause).await,
        RouteOutcome::Skipped(SkipReason::Unsupported("CanPause"))
    );
    // Stop does not require CanPlay, CanPause, or the nonexistent CanStop.
    assert_eq!(
        routing::dispatch(&connection, &current, &selected.target, Action::Stop).await,
        RouteOutcome::Acknowledged
    );
    selected.state.properties.lock().unwrap().control = false;
    for action in [
        Action::PlayPause,
        Action::Stop,
        Action::Previous,
        Action::Next,
    ] {
        assert_eq!(
            routing::dispatch(&connection, &current, &selected.target, action).await,
            RouteOutcome::Skipped(SkipReason::Unsupported("CanControl"))
        );
    }
    assert_eq!(
        *selected.state.calls.lock().unwrap(),
        [Action::Next, Action::PlayPause, Action::Stop]
    );
}

#[tokio::test]
async fn missing_selection_and_invalid_properties_never_send_commands() {
    let bus = Bus::start();
    let selected = fixture(&bus, "org.mpris.MediaPlayer2.selected", "selected").await;
    let connection = bus.connect().await;
    let (sender, current) = watch::channel(SelectionState::Unselected);
    assert_eq!(
        routing::dispatch(
            &connection,
            &current,
            &SelectionState::Unselected,
            Action::Next
        )
        .await,
        RouteOutcome::Skipped(SkipReason::NoSelection)
    );
    let absent = SelectionState::Unavailable {
        application: ApplicationId::DesktopEntry("absent".into()),
    };
    sender.send_replace(absent.clone());
    assert_eq!(
        routing::dispatch(&connection, &current, &absent, Action::Next).await,
        RouteOutcome::Skipped(SkipReason::Unavailable)
    );
    assert_eq!(
        routing::dispatch(&connection, &current, &selected.target, Action::Next).await,
        RouteOutcome::Skipped(SkipReason::SelectionChanged)
    );
    sender.send_replace(selected.target.clone());
    selected.state.fail_property.store(true, Ordering::SeqCst);
    assert!(matches!(
        routing::dispatch(&connection, &current, &selected.target, Action::Next).await,
        RouteOutcome::NotSent(_)
    ));
    selected.state.fail_property.store(false, Ordering::SeqCst);
    selected.state.properties.lock().unwrap().status = "Unknown".into();
    assert!(matches!(
        routing::dispatch(&connection, &current, &selected.target, Action::PlayPause).await,
        RouteOutcome::NotSent(_)
    ));
    assert!(selected.state.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn owner_change_during_checks_does_not_route_to_old_or_new_player() {
    let bus = Bus::start();
    let name = "org.mpris.MediaPlayer2.selected";
    let old = fixture(&bus, name, "selected").await;
    let connection = bus.connect().await;
    let (_sender, current) = watch::channel(old.target.clone());
    old.state.block_property.store(true, Ordering::SeqCst);
    let operation = routing::dispatch(&connection, &current, &old.target, Action::Next);
    tokio::pin!(operation);
    tokio::select! {
        result = &mut operation => panic!("completed before owner change: {result:?}"),
        () = old.state.property_started.notified() => {}
    }
    old.connection.release_name(name).await.unwrap();
    let new = fixture(&bus, name, "selected").await;
    old.state.property_continue.notify_one();
    assert_eq!(
        operation.await,
        RouteOutcome::Skipped(SkipReason::OwnerChanged)
    );
    assert_eq!(
        routing::dispatch(&connection, &current, &old.target, Action::Stop).await,
        RouteOutcome::Skipped(SkipReason::OwnerChanged)
    );
    assert!(old.state.calls.lock().unwrap().is_empty());
    assert!(new.state.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn selection_change_during_checks_prevents_dispatch() {
    let bus = Bus::start();
    let selected = fixture(&bus, "org.mpris.MediaPlayer2.selected", "selected").await;
    let connection = bus.connect().await;
    let (sender, current) = watch::channel(selected.target.clone());
    selected.state.block_property.store(true, Ordering::SeqCst);
    let operation = routing::dispatch(&connection, &current, &selected.target, Action::Next);
    tokio::pin!(operation);
    tokio::select! {
        result = &mut operation => panic!("completed before selection change: {result:?}"),
        () = selected.state.property_started.notified() => {}
    }
    sender.send_replace(SelectionState::Unselected);
    selected.state.property_continue.notify_one();
    assert_eq!(
        operation.await,
        RouteOutcome::Skipped(SkipReason::SelectionChanged)
    );
    assert!(selected.state.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn timeouts_and_method_errors_are_not_retried() {
    let bus = Bus::start();
    let selected = fixture(&bus, "org.mpris.MediaPlayer2.selected", "selected").await;
    let connection = bus.connect().await;
    let (_sender, current) = watch::channel(selected.target.clone());
    selected.state.block_property.store(true, Ordering::SeqCst);
    assert!(matches!(
        routing::dispatch(&connection, &current, &selected.target, Action::Next).await,
        RouteOutcome::NotSent(_)
    ));
    assert!(selected.state.calls.lock().unwrap().is_empty());
    selected.state.block_property.store(false, Ordering::SeqCst);
    selected.state.property_continue.notify_one();
    selected.state.fail_method.store(true, Ordering::SeqCst);
    assert!(matches!(
        routing::dispatch(&connection, &current, &selected.target, Action::Stop).await,
        RouteOutcome::PlayerError(_)
    ));
    selected.state.fail_method.store(false, Ordering::SeqCst);
    selected.state.block_method.store(true, Ordering::SeqCst);
    assert!(matches!(
        routing::dispatch(&connection, &current, &selected.target, Action::PlayPause).await,
        RouteOutcome::Uncertain(_)
    ));
    assert_eq!(
        *selected.state.calls.lock().unwrap(),
        [Action::Stop, Action::PlayPause]
    );
}

struct Cli {
    child: Child,
    lines: mpsc::UnboundedReceiver<String>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Cli {
    fn start(bus: &Bus, interactive: bool) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_media-router"));
        command.args(["--select", "selected"]);
        if interactive {
            command.arg("--interactive");
        }
        let mut child = command
            .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::unbounded_channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
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

    fn input(&mut self, value: &str) {
        writeln!(self.child.stdin.as_mut().unwrap(), "{value}").unwrap();
    }

    async fn expect(&mut self, expected: &str) {
        timeout(Duration::from_secs(5), async {
            loop {
                let line = self.lines.recv().await.expect("CLI unexpectedly exited");
                if line == expected {
                    return;
                }
                assert!(
                    !line.starts_with("COMMAND "),
                    "unexpected command outcome: {line}; expected {expected}"
                );
            }
        })
        .await
        .expect("CLI output timed out");
    }

    async fn exit_successfully(&mut self) {
        timeout(Duration::from_secs(3), async {
            loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    assert!(status.success());
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("CLI did not exit");
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = self.reader.take().unwrap().join();
    }
}

#[tokio::test]
async fn interactive_cli_routes_in_order_drops_unavailable_inputs_and_handles_eof() {
    let bus = Bus::start();
    let other = fixture(&bus, "org.mpris.MediaPlayer2.other", "other").await;
    let mut cli = Cli::start(&bus, true);
    cli.expect("SELECTION UNAVAILABLE").await;
    cli.input("next");
    cli.expect("COMMAND SKIPPED: next: Unavailable").await;
    let name = "org.mpris.MediaPlayer2.selected";
    let selected = fixture(&bus, name, "selected").await;
    cli.expect("SELECTION AVAILABLE").await;
    assert!(selected.state.calls.lock().unwrap().is_empty());
    cli.input("play-pause\nstop\nprevious\nnext");
    for action in ["play-pause", "stop", "previous", "next"] {
        cli.expect(&format!("COMMAND ACKNOWLEDGED: {action}")).await;
    }
    assert_eq!(
        *selected.state.calls.lock().unwrap(),
        [
            Action::PlayPause,
            Action::Stop,
            Action::Previous,
            Action::Next
        ]
    );
    selected.state.properties.lock().unwrap().next = false;
    cli.input("next");
    cli.expect("COMMAND SKIPPED: next: Unsupported(\"CanGoNext\")")
        .await;
    selected.connection.release_name(name).await.unwrap();
    cli.expect("SELECTION UNAVAILABLE").await;
    cli.input("play-pause");
    cli.expect("COMMAND SKIPPED: play-pause: Unavailable").await;
    let returned = fixture(&bus, name, "selected").await;
    cli.expect("SELECTION AVAILABLE").await;
    assert!(returned.state.calls.lock().unwrap().is_empty());
    cli.input("stop");
    drop(cli.child.stdin.take());
    cli.expect("COMMAND ACKNOWLEDGED: stop").await;
    cli.exit_successfully().await;
    assert_eq!(*returned.state.calls.lock().unwrap(), [Action::Stop]);
    assert!(other.state.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn interactive_idle_stdin_does_not_prevent_sigint_shutdown() {
    let bus = Bus::start();
    let mut cli = Cli::start(&bus, true);
    cli.expect("SELECTION UNAVAILABLE").await;
    assert!(
        Command::new("kill")
            .args(["-INT", &cli.child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    cli.exit_successfully().await;
}

#[tokio::test]
async fn discovery_and_input_continue_while_dispatch_checks_are_waiting() {
    let bus = Bus::start();
    let name = "org.mpris.MediaPlayer2.selected";
    let selected = fixture(&bus, name, "selected").await;
    let mut cli = Cli::start(&bus, true);
    cli.expect("SELECTION AVAILABLE").await;
    selected.state.block_property.store(true, Ordering::SeqCst);
    cli.input("next");
    timeout(
        Duration::from_secs(1),
        selected.state.property_started.notified(),
    )
    .await
    .unwrap();
    selected.connection.release_name(name).await.unwrap();
    cli.expect("SELECTION UNAVAILABLE").await;
    cli.input("stop");
    cli.expect("COMMAND SKIPPED: stop: Unavailable").await;
    let replacement = fixture(&bus, name, "selected").await;
    cli.expect("SELECTION AVAILABLE").await;
    selected.state.property_continue.notify_one();
    cli.expect("COMMAND SKIPPED: next: OwnerChanged").await;
    assert!(selected.state.calls.lock().unwrap().is_empty());
    assert!(replacement.state.calls.lock().unwrap().is_empty());
    drop(cli.child.stdin.take());
    cli.exit_successfully().await;
}

#[tokio::test]
async fn default_observation_mode_ignores_transport_text_on_stdin() {
    let bus = Bus::start();
    let selected = fixture(&bus, "org.mpris.MediaPlayer2.selected", "selected").await;
    let mut cli = Cli::start(&bus, false);
    cli.expect("SELECTION AVAILABLE").await;
    cli.input("next\nstop");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(selected.state.calls.lock().unwrap().is_empty());
}
