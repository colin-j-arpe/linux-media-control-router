//! Cinnamon/X11 capture with a durable, independently recoverable settings lease.
mod settings;
mod x11;

use crate::player::routing::TransportAction;
use settings::{Cinnamon, Lease, state_directory};
use std::time::Duration;

type Error = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, Error>;

pub struct Capture {
    keyboard: Option<x11::Keyboard>,
    lease: Lease<Cinnamon>,
}
impl Capture {
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
    pub fn stop(&mut self) -> Result<()> {
        // Disconnect first: Cinnamon must be able to reclaim the keys.
        self.keyboard.take();
        self.lease.restore()
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("BINDING RECOVERY REQUIRED: {error}; run media-router --restore-bindings");
        }
    }
}

pub fn restore_bindings() -> Result<()> {
    Lease::open(Cinnamon::new()?, state_directory()?)?.restore()
}
