//! Client-side records matching docs/org.mediarouter.MediaRouter1.xml.
//! Intentionally independent of daemon implementation types.
use serde::{Deserialize, Serialize};
use zbus::zvariant::Type;

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
