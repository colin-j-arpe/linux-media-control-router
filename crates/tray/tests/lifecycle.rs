//! Exercise the executable with a fake tray host, without any desktop access.
use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::sync::mpsc;
use zbus::zvariant::{OwnedValue, Value};
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Host(mpsc::Sender<String>);
#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Host {
    async fn register_status_notifier_item(&self, service: &str) {
        let _ = self.0.send(service.into()).await;
    }
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }
}
async fn exit(process: &mut Process) -> std::process::ExitStatus {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn no_host_wait_for_late_host_duplicate_instance_and_menu_quit() {
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
    let spawn = |wait_for_host| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_media-router-tray"));
        if wait_for_host {
            command.arg("--wait-for-host");
        }
        Process(
            command
                .env("DBUS_SESSION_BUS_ADDRESS", address.trim())
                .stderr(if wait_for_host {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .spawn()
                .unwrap(),
        )
    };
    let mut no_host = spawn(false);
    assert!(!exit(&mut no_host).await.success());
    let mut tray = spawn(true);
    let observer = zbus::connection::Builder::address(address.trim())
        .unwrap()
        .build()
        .await
        .unwrap();
    let dbus = zbus::fdo::DBusProxy::new(&observer).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if dbus
                .name_has_owner("org.mediarouter.Tray1".try_into().unwrap())
                .await
                .unwrap()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let stderr = tray.0.stderr.take().unwrap();
    let (stderr_sender, stderr_receiver) = tokio::sync::oneshot::channel();
    let stderr_task = tokio::task::spawn_blocking(move || {
        let mut sender = Some(stderr_sender);
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else {
                break;
            };
            if let Some(sender) = sender.take() {
                let _ = sender.send(line);
            }
        }
    });
    let line = tokio::time::timeout(Duration::from_secs(5), stderr_receiver)
        .await
        .unwrap()
        .unwrap();
    assert!(line.contains("Waiting for the desktop tray host"), "{line}");
    assert!(tray.0.try_wait().unwrap().is_none());
    let (sender, mut registrations) = mpsc::channel(2);
    let mut host = zbus::connection::Builder::address(address.trim())
        .unwrap()
        .name("org.kde.StatusNotifierWatcher")
        .unwrap()
        .serve_at("/StatusNotifierWatcher", Host(sender.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    let name = tokio::time::timeout(Duration::from_secs(5), registrations.recv())
        .await
        .unwrap()
        .unwrap();
    let mut duplicate = spawn(false);
    assert!(!exit(&mut duplicate).await.success());
    host.close().await.unwrap();
    host = zbus::connection::Builder::address(address.trim())
        .unwrap()
        .name("org.kde.StatusNotifierWatcher")
        .unwrap()
        .serve_at("/StatusNotifierWatcher", Host(sender))
        .unwrap()
        .build()
        .await
        .unwrap();
    let returned_name = tokio::time::timeout(Duration::from_secs(5), registrations.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(returned_name, name);
    assert!(tray.0.try_wait().unwrap().is_none());
    let menu = zbus::Proxy::new(&host, name.as_str(), "/MenuBar", "com.canonical.dbusmenu")
        .await
        .unwrap();
    // Wait for the unavailable-daemon snapshot so the menu's item IDs are stable.
    let quit = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);
            let (_, root): (u32, Layout) = menu
                .call("GetLayout", &(0_i32, 1_i32, vec!["label"]))
                .await
                .unwrap();
            let items: Vec<Layout> = root
                .2
                .into_iter()
                .map(|child| child.try_into().unwrap())
                .collect();
            let labels: Vec<_> = items
                .iter()
                .filter_map(|(id, props, _)| {
                    props
                        .get("label")
                        .and_then(|v| <&str>::try_from(v).ok())
                        .map(|label| (*id, label))
                })
                .collect();
            if labels
                .iter()
                .any(|(_, label)| label.contains("org.freedesktop.DBus.Error.NameHasNoOwner"))
            {
                break labels
                    .iter()
                    .find(|(_, label)| label.starts_with("Quit tray"))
                    .unwrap()
                    .0;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let _: () = menu
        .call("Event", &(quit, "clicked", Value::from(0_i32), 0_u32))
        .await
        .unwrap();
    assert!(exit(&mut tray).await.success());
    stderr_task.await.unwrap();
}
#[test]
fn help_and_invalid_arguments_do_not_need_a_bus() {
    let command = |arg| {
        Command::new(env!("CARGO_BIN_EXE_media-router-tray"))
            .arg(arg)
            .env("DBUS_SESSION_BUS_ADDRESS", "invalid:")
            .output()
            .unwrap()
    };
    assert!(command("--help").status.success());
    assert!(!command("--unknown").status.success());
}
