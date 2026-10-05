//! Version 1 session-bus API. Wire records are separate from selection state.
use crate::config::Preferences;
use crate::player::selection::{ApplicationId, Selection, SelectionState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::sync::{mpsc, oneshot, watch};
use zbus::{Connection, object_server::SignalEmitter, zvariant::Type};

pub const NAME: &str = "org.mediarouter.MediaRouter1";
pub const PATH: &str = "/org/mediarouter/MediaRouter1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Application {
    pub kind: String,
    pub value: String,
    pub name: String,
    pub instances: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Selected {
    /// "none", "desktop-entry", or "identity". Empty identity is valid.
    pub kind: String,
    pub value: String,
    pub available: bool,
    pub service: String,
    pub owner: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CaptureState {
    /// "disabled", "starting", "enabled", or "faulted".
    pub status: String,
    pub error: String,
}
impl Default for CaptureState {
    fn default() -> Self {
        Self {
            status: "disabled".into(),
            error: String::new(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct State {
    pub revision: u64,
    pub applications: Vec<Application>,
    pub selected: Selected,
    pub capture: CaptureState,
}
/// Exact logical identity, without inventory or availability fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Identity {
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Settings {
    pub revision: u64,
    pub capture_enabled: bool,
    pub auto_select_new: bool,
    pub exclusions: Vec<Identity>,
    pub persistence_error: String,
}
impl Settings {
    pub fn snapshot(preferences: &Preferences, persistence_error: String) -> Self {
        Self {
            revision: 0,
            capture_enabled: preferences.capture_enabled,
            auto_select_new: preferences.auto_select_new,
            exclusions: preferences
                .exclusions
                .iter()
                .map(|id| {
                    let (kind, value) = identity(id);
                    Identity {
                        kind: kind.into(),
                        value: value.into(),
                    }
                })
                .collect(),
            persistence_error,
        }
    }
}
fn parse_identity(kind: &str, value: &str) -> Result<ApplicationId, Error> {
    match kind {
        "desktop-entry" if !value.is_empty() => Ok(ApplicationId::DesktopEntry(value.into())),
        "identity" => Ok(ApplicationId::Identity(value.into())),
        _ => Err(Error::InvalidArgument(
            "expected desktop-entry with nonempty value, or identity".into(),
        )),
    }
}
fn identity(id: &ApplicationId) -> (&str, &str) {
    match id {
        ApplicationId::DesktopEntry(value) => ("desktop-entry", value),
        ApplicationId::Identity(value) => ("identity", value),
    }
}
impl State {
    pub fn snapshot(selection: &Selection, capture: &CaptureState) -> Self {
        let mut applications = BTreeMap::<(String, String), Application>::new();
        // The selection registry iterates by service name, also making the
        // display label deterministic when instances supply different names.
        for (id, player) in selection.players() {
            let (kind, value) = identity(id);
            let entry = applications
                .entry((kind.into(), value.into()))
                .or_insert_with(|| Application {
                    kind: kind.into(),
                    value: value.into(),
                    name: player.identity.clone(),
                    instances: 0,
                });
            entry.instances += 1;
        }
        let mut selected = Selected {
            kind: "none".into(),
            value: String::new(),
            available: false,
            service: String::new(),
            owner: String::new(),
        };
        if let Some(id) = selection.selected_application() {
            let (kind, value) = identity(id);
            selected.kind = kind.into();
            selected.value = value.into();
        }
        if let SelectionState::Available { instance, .. } = selection.state() {
            selected.available = true;
            selected.service = instance.service;
            selected.owner = instance.owner;
        }
        Self {
            revision: 0,
            applications: applications.into_values().collect(),
            selected,
            capture: capture.clone(),
        }
    }
}
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.mediarouter.MediaRouter1.Error")]
pub enum Error {
    InvalidArgument(String),
    Busy(String),
    Unavailable(String),
    CaptureFailed(String),
    PersistenceFailed(String),
    #[zbus(error)]
    ZBus(zbus::Error),
}
pub enum Operation {
    Select(Option<ApplicationId>),
    Capture(bool),
    Reclaim,
    AutoSelectNew(bool),
    Exclusions(Vec<ApplicationId>),
}
pub struct Response {
    pub state: State,
    pub settings: Settings,
}
pub struct Request {
    pub operation: Operation,
    pub reply: oneshot::Sender<Result<Response, Error>>,
}
struct Service {
    state: watch::Receiver<State>,
    settings: watch::Receiver<Settings>,
    requests: mpsc::Sender<Request>,
}
impl Service {
    async fn request(&self, operation: Operation) -> Result<Response, Error> {
        let (reply, receiver) = oneshot::channel();
        self.requests
            .try_send(Request { operation, reply })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    Error::Busy("router request queue is full".into())
                }
                mpsc::error::TrySendError::Closed(_) => {
                    Error::Unavailable("router is stopping".into())
                }
            })?;
        receiver
            .await
            .map_err(|_| Error::Unavailable("router stopped before replying".into()))?
    }
}
#[zbus::interface(name = "org.mediarouter.MediaRouter1")]
impl Service {
    fn get_state(&self) -> State {
        self.state.borrow().clone()
    }
    async fn select_application(&self, kind: &str, value: &str) -> Result<State, Error> {
        let id = parse_identity(kind, value)?;
        self.request(Operation::Select(Some(id)))
            .await
            .map(|reply| reply.state)
    }
    async fn clear_selection(&self) -> Result<State, Error> {
        self.request(Operation::Select(None))
            .await
            .map(|reply| reply.state)
    }
    async fn set_capture_enabled(&self, enabled: bool) -> Result<State, Error> {
        self.request(Operation::Capture(enabled))
            .await
            .map(|reply| reply.state)
    }
    async fn reclaim_media_keys(&self) -> Result<State, Error> {
        self.request(Operation::Reclaim)
            .await
            .map(|reply| reply.state)
    }
    fn get_settings(&self) -> Settings {
        self.settings.borrow().clone()
    }
    async fn set_auto_select_new(&self, enabled: bool) -> Result<Settings, Error> {
        self.request(Operation::AutoSelectNew(enabled))
            .await
            .map(|reply| reply.settings)
    }
    async fn set_auto_select_exclusions(
        &self,
        mut exclusions: Vec<Identity>,
    ) -> Result<Settings, Error> {
        exclusions.sort_by(|a, b| (&a.kind, &a.value).cmp(&(&b.kind, &b.value)));
        exclusions.dedup();
        let ids = exclusions
            .iter()
            .map(|id| parse_identity(&id.kind, &id.value))
            .collect::<Result<Vec<_>, _>>()?;
        self.request(Operation::Exclusions(ids))
            .await
            .map(|reply| reply.settings)
    }
    #[zbus(signal)]
    async fn settings_changed(emitter: &SignalEmitter<'_>, settings: &Settings)
    -> zbus::Result<()>;
    #[zbus(signal)]
    async fn state_changed(emitter: &SignalEmitter<'_>, state: &State) -> zbus::Result<()>;
}

/// Owns the latest snapshot and its signal publisher. There is no history.
pub struct Server {
    state: watch::Sender<State>,
    settings: watch::Sender<Settings>,
    publisher: tokio::task::JoinHandle<zbus::Result<()>>,
}
impl Server {
    pub async fn start(
        connection: &Connection,
        initial: State,
        initial_settings: Settings,
    ) -> zbus::Result<(Self, mpsc::Receiver<Request>)> {
        let (state, receiver) = watch::channel(initial);
        let (settings, settings_receiver) = watch::channel(initial_settings);
        let (requests, inbox) = mpsc::channel(32);
        connection
            .object_server()
            .at(
                PATH,
                Service {
                    state: receiver.clone(),
                    settings: settings_receiver.clone(),
                    requests,
                },
            )
            .await?;
        // Do not replace an existing daemon, and never queue behind it.
        connection
            .request_name_with_flags(NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into())
            .await?;
        let emitter = SignalEmitter::new(connection, PATH)?.into_owned();
        let mut receiver = receiver;
        let mut settings_receiver = settings_receiver;
        let publisher = tokio::spawn(async move {
            loop {
                let send = async {
                    tokio::select! {
                        changed = receiver.changed() => {
                            changed.map_err(|e| zbus::Error::Failure(e.to_string()))?;
                            let snapshot = receiver.borrow_and_update().clone();
                            tokio::time::timeout(std::time::Duration::from_secs(2), Service::state_changed(&emitter, &snapshot)).await.map_err(|_| zbus::Error::Failure("state signal timed out".into()))?
                        }
                        changed = settings_receiver.changed() => {
                            changed.map_err(|e| zbus::Error::Failure(e.to_string()))?;
                            let snapshot = settings_receiver.borrow_and_update().clone();
                            tokio::time::timeout(std::time::Duration::from_secs(2), Service::settings_changed(&emitter, &snapshot)).await.map_err(|_| zbus::Error::Failure("settings signal timed out".into()))?
                        }
                    }
                };
                // Wait for a change without a timeout, then bound only its send.
                send.await?;
            }
        });
        Ok((
            Self {
                state,
                settings,
                publisher,
            },
            inbox,
        ))
    }
    pub fn settings(&self) -> Settings {
        self.settings.borrow().clone()
    }
    pub fn publish_settings(&self, mut next: Settings) -> Settings {
        let current = self.settings.borrow();
        next.revision = current.revision;
        if next == *current {
            return next;
        }
        next.revision = current
            .revision
            .checked_add(1)
            .expect("settings revision exhausted");
        drop(current);
        self.settings.send_replace(next.clone());
        next
    }
    pub fn publish(&self, mut next: State) -> State {
        let current = self.state.borrow();
        next.revision = current.revision;
        if next == *current {
            return next;
        }
        next.revision = current
            .revision
            .checked_add(1)
            .expect("state revision exhausted");
        drop(current);
        self.state.send_replace(next.clone());
        next
    }
    pub async fn stopped(&mut self) -> zbus::Result<()> {
        (&mut self.publisher)
            .await
            .map_err(|error| zbus::Error::Failure(error.to_string()))?
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.publisher.abort();
    }
}
