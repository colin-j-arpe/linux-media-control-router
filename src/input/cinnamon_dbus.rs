//! Cinnamon media-key registration. Listener priority is best effort.
use super::{
    Event, Result,
    settings::{BINDINGS, Cinnamon, Lease, Settings, state_directory},
};
use crate::player::routing::TransportAction;
use futures_util::StreamExt;
use std::time::Duration;
use tokio::{sync::mpsc, task::JoinHandle};
use zbus::{Connection, Proxy, fdo::DBusProxy};

const SERVICE: &str = "org.gnome.SettingsDaemon";
const PATH: &str = "/org/gnome/SettingsDaemon/MediaKeys";
const INTERFACE: &str = "org.gnome.SettingsDaemon.MediaKeys";
const APPLICATION: &str = "media-router";
const LOCKER: &str = "org.cinnamon.ScreenSaver";

pub(super) struct Capture {
    connection: Connection,
    owner: String,
    events: mpsc::Receiver<std::result::Result<Event, String>>,
    worker: JoinHandle<()>,
    _lease: Lease<Cinnamon>,
    settings: Cinnamon,
    stopped: bool,
}
impl Capture {
    pub(super) async fn start() -> Result<Self> {
        if std::env::var("XDG_SESSION_TYPE").as_deref() != Ok("x11")
            || !std::env::var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .split(':')
                .any(|s| s.eq_ignore_ascii_case("cinnamon") || s.eq_ignore_ascii_case("x-cinnamon"))
        {
            return Err("capture requires a Cinnamon X11 session".into());
        }
        let settings = Cinnamon::new()?;
        let lease = Lease::open(Cinnamon::new()?, state_directory()?)?;
        lease.restore()?; // Recover old X11 capture journals before registering.
        validate_settings(&settings)?;
        let connection = zbus::connection::Builder::session()?
            .method_timeout(Duration::from_secs(2))
            .build()
            .await?;
        connection
            .request_name_with_flags(
                "org.mediarouter.Capture1",
                zbus::fdo::RequestNameFlags::DoNotQueue.into(),
            )
            .await?;
        let bus = DBusProxy::new(&connection).await?;
        let owner = bus.get_name_owner(SERVICE.try_into()?).await?.to_string();
        let proxy = Proxy::new(&connection, owner.as_str(), PATH, INTERFACE).await?;
        let mut keys = proxy.receive_signal("MediaPlayerKeyPressed").await?;
        let mut owners = bus
            .receive_name_owner_changed_with_args(&[(0, SERVICE)])
            .await?;
        // Well-known-name signal streams follow screensaver exits/restarts.
        // Do not activate the screensaver just to observe it.
        let locker = Proxy::new(&connection, LOCKER, "/org/cinnamon/ScreenSaver", LOCKER).await?;
        let mut locks = locker.receive_signal("ActiveChanged").await?;
        let (sender, events) = mpsc::channel(32);
        let worker_owner = owner.clone();
        let worker = tokio::spawn(async move {
            let result: std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> = async {
                loop {
                    let event = tokio::select! {
                        signal = keys.next() => {
                            let signal = signal.ok_or("Cinnamon media-key signal stream ended")?;
                            let (application, key): (String, String) = signal.body().deserialize()?;
                            if application != APPLICATION { continue; }
                            let action = match key.as_str() {
                                "Play" | "Pause" => TransportAction::PlayPause,
                                "Stop" => TransportAction::Stop,
                                "Previous" => TransportAction::Previous,
                                "Next" => TransportAction::Next,
                                _ => continue, // Volume/mute and extra transport actions are untouched.
                            };
                            Event::Action(action)
                        }
                        signal = locks.next() => {
                            let signal = signal.ok_or("Cinnamon screen-state signal stream ended")?;
                            if !signal.body().deserialize::<bool>()? { continue; }
                            Event::Reclaim
                        }
                        signal = owners.next() => {
                            let signal = signal.ok_or("Cinnamon service owner stream ended")?;
                            if signal.args()?.new_owner().as_ref().map(|name| name.as_str()) == Some(worker_owner.as_str()) { continue; }
                            return Err("Cinnamon media-key service owner changed; retry enabling capture".into());
                        }
                    };
                    // Bounded input buffer: discard overflow instead of replaying later.
                    match sender.try_send(Ok(event)) {
                        Ok(()) => (),
                        Err(mpsc::error::TrySendError::Closed(_)) => return Ok(()),
                        Err(mpsc::error::TrySendError::Full(_)) => eprintln!("CAPTURE event discarded: input queue full"),
                    }
                }
            }.await;
            if let Err(error) = result {
                let _ = sender.send(Err(error.to_string())).await;
            }
        });
        let mut capture = Self {
            connection,
            owner,
            events,
            worker,
            _lease: lease,
            settings,
            stopped: false,
        };
        if let Err(error) = capture.reclaim().await {
            let _ = capture.stop().await;
            return Err(error);
        }
        Ok(capture)
    }
    pub(super) async fn reclaim(&mut self) -> Result<()> {
        validate_settings(&self.settings)?;
        let bus = DBusProxy::new(&self.connection).await?;
        if bus.get_name_owner(SERVICE.try_into()?).await?.as_str() != self.owner {
            return Err("Cinnamon media-key service owner changed; retry enabling capture".into());
        }
        let proxy = Proxy::new(&self.connection, self.owner.as_str(), PATH, INTERFACE).await?;
        proxy
            .call::<_, _, ()>("GrabMediaPlayerKeys", &(APPLICATION, 0u32))
            .await?;
        Ok(())
    }
    pub(super) async fn next(&mut self) -> Result<Event> {
        loop {
            validate_settings(&self.settings)?;
            tokio::select! {
                event = self.events.recv() => return event.ok_or("Cinnamon input worker stopped")?.map_err(Into::into),
                _ = tokio::time::sleep(Duration::from_millis(100)) => (),
            }
        }
    }
    pub(super) async fn stop(&mut self) -> Result<()> {
        if self.stopped {
            return Ok(());
        }
        self.worker.abort();
        let proxy = Proxy::new(&self.connection, self.owner.as_str(), PATH, INTERFACE).await?;
        let release = proxy
            .call::<_, _, ()>("ReleaseMediaPlayerKeys", &(APPLICATION,))
            .await;
        // Even a failed release is cleaned up by owner disappearance.
        let close = self.connection.clone().close().await;
        self.stopped = true;
        close?;
        if let Err(error) = release {
            eprintln!("CAPTURE release reply failed; connection closed: {error}");
        }
        Ok(())
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

fn validate_settings(settings: &Cinnamon) -> Result<()> {
    for (key, expected) in BINDINGS {
        let values = settings.read(key)?.effective;
        if values.is_empty() || values.iter().any(|value| value != expected) {
            return Err(format!("Cinnamon {key} must contain only {expected} for D-Bus capture; custom/disabled transport shortcuts are preserved. Use --input-backend x11 for legacy capture").into());
        }
    }
    Ok(())
}
