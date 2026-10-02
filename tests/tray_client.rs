//! An independently compiled GUI client against the real daemon on a private bus.
use media_router_tray::{
    client,
    model::{Action, Command as Intent, View},
    protocol::{Identity, NAME},
};
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::sync::{mpsc, watch};

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
        let directory =
            std::env::temp_dir().join(format!("media-router-tray-test-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        Self {
            bus,
            address: address.trim().into(),
            directory,
        }
    }
    fn daemon(&self) -> Process {
        Process(
            Command::new(env!("CARGO_BIN_EXE_media-router"))
                .args(["--serve", "--no-capture"])
                .env("DBUS_SESSION_BUS_ADDRESS", &self.address)
                .env("XDG_CONFIG_HOME", &self.directory)
                .env("XDG_STATE_HOME", self.directory.join("state"))
                .env("XDG_SESSION_TYPE", "wayland")
                .env("GSETTINGS_BACKEND", "memory")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }
}
async fn wait(views: &mut watch::Receiver<View>, predicate: impl Fn(&View) -> bool) -> View {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let view = views.borrow_and_update().clone();
            if predicate(&view) {
                return view;
            }
            views.changed().await.unwrap();
        }
    })
    .await
    .expect("client did not reach expected state")
}
async fn send(commands: &mpsc::Sender<Intent>, owner: &str, action: Action) {
    commands
        .send(Intent {
            owner: owner.into(),
            action,
        })
        .await
        .unwrap();
}
#[tokio::test(flavor = "current_thread")]
async fn real_daemon_contract_settings_errors_and_restart() {
    let sandbox = Sandbox::new();
    let connection = zbus::connection::Builder::address(sandbox.address.as_str())
        .unwrap()
        .method_timeout(Duration::from_secs(3))
        .build()
        .await
        .unwrap();
    let (commands, inbox) = mpsc::channel(1);
    let (publish, mut views) = watch::channel(View::default());
    let worker = tokio::spawn(client::run(connection.clone(), inbox, publish));
    wait(&mut views, |v| !v.connected() && !v.error.is_empty()).await;
    let daemon = sandbox.daemon();
    let initial = wait(&mut views, View::connected).await;
    let owner = initial.owner.unwrap();
    let empty = Identity {
        kind: "identity".into(),
        value: String::new(),
    };
    send(&commands, &owner, Action::Select(Some(empty.clone()))).await;
    let selected = wait(&mut views, |v| {
        !v.busy
            && v.state
                .as_ref()
                .is_some_and(|s| s.selected.kind == "identity")
    })
    .await;
    assert!(selected.warning()); // Absent selection must remain visible, not fall back.
    assert_eq!(selected.state.unwrap().selected.value, "");
    send(&commands, &owner, Action::AutoSelect(true)).await;
    wait(&mut views, |v| {
        !v.busy && v.settings.as_ref().is_some_and(|s| s.auto_select_new)
    })
    .await;
    send(&commands, &owner, Action::Exclude(empty.clone(), true)).await;
    wait(&mut views, |v| {
        !v.busy
            && v.settings
                .as_ref()
                .is_some_and(|s| s.exclusions.contains(&empty))
    })
    .await;
    let other = Identity {
        kind: "desktop-entry".into(),
        value: "absent-player".into(),
    };
    send(&commands, &owner, Action::Exclude(other.clone(), true)).await;
    wait(&mut views, |v| {
        !v.busy && v.settings.as_ref().is_some_and(|s| s.exclusions.len() == 2)
    })
    .await;
    send(&commands, &owner, Action::Exclude(empty.clone(), false)).await;
    let excluded = wait(&mut views, |v| {
        !v.busy
            && v.settings
                .as_ref()
                .is_some_and(|s| s.exclusions == vec![other.clone()])
    })
    .await;
    assert!(excluded.error.is_empty());
    let blocked_save = sandbox.directory.join("media-router/config.tmp");
    std::fs::create_dir(&blocked_save).unwrap();
    send(&commands, &owner, Action::Select(None)).await;
    let failed = wait(&mut views, |v| {
        !v.busy
            && !v.error.is_empty()
            && v.settings
                .as_ref()
                .is_some_and(|s| !s.persistence_error.is_empty())
    })
    .await;
    assert_eq!(failed.state.unwrap().selected.kind, "identity");
    std::fs::remove_dir(&blocked_save).unwrap();
    // Capture failure is safe on our private Wayland/memory-settings environment.
    send(&commands, &owner, Action::Capture(true)).await;
    wait(&mut views, |v| {
        !v.busy
            && v.state
                .as_ref()
                .is_some_and(|s| s.capture.status == "faulted")
            && !v.error.is_empty()
    })
    .await;
    send(&commands, &owner, Action::Capture(false)).await;
    wait(&mut views, |v| {
        !v.busy
            && v.error.is_empty()
            && v.state
                .as_ref()
                .is_some_and(|s| s.capture.status == "disabled")
    })
    .await;
    // External API calls reach the tray through signals as well as method replies.
    let proxy = zbus::Proxy::new(&connection, NAME, media_router_tray::protocol::PATH, NAME)
        .await
        .unwrap();
    let _: media_router_tray::protocol::Settings =
        proxy.call("SetAutoSelectNew", &(false,)).await.unwrap();
    wait(&mut views, |v| {
        v.settings.as_ref().is_some_and(|s| !s.auto_select_new)
    })
    .await;
    drop(daemon);
    wait(&mut views, |v| !v.connected()).await;
    let _replacement = sandbox.daemon();
    let restarted = wait(&mut views, |v| {
        v.connected() && v.owner.as_deref() != Some(&owner)
    })
    .await;
    assert_eq!(restarted.state.as_ref().unwrap().selected.kind, "identity");
    let replacement_owner = restarted.owner.unwrap();
    send(&commands, &owner, Action::Select(None)).await;
    let rejected = wait(&mut views, |v| v.error.contains("old daemon")).await;
    assert_eq!(rejected.state.unwrap().selected.kind, "identity");
    send(&commands, &replacement_owner, Action::Select(None)).await;
    wait(&mut views, |v| {
        !v.busy && v.state.as_ref().is_some_and(|s| s.selected.kind == "none") && v.error.is_empty()
    })
    .await;
    worker.abort();
    let _ = worker.await;
    assert!(sandbox.bus.0.id() > 0);
}
