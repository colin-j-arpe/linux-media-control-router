pub mod discovery;
mod mpris;
pub mod routing;
pub mod selection;

/// One validated, running MPRIS instance. This is not a persistent selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    pub service: String,
    pub owner: String,
    pub identity: String,
    pub desktop_entry: Option<String>,
    pub playback_status: PlaybackStatus,
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub can_control: bool,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
    pub can_seek: bool,
}
