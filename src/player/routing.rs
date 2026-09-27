use std::{fmt, str::FromStr, time::Duration};

use tokio::sync::watch;
use zbus::{Connection, Proxy, fdo::DBusProxy, proxy::CacheProperties};

use super::{mpris::PATH, selection::SelectionState};

const INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
const TIMEOUT: Duration = Duration::from_secs(2);

/// Input-independent transport actions. Volume and mute are intentionally absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportAction {
    PlayPause,
    Stop,
    Previous,
    Next,
}

impl TransportAction {
    fn method(self) -> &'static str {
        match self {
            Self::PlayPause => "PlayPause",
            Self::Stop => "Stop",
            Self::Previous => "Previous",
            Self::Next => "Next",
        }
    }
}

impl FromStr for TransportAction {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "play-pause" => Ok(Self::PlayPause),
            "stop" => Ok(Self::Stop),
            "previous" => Ok(Self::Previous),
            "next" => Ok(Self::Next),
            _ => Err("expected play-pause, stop, previous, or next"),
        }
    }
}

impl fmt::Display for TransportAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PlayPause => "play-pause",
            Self::Stop => "stop",
            Self::Previous => "previous",
            Self::Next => "next",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    NoSelection,
    Unavailable,
    SelectionChanged,
    OwnerChanged,
    Unsupported(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteOutcome {
    /// The method returned successfully; this does not prove playback changed.
    Acknowledged,
    Skipped(SkipReason),
    /// No transport method was attempted.
    NotSent(String),
    /// The player returned a D-Bus error in response to the transport method.
    PlayerError(String),
    /// A method was attempted, but no definitive reply was received. Never retry.
    Uncertain(String),
}

enum PreparationError {
    Skip(SkipReason),
    Query(zbus::Error),
    InvalidStatus(String),
}

impl From<zbus::Error> for PreparationError {
    fn from(error: zbus::Error) -> Self {
        Self::Query(error)
    }
}

async fn owner_matches(
    bus: &DBusProxy<'_>,
    service: &str,
    owner: &str,
) -> Result<bool, PreparationError> {
    match bus
        .get_name_owner(service.try_into().map_err(zbus::Error::from)?)
        .await
    {
        Ok(current) => Ok(current.as_str() == owner),
        Err(zbus::fdo::Error::NameHasNoOwner(_)) => Ok(false),
        Err(error) => Err(PreparationError::Query(error.into())),
    }
}

async fn require(proxy: &Proxy<'_>, property: &'static str) -> Result<(), PreparationError> {
    if proxy.get_property::<bool>(property).await? {
        Ok(())
    } else {
        Err(PreparationError::Skip(SkipReason::Unsupported(property)))
    }
}

async fn prepare<'a>(
    connection: &'a Connection,
    service: &str,
    owner: &str,
    action: TransportAction,
) -> Result<Proxy<'a>, PreparationError> {
    let bus = DBusProxy::new(connection).await?;
    if !owner_matches(&bus, service, owner).await? {
        return Err(PreparationError::Skip(SkipReason::OwnerChanged));
    }
    let proxy = zbus::proxy::Builder::new(connection)
        .destination(owner.to_owned())?
        .path(PATH)?
        .interface(INTERFACE)?
        .cache_properties(CacheProperties::No)
        .build()
        .await?;

    require(&proxy, "CanControl").await?;
    match action {
        TransportAction::Next => require(&proxy, "CanGoNext").await?,
        TransportAction::Previous => require(&proxy, "CanGoPrevious").await?,
        TransportAction::Stop => {} // MPRIS has no CanStop property.
        TransportAction::PlayPause => {
            // The PlayPause method explicitly requires CanPause. Starting or
            // resuming also needs CanPlay, as defined by that property's contract.
            require(&proxy, "CanPause").await?;
            match proxy
                .get_property::<String>("PlaybackStatus")
                .await?
                .as_str()
            {
                "Playing" => {}
                "Paused" | "Stopped" => require(&proxy, "CanPlay").await?,
                status => return Err(PreparationError::InvalidStatus(status.to_owned())),
            }
        }
    }
    // A player may still exist after losing its well-known name. Check again
    // after the property calls, before sending to its unique owner.
    if !owner_matches(&bus, service, owner).await? {
        return Err(PreparationError::Skip(SkipReason::OwnerChanged));
    }
    Ok(proxy)
}

/// Dispatch once to the instance captured for this input. Never retarget/retry.
///
/// `current` tracks selection changes while asynchronous checks run. Property
/// reads are uncached, and neither checks nor the method wait indefinitely.
pub async fn dispatch(
    connection: &Connection,
    current: &watch::Receiver<SelectionState>,
    target: &SelectionState,
    action: TransportAction,
) -> RouteOutcome {
    let instance = match target {
        SelectionState::Unselected => return RouteOutcome::Skipped(SkipReason::NoSelection),
        SelectionState::Unavailable { .. } => {
            return RouteOutcome::Skipped(SkipReason::Unavailable);
        }
        SelectionState::Available { instance, .. } => instance,
    };
    if *current.borrow() != *target {
        return RouteOutcome::Skipped(SkipReason::SelectionChanged);
    }
    let proxy = match tokio::time::timeout(
        TIMEOUT,
        prepare(connection, &instance.service, &instance.owner, action),
    )
    .await
    {
        Ok(Ok(proxy)) => proxy,
        Ok(Err(PreparationError::Skip(reason))) => return RouteOutcome::Skipped(reason),
        Ok(Err(PreparationError::Query(error))) => return RouteOutcome::NotSent(error.to_string()),
        Ok(Err(PreparationError::InvalidStatus(status))) => {
            return RouteOutcome::NotSent(format!("invalid PlaybackStatus: {status:?}"));
        }
        Err(_) => return RouteOutcome::NotSent("dispatch checks timed out".into()),
    };
    if *current.borrow() != *target {
        return RouteOutcome::Skipped(SkipReason::SelectionChanged);
    }
    match tokio::time::timeout(TIMEOUT, proxy.call::<_, _, ()>(action.method(), &())).await {
        Ok(Ok(())) => RouteOutcome::Acknowledged,
        Ok(Err(error @ zbus::Error::MethodError(..))) => {
            RouteOutcome::PlayerError(error.to_string())
        }
        Ok(Err(error)) => RouteOutcome::Uncertain(error.to_string()),
        Err(_) => RouteOutcome::Uncertain(
            "method reply timed out; execution may already have occurred".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::TransportAction;

    #[test]
    fn input_rejects_commands_outside_transport_scope() {
        for input in [
            "volume-up",
            "volume-down",
            "mute",
            "play",
            "pause",
            "Next",
            "",
        ] {
            assert!(
                input.parse::<TransportAction>().is_err(),
                "accepted {input:?}"
            );
        }
    }
}
