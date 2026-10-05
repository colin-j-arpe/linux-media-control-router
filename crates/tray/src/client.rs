//! Owner-pinned D-Bus synchronization. No MPRIS, config files, or input backend.
use crate::{
    model::{Action, Command, View},
    protocol::{NAME, PATH, Settings, State},
};
use futures_util::StreamExt;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use zbus::{Connection, Proxy};

async fn owner(bus: &Proxy<'_>) -> zbus::Result<String> {
    bus.call("GetNameOwner", &(NAME,)).await
}

/// Runs until cancelled by the executable. A missing/restarted daemon is recoverable.
/// Session-bus disconnection remains visible as an error; no mutation is replayed.
pub async fn run(
    connection: Connection,
    mut commands: mpsc::Receiver<Command>,
    views: watch::Sender<View>,
) {
    loop {
        let result = session(&connection, &mut commands, &views).await;
        let error = result
            .err()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "Daemon owner changed".into());
        views.send_replace(View {
            error,
            ..Default::default()
        });
        // Drain stale actions while disconnected. Never queue for a future owner.
        let retry = tokio::time::sleep(Duration::from_secs(1));
        tokio::pin!(retry);
        loop {
            tokio::select! {
                _ = &mut retry => break,
                command = commands.recv() => if command.is_none() { return; },
            }
        }
    }
}
async fn session(
    connection: &Connection,
    commands: &mut mpsc::Receiver<Command>,
    views: &watch::Sender<View>,
) -> zbus::Result<()> {
    let bus = Proxy::new(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await?;
    let mut owners = bus
        .receive_signal_with_args("NameOwnerChanged", &[(0, NAME)])
        .await?;
    let current = owner(&bus).await?;
    let proxy = Proxy::new(connection, current.as_str(), PATH, NAME).await?;
    let mut states = proxy.receive_signal("StateChanged").await?;
    let mut settings = proxy.receive_signal("SettingsChanged").await?;
    let mut view = View {
        owner: Some(current.clone()),
        ..Default::default()
    };
    refresh(&proxy, &mut view).await?;
    if owner(&bus).await? != current {
        return Ok(());
    }
    views.send_replace(view.clone());
    loop {
        tokio::select! {
            _ = owners.next() => return Ok(()),
            message = states.next() => {
                let message = message.ok_or_else(|| zbus::Error::Failure("State stream closed".into()))?;
                view.apply_state(message.body().deserialize()?);
            }
            message = settings.next() => {
                let message = message.ok_or_else(|| zbus::Error::Failure("Settings stream closed".into()))?;
                view.apply_settings(message.body().deserialize()?);
            }
            command = commands.recv() => {
                let Some(command) = command else { return Ok(()); };
                if command.owner != current || owner(&bus).await? != current {
                    view.error = "Discarded action for an old daemon; choose again".into();
                } else {
                    view.busy = true;
                    views.send_replace(view.clone());
                    view.error = match mutate(&proxy, command.action, &mut view).await {
                        Ok(()) => String::new(),
                        Err(error) => format!("Request failed (not retried): {error}"),
                    };
                    // Even an error/timeout may follow an applied mutation.
                    refresh(&proxy, &mut view).await?;
                    view.busy = false;
                }
            }
        }
        views.send_replace(view.clone());
    }
}
async fn refresh(proxy: &Proxy<'_>, view: &mut View) -> zbus::Result<()> {
    view.apply_state(proxy.call("GetState", &()).await?);
    view.apply_settings(proxy.call("GetSettings", &()).await?);
    Ok(())
}
async fn mutate(proxy: &Proxy<'_>, action: Action, view: &mut View) -> zbus::Result<()> {
    match action {
        Action::Select(Some(id)) => {
            let state: State = proxy
                .call("SelectApplication", &(id.kind, id.value))
                .await?;
            view.apply_state(state);
        }
        Action::Select(None) => view.apply_state(proxy.call("ClearSelection", &()).await?),
        Action::Capture(enabled) => {
            view.apply_state(proxy.call("SetCaptureEnabled", &(enabled,)).await?)
        }
        Action::Reclaim => view.apply_state(proxy.call("ReclaimMediaKeys", &()).await?),
        Action::AutoSelect(enabled) => {
            view.apply_settings(proxy.call("SetAutoSelectNew", &(enabled,)).await?)
        }
        Action::Exclude(id, excluded) => {
            // API v1 replaces the whole list. Read immediately before writing to
            // preserve absent players and changes from other clients where possible.
            let mut settings: Settings = proxy.call("GetSettings", &()).await?;
            settings.exclusions.retain(|entry| entry != &id);
            if excluded {
                settings.exclusions.push(id);
            }
            view.apply_settings(
                proxy
                    .call("SetAutoSelectExclusions", &(settings.exclusions,))
                    .await?,
            );
        }
    }
    Ok(())
}
