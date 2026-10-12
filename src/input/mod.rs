//! Replaceable Cinnamon input backends with shared legacy binding recovery.
mod cinnamon_dbus;
mod settings;
mod x11;

use crate::player::routing::TransportAction;
use settings::{Cinnamon, Lease, state_directory};
use std::time::Duration;

type Error = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, Error>;

struct X11Capture {
    keyboard: Option<x11::Keyboard>,
    lease: Lease<Cinnamon>,
}
impl X11Capture {
    /// Starts legacy X11 capture after recovering and journaling Cinnamon shortcuts.
    ///
    /// # Returns
    ///
    /// Returns a capture handle holding the keyboard connection and settings lease.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported sessions, recovery, keyboard setup, shortcut changes, or
    /// failed grabs; startup failures attempt restoration.
    pub async fn start() -> Result<Self> {
        if std::env::var("XDG_SESSION_TYPE").as_deref() != Ok("x11")
            || !std::env::var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .split(':')
                .any(|s| s.eq_ignore_ascii_case("cinnamon") || s.eq_ignore_ascii_case("x-cinnamon"))
        {
            return Err("--capture requires a Cinnamon X11 session".into());
        }
        let lease = Lease::open(Cinnamon::new()?, state_directory()?)?;
        lease.restore()?;
        let keyboard = x11::Keyboard::connect()?;
        let mut capture = Self {
            keyboard: Some(keyboard),
            lease,
        };
        let result = async {
            capture.lease.acquire()?;
            // Cinnamon releases its old grabs asynchronously after the settings
            // notification. Retry acquisition briefly; never retry media commands.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            loop {
                match capture
                    .keyboard
                    .as_ref()
                    .expect("keyboard connected")
                    .grab()
                {
                    Ok(()) => return Ok::<(), Error>(()),
                    Err(error) if tokio::time::Instant::now() >= deadline => {
                        return Err(format!("cannot acquire transport keys: {error}").into());
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                }
                if !capture.lease.unchanged()? {
                    return Err("Cinnamon shortcuts changed during capture startup".into());
                }
            }
        }
        .await;
        if let Err(error) = result {
            return match capture.stop() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(format!(
                    "{error}; restoration failed: {cleanup}; run --restore-bindings"
                )
                .into()),
            };
        }
        Ok(capture)
    }
    /// Waits for a legacy X11 transport action while monitoring shortcut changes.
    ///
    /// # Returns
    ///
    /// Returns the next hardware transport action.
    ///
    /// # Errors
    ///
    /// Returns an error if shortcuts changed externally or keyboard monitoring fails.
    ///
    /// # Panics
    ///
    /// Panics if called after the keyboard connection has been stopped.
    pub async fn next(&mut self) -> Result<TransportAction> {
        loop {
            if !self.lease.unchanged()? {
                return Err("Cinnamon shortcuts changed externally; stopping capture and preserving your edits".into());
            }
            tokio::select! {
                action = self.keyboard.as_mut().expect("capture is active").next() => return action,
                // Dispatch GLib settings notifications even when no key is pressed.
                _ = tokio::time::sleep(Duration::from_millis(100)) => (),
            }
        }
    }
    /// Disconnects the legacy keyboard capture before restoring Cinnamon shortcuts.
    ///
    /// # Returns
    ///
    /// Returns Ok(()) after binding restoration, including when no journal remains.
    ///
    /// # Errors
    ///
    /// Returns a recovery error if saved shortcuts cannot be restored.
    pub fn stop(&mut self) -> Result<()> {
        // Disconnect first: Cinnamon must be able to reclaim the keys.
        self.keyboard.take();
        self.lease.restore()
    }
}
impl Drop for X11Capture {
    /// Attempts legacy capture cleanup when its handle is dropped.
    ///
    /// Reports restoration failures to standard error with the binding-recovery command.
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("BINDING RECOVERY REQUIRED: {error}; run media-router --restore-bindings");
        }
    }
}

/// Restores pending Cinnamon bindings under the recovery transaction lock.
///
/// # Returns
///
/// Returns Ok(()) after recovery, including when no journal exists.
///
/// # Errors
///
/// Returns an error for settings setup, state-path resolution, lock contention, or restoration
/// failure.
pub fn restore_bindings() -> Result<()> {
    Lease::open(Cinnamon::new()?, state_directory()?)?.restore()
}

/// Recovers pending legacy bindings before applying serving capture intent.
///
/// Run even when desired capture is off. With no journal, observation-only serving needs no
/// Cinnamon schema or backend.
///
/// # Returns
///
/// Returns Ok(()) when recovery succeeds or no journal exists.
///
/// # Errors
///
/// Returns an error for state-path, filesystem, settings, locking, or restoration failures.
pub fn recover_pending_bindings() -> Result<()> {
    let directory = state_directory()?;
    match std::fs::symlink_metadata(directory.join("bindings.json")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => Lease::open(Cinnamon::new()?, directory)?.restore(),
    }
}

/// Backend selection is a process option, independent of saved capture intent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Backend {
    #[default]
    CinnamonDbus,
    X11,
}
pub enum Event {
    Action(TransportAction),
    Reclaim,
}
pub struct Capture {
    backend: Active,
}
enum Active {
    Cinnamon(cinnamon_dbus::Capture),
    X11(Box<X11Capture>),
}
impl Capture {
    /// Starts the selected media-key capture backend.
    ///
    /// # Arguments
    ///
    /// * `backend` — Process-selected Cinnamon D-Bus or legacy X11 backend.
    ///
    /// # Returns
    ///
    /// Returns an active capture handle.
    ///
    /// # Errors
    ///
    /// Returns the backend startup error, including session, settings, recovery, or connection
    /// failures.
    pub async fn start(backend: Backend) -> Result<Self> {
        Ok(Self {
            backend: match backend {
                Backend::CinnamonDbus => Active::Cinnamon(cinnamon_dbus::Capture::start().await?),
                Backend::X11 => Active::X11(Box::new(X11Capture::start().await?)),
            },
        })
    }
    /// Waits for the next event from the active input backend.
    ///
    /// # Returns
    ///
    /// Returns a transport action or a backend reclaim request.
    ///
    /// # Errors
    ///
    /// Returns the backend monitoring error.
    ///
    /// # Panics
    ///
    /// Panics for the legacy X11 backend if called after stop.
    pub async fn next(&mut self) -> Result<Event> {
        match &mut self.backend {
            Active::Cinnamon(capture) => capture.next().await,
            Active::X11(capture) => capture.next().await.map(Event::Action),
        }
    }
    /// Refreshes backend listener priority when applicable.
    ///
    /// # Returns
    ///
    /// Returns Ok(()) after Cinnamon registration, or immediately for X11 whose grabs remain owned.
    ///
    /// # Errors
    ///
    /// Returns the Cinnamon backend settings, ownership, or registration error.
    pub async fn reclaim(&mut self) -> Result<()> {
        match &mut self.backend {
            Active::Cinnamon(capture) => capture.reclaim().await,
            Active::X11(_) => Ok(()), // X11 grabs remain owned while active.
        }
    }
    /// Stops the active backend and performs its cleanup.
    ///
    /// # Returns
    ///
    /// Returns Ok(()) when backend cleanup succeeds.
    ///
    /// # Errors
    ///
    /// Returns a backend release, disconnection, or binding-restoration error.
    pub async fn stop(&mut self) -> Result<()> {
        match &mut self.backend {
            Active::Cinnamon(capture) => capture.stop().await,
            Active::X11(capture) => capture.stop(),
        }
    }
}
