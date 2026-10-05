//! Standalone, log-only Cinnamon D-Bus media-key compatibility probe.
//! Does not change settings or send playback commands.
use futures_util::StreamExt;
use gio::prelude::*;
use std::{error::Error, time::Duration};
use zbus::{Connection, Proxy, fdo::DBusProxy};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const SERVICE: &str = "org.gnome.SettingsDaemon";
const PATH: &str = "/org/gnome/SettingsDaemon/MediaKeys";
const INTERFACE: &str = "org.gnome.SettingsDaemon.MediaKeys";
const LOCKER: &str = "org.cinnamon.ScreenSaver";

fn arguments() -> Result<Option<(u64, bool)>> {
    let mut seconds = 600;
    let mut contender = false;
    let mut duration_seen = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "Usage: media_keys_probe [--seconds 1..3600] [--contender]\n\nLog Cinnamon media-key events, including while the screen is locked.\nTurn off Media Router capture first. No playback commands or settings writes.\nDefault lifetime: 600 seconds. Ctrl+C ends the probe.\nA second --contender process tests last-registered-listener priority."
                );
                return Ok(None);
            }
            "--seconds" if !duration_seen => {
                duration_seen = true;
                seconds = args.next().ok_or("--seconds needs a value")?.parse()?;
                if !(1..=3600).contains(&seconds) {
                    return Err("--seconds must be between 1 and 3600".into());
                }
            }
            "--contender" if !contender => contender = true,
            _ => return Err(format!("unknown or duplicate argument: {arg}; use --help").into()),
        }
    }
    Ok(Some((seconds, contender)))
}

fn check_settings() -> Result<()> {
    let source = gio::SettingsSchemaSource::default().ok_or("no GSettings schemas")?;
    let schema = source
        .lookup("org.cinnamon.desktop.keybindings.media-keys", true)
        .ok_or("Cinnamon media-key schema not installed")?;
    let settings = gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None);
    for key in ["play", "pause", "stop", "previous", "next"] {
        let bindings = settings.strv(key);
        println!("BINDING {key}: {bindings:?}");
        if bindings.is_empty() {
            return Err(format!("{key} has no Cinnamon binding; turn off Media Router capture before running this probe (custom empty bindings are left unchanged)").into());
        }
    }
    let schema = source
        .lookup("org.cinnamon.desktop.screensaver", true)
        .ok_or("Cinnamon screensaver schema not installed")?;
    let settings = gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None);
    if !settings.boolean("allow-keyboard-shortcuts") {
        return Err("Cinnamon lock-screen keyboard shortcuts are disabled; enable them in lock-screen settings before testing locked delivery".into());
    }
    Ok(())
}

// No owner means no snapshot, not a failed media-key registration. Avoid
// activating or keeping the screensaver alive merely to annotate key events.
async fn screen_snapshot(connection: &Connection) -> Result<Option<bool>> {
    let bus = DBusProxy::new(connection).await?;
    let owner = match bus.get_name_owner(LOCKER.try_into()?).await {
        Ok(owner) => owner,
        Err(zbus::fdo::Error::NameHasNoOwner(_)) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let locker = Proxy::new(
        connection,
        owner.as_str(),
        "/org/cinnamon/ScreenSaver",
        LOCKER,
    )
    .await?;
    match locker.call("GetActive", &()).await {
        Ok(active) => Ok(Some(active)),
        Err(error) => {
            // Owner may disappear between lookup and query; key delivery remains useful.
            eprintln!("SCREEN snapshot unavailable: {error}");
            Ok(None)
        }
    }
}

async fn listen(connection: &Connection, app: &str, proxy: &Proxy<'_>, seconds: u64) -> Result<()> {
    // Subscribe before registering, so even the first event is observable.
    let mut keys = proxy.receive_signal("MediaPlayerKeyPressed").await?;
    let bus = DBusProxy::new(connection).await?;
    let mut owners = bus.receive_name_owner_changed().await?;
    // zbus tracks the well-known owner's changes for signal filtering. The
    // screensaver normally exits while idle, so it must not own our lifetime.
    let locker = Proxy::new(connection, LOCKER, "/org/cinnamon/ScreenSaver", LOCKER).await?;
    let mut active_changes = locker.receive_signal("ActiveChanged").await?;
    let mut active = screen_snapshot(connection).await?;
    let owner = bus.get_name_owner(SERVICE.try_into()?).await?;
    if proxy.destination().as_str() != owner.as_str() {
        return Err("media-key service changed during startup; restart the probe".into());
    }
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let started = tokio::time::Instant::now();
    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);
    proxy
        .call::<_, _, ()>("GrabMediaPlayerKeys", &(app, 0u32))
        .await?;
    println!("PROBE READY app={app}; screen_active={active:?}; lifetime={seconds}s; LOG ONLY");
    println!(
        "Press Play/Pause, Stop, Previous, Next; lock manually and repeat, then unlock and repeat. Volume/mute retain desktop handling."
    );
    loop {
        tokio::select! {
            _ = interrupt.recv() => break,
            _ = terminate.recv() => break,
            _ = &mut deadline => { println!("PROBE lifetime expired"); break; }
            event = keys.next() => {
                let event = event.ok_or("media-key signal stream ended")?;
                let (target, key): (String, String) = event.body().deserialize()?;
                if target == app {
                    let action = match key.as_str() {
                        "Play" | "Pause" => "PLAY_PAUSE",
                        "Stop" => "STOP",
                        "Previous" => "PREVIOUS",
                        "Next" => "NEXT",
                        _ => "OUTSIDE_TRANSPORT_SCOPE",
                    };
                    println!("KEY +{:.3}s app={app} screen_active={active:?} key={key:?} action={action}", started.elapsed().as_secs_f64());
                }
            }
            event = active_changes.next() => {
                let event = event.ok_or("lock-screen monitoring stream ended")?;
                active = Some(event.body().deserialize::<bool>()?);
                // Active includes an unlocked screensaver; it is not proof of authentication lock.
                println!("SCREEN +{:.3}s active={active:?}", started.elapsed().as_secs_f64());
            }
            event = owners.next() => {
                let event = event.ok_or("bus owner signal stream ended")?;
                let args = event.args()?;
                let name = args.name().as_str();
                if name == SERVICE || name == LOCKER {
                    println!("OWNER +{:.3}s service={name} old={:?} new={:?}",
                        started.elapsed().as_secs_f64(), args.old_owner(), args.new_owner());
                }
                if name == SERVICE && args.new_owner().as_ref().map(|value| value.as_str()) != Some(owner.as_str()) {
                    return Err(format!("media-key service {SERVICE} ownership changed; restart the probe").into());
                }
                if name == LOCKER {
                    // Re-read current ownership instead of trusting a buffered transition.
                    active = screen_snapshot(connection).await?;
                    println!("SCREEN +{:.3}s active={active:?} (owner change)", started.elapsed().as_secs_f64());
                }
            }
        }
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let Some((seconds, contender)) = arguments()? else {
        return Ok(());
    };
    check_settings()?;
    let connection = zbus::connection::Builder::session()?
        .method_timeout(Duration::from_secs(3))
        .build()
        .await?;
    let (name, app) = if contender {
        (
            "org.mediarouter.MediaKeysContender1",
            "media-router-media-keys-contender",
        )
    } else {
        (
            "org.mediarouter.MediaKeysProbe1",
            "media-router-media-keys-probe",
        )
    };
    connection
        .request_name_with_flags(name, zbus::fdo::RequestNameFlags::DoNotQueue.into())
        .await?;
    let bus = DBusProxy::new(&connection).await?;
    let owner = bus.get_name_owner(SERVICE.try_into()?).await?;
    let proxy = Proxy::new(&connection, owner.as_str(), PATH, INTERFACE).await?;
    let outcome = listen(&connection, app, &proxy, seconds).await;
    // Also release after errors: a timed-out registration may still have applied.
    let cleanup = proxy
        .call::<_, _, ()>("ReleaseMediaPlayerKeys", &(app,))
        .await;
    if let Err(error) = cleanup {
        eprintln!("PROBE release failed: {error}; disconnecting removes this listener");
    } else {
        println!("PROBE RELEASED app={app}");
    }
    // Cinnamon watches the unique connection owner and removes it on disconnect.
    connection.close().await?;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        process::{Child, Command, Stdio},
        sync::Arc,
    };
    use tokio::sync::Notify;

    struct Bus(Child);
    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    struct Keys(Arc<Notify>);
    #[zbus::interface(name = "org.gnome.SettingsDaemon.MediaKeys")]
    impl Keys {
        fn grab_media_player_keys(&self, _application: &str, _time: u32) {
            self.0.notify_one();
        }
    }
    struct Screensaver;
    #[zbus::interface(name = "org.cinnamon.ScreenSaver")]
    impl Screensaver {
        fn get_active(&self) -> bool {
            false
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn listener_survives_missing_disappearing_and_returning_screensaver() {
        let mut bus = Bus(Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap());
        let mut address = String::new();
        BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim();
        let registered = Arc::new(Notify::new());
        let service = zbus::connection::Builder::address(address)
            .unwrap()
            .name(SERVICE)
            .unwrap()
            .serve_at(PATH, Keys(registered.clone()))
            .unwrap()
            .build()
            .await
            .unwrap();
        let screen = zbus::connection::Builder::address(address)
            .unwrap()
            .serve_at("/org/cinnamon/ScreenSaver", Screensaver)
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = zbus::connection::Builder::address(address)
            .unwrap()
            .build()
            .await
            .unwrap();
        assert_eq!(screen_snapshot(&client).await.unwrap(), None);
        let owner = service.unique_name().unwrap();
        let proxy = Proxy::new(&client, owner.as_str(), PATH, INTERFACE)
            .await
            .unwrap();
        let scenario = async {
            registered.notified().await;
            // Absent at startup, appears, vanishes, then returns under a new owner.
            screen.request_name(LOCKER).await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(screen_snapshot(&client).await.unwrap(), Some(false));
            screen.release_name(LOCKER).await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(screen_snapshot(&client).await.unwrap(), None);
            let replacement = zbus::connection::Builder::address(address)
                .unwrap()
                .name(LOCKER)
                .unwrap()
                .serve_at("/org/cinnamon/ScreenSaver", Screensaver)
                .unwrap()
                .build()
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(screen_snapshot(&client).await.unwrap(), Some(false));
            replacement.release_name(LOCKER).await.unwrap();
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(listen(&client, "test-listener", &proxy, 1), scenario)
        })
        .await
        .expect("listener failed to finish");
        result.expect("screensaver lifecycle must not terminate media-key listener");
    }
}
