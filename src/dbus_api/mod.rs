//! Version 1 session-bus API. Wire records are separate from selection state.
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
    #[zbus(error)]
    ZBus(zbus::Error),
}
pub enum Operation {
    Select(Option<ApplicationId>),
    Capture(bool),
}
pub struct Request {
    pub operation: Operation,
    pub reply: oneshot::Sender<Result<State, Error>>,
}
struct Service {
    state: watch::Receiver<State>,
    requests: mpsc::Sender<Request>,
}
impl Service {
    async fn request(&self, operation: Operation) -> Result<State, Error> {
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
        let id = match kind {
            "desktop-entry" if !value.is_empty() => ApplicationId::DesktopEntry(value.into()),
            "identity" => ApplicationId::Identity(value.into()),
            _ => {
                return Err(Error::InvalidArgument(
                    "expected desktop-entry with nonempty value, or identity".into(),
                ));
            }
        };
        self.request(Operation::Select(Some(id))).await
    }
    async fn clear_selection(&self) -> Result<State, Error> {
        self.request(Operation::Select(None)).await
    }
    async fn set_capture_enabled(&self, enabled: bool) -> Result<State, Error> {
        self.request(Operation::Capture(enabled)).await
    }
    #[zbus(signal)]
    async fn state_changed(emitter: &SignalEmitter<'_>, state: &State) -> zbus::Result<()>;
}

/// Owns the latest snapshot and its signal publisher. There is no history.
pub struct Server {
    state: watch::Sender<State>,
    publisher: tokio::task::JoinHandle<zbus::Result<()>>,
}
impl Server {
    pub async fn start(
        connection: &Connection,
        initial: State,
    ) -> zbus::Result<(Self, mpsc::Receiver<Request>)> {
        let (state, receiver) = watch::channel(initial);
        let (requests, inbox) = mpsc::channel(32);
        connection
            .object_server()
            .at(
                PATH,
                Service {
                    state: receiver.clone(),
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
        let publisher = tokio::spawn(async move {
            while receiver.changed().await.is_ok() {
                let snapshot = receiver.borrow_and_update().clone();
                // One in-flight signal plus the latest snapshot bounds memory;
                // intermediate revisions may coalesce for a slow bus.
                tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    Service::state_changed(&emitter, &snapshot),
                )
                .await
                .map_err(|_| zbus::Error::Failure("state signal timed out".into()))??;
            }
            Ok(())
        });
        Ok((Self { state, publisher }, inbox))
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
