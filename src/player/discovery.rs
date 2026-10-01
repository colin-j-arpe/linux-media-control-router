use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use tokio::task::{AbortHandle, JoinSet};
use zbus::{Connection, fdo::DBusProxy};

use super::{Player, mpris};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const RETRY_DELAYS_MS: [u64; 5] = [0, 100, 250, 500, 1000];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    Added(Player),
    Removed(Player),
    Skipped { service: String, reason: String },
}

/// Origin of an admission, retained through retries and delayed validation.
#[derive(Debug, Clone, Copy)]
pub struct DiscoveryOrigin {
    pub startup: bool,
    pub observed_at: Instant,
}

struct Candidate {
    origin: DiscoveryOrigin,
    owner: String,
    generation: u64,
    task: AbortHandle,
    player: Option<Player>,
}

struct ProbeResult {
    service: String,
    generation: u64,
    outcome: Result<Option<Player>, String>,
}

#[derive(Default)]
struct Inventory {
    candidates: HashMap<String, Candidate>,
    probes: JoinSet<ProbeResult>,
    generation: u64,
}

impl Inventory {
    fn reconcile(
        &mut self,
        connection: &Connection,
        service: &str,
        owner: Option<String>,
        emit: &mut impl FnMut(DiscoveryEvent, DiscoveryOrigin),
        startup: bool,
    ) {
        if self
            .candidates
            .get(service)
            .map(|candidate| candidate.owner.as_str())
            == owner.as_deref()
        {
            return;
        }
        if let Some(previous) = self.candidates.remove(service) {
            previous.task.abort();
            if let Some(player) = previous.player {
                emit(DiscoveryEvent::Removed(player), previous.origin);
            }
        }
        if let Some(owner) = owner {
            self.generation += 1;
            let generation = self.generation;
            let task_connection = connection.clone();
            let task_service = service.to_owned();
            let task_owner = owner.clone();
            let task = self.probes.spawn(async move {
                let outcome = probe(&task_connection, &task_service, &task_owner).await;
                ProbeResult {
                    service: task_service,
                    generation,
                    outcome,
                }
            });
            self.candidates.insert(
                service.to_owned(),
                Candidate {
                    origin: DiscoveryOrigin {
                        startup,
                        observed_at: Instant::now(),
                    },
                    owner,
                    generation,
                    task,
                    player: None,
                },
            );
        }
    }

    fn finish(
        &mut self,
        result: ProbeResult,
        emit: &mut impl FnMut(DiscoveryEvent, DiscoveryOrigin),
    ) {
        let Some(candidate) = self.candidates.get_mut(&result.service) else {
            return;
        };
        // A completed task may already have been queued when its owner disappeared.
        if candidate.generation != result.generation {
            return;
        }
        let reason = match result.outcome {
            Ok(Some(player)) => {
                candidate.player = Some(player.clone());
                emit(DiscoveryEvent::Added(player), candidate.origin);
                return;
            }
            Ok(None) => "CanControl is false".to_owned(),
            Err(error) => error,
        };
        emit(
            DiscoveryEvent::Skipped {
                service: result.service,
                reason,
            },
            candidate.origin,
        );
    }
}

/// Watch until the caller cancels this future, or the session bus fails.
///
/// The callback should return promptly. Properties are discovery-time snapshots;
/// live PropertiesChanged tracking and persistent selection are separate layers.
/// Dropping this future aborts all outstanding validation tasks.
pub async fn watch(
    connection: &Connection,
    mut emit: impl FnMut(DiscoveryEvent),
) -> zbus::Result<()> {
    watch_with_origin(connection, |event, _| emit(event)).await
}

/// Like watch, with appearance provenance for automatic-selection policy.
pub async fn watch_with_origin(
    connection: &Connection,
    mut emit: impl FnMut(DiscoveryEvent, DiscoveryOrigin),
) -> zbus::Result<()> {
    let bus = DBusProxy::new(connection).await?;
    // Subscribe first: services may appear while we enumerate the startup snapshot.
    let mut changes = bus.receive_name_owner_changed().await?;
    let mut inventory = Inventory::default();
    let mut names = bus.list_names().await?;
    names.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    for name in names
        .into_iter()
        .filter(|name| name.as_str().starts_with(PREFIX))
    {
        let owner = current_owner(&bus, name.as_str()).await?;
        inventory.reconcile(connection, name.as_str(), owner, &mut emit, true);
    }

    loop {
        tokio::select! {
            change = changes.next() => {
                let change = change.ok_or_else(|| zbus::Error::Failure("D-Bus ownership signal stream ended".into()))?;
                let args = change.args()?;
                let service = args.name().as_str();
                if service.starts_with(PREFIX) {
                    // Reconcile current reality rather than replay stale events queued
                    // during enumeration or rapid owner replacement.
                    let owner = current_owner(&bus, service).await?;
                    inventory.reconcile(connection, service, owner, &mut emit, false);
                }
            }
            result = inventory.probes.join_next(), if !inventory.probes.is_empty() => {
                match result.expect("nonempty JoinSet") {
                    Ok(result) => {
                        if inventory.candidates.get(&result.service)
                            .is_some_and(|candidate| candidate.generation == result.generation)
                        {
                            // Never admit a result belonging to a previous owner.
                            let owner = current_owner(&bus, &result.service).await?;
                            inventory.reconcile(connection, &result.service, owner, &mut emit, false);
                            inventory.finish(result, &mut emit);
                        }
                    }
                    Err(error) if error.is_cancelled() => {}
                    Err(error) => return Err(zbus::Error::Failure(format!("validation task failed: {error}"))),
                }
            }
        }
    }
}

async fn current_owner(bus: &DBusProxy<'_>, service: &str) -> zbus::Result<Option<String>> {
    match bus.get_name_owner(service.try_into()?).await {
        Ok(owner) => Ok(Some(owner.to_string())),
        Err(zbus::fdo::Error::NameHasNoOwner(_)) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn probe(
    connection: &Connection,
    service: &str,
    owner: &str,
) -> Result<Option<Player>, String> {
    let mut last_error = String::new();
    for delay in RETRY_DELAYS_MS {
        if delay != 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        match tokio::time::timeout(PROBE_TIMEOUT, mpris::inspect(connection, service, owner)).await
        {
            Ok(Ok(player)) => return Ok(player),
            Ok(Err(error)) => last_error = error,
            Err(_) => last_error = "player validation timed out".to_owned(),
        }
    }
    Err(format!(
        "validation failed after {} attempts: {last_error}",
        RETRY_DELAYS_MS.len()
    ))
}
