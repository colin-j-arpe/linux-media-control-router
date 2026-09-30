use std::{collections::BTreeMap, fmt};

use super::{Player, discovery::DiscoveryEvent};

/// Logical application identity, independent of any running D-Bus instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationId {
    DesktopEntry(String),
    Identity(String),
}

impl From<&Player> for ApplicationId {
    fn from(player: &Player) -> Self {
        match player
            .desktop_entry
            .as_deref()
            .filter(|entry| !entry.is_empty())
        {
            Some(entry) => Self::DesktopEntry(entry.to_owned()),
            None => Self::Identity(player.identity.clone()),
        }
    }
}

impl fmt::Display for ApplicationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DesktopEntry(value) => write!(formatter, "desktop-entry {value:?}"),
            Self::Identity(value) => write!(formatter, "identity {value:?}"),
        }
    }
}

/// Both parts are needed: a service name can be reused by a new process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceId {
    pub service: String,
    pub owner: String,
}

impl From<&Player> for InstanceId {
    fn from(player: &Player) -> Self {
        Self {
            service: player.service.clone(),
            owner: player.owner.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionState {
    Unselected,
    Unavailable {
        application: ApplicationId,
    },
    Available {
        application: ApplicationId,
        instance: InstanceId,
    },
}

struct RegisteredPlayer {
    application: ApplicationId,
    player: Player,
}

/// Retains user intent independently of the currently available instances.
///
/// The caller supplies validated players and their logical identities. This
/// module does not infer identity from a service name or perform any D-Bus I/O.
#[derive(Default)]
pub struct Selection {
    selected: Option<ApplicationId>,
    active: Option<InstanceId>,
    players: BTreeMap<String, RegisteredPlayer>,
}

impl Selection {
    /// Apply a validated discovery event and report only selection-state changes.
    pub fn apply_event(&mut self, event: &DiscoveryEvent) -> Option<SelectionState> {
        let before = self.state();
        match event {
            DiscoveryEvent::Added(player) => self.add(ApplicationId::from(player), player.clone()),
            DiscoveryEvent::Removed(player) => self.remove(&player.service, &player.owner),
            DiscoveryEvent::Skipped { .. } => return None,
        }
        let after = self.state();
        (before != after).then_some(after)
    }

    /// Validated controllable instances, ordered by service name.
    pub fn players(&self) -> impl Iterator<Item = (&ApplicationId, &Player)> {
        self.players
            .values()
            .filter(|entry| entry.player.capabilities.can_control)
            .map(|entry| (&entry.application, &entry.player))
    }

    pub fn selected_application(&self) -> Option<&ApplicationId> {
        self.selected.as_ref()
    }

    pub fn active_player(&self) -> Option<&Player> {
        let active = self.active.as_ref()?;
        self.players
            .get(&active.service)
            .filter(|registered| registered.player.owner == active.owner)
            .map(|registered| &registered.player)
    }

    pub fn state(&self) -> SelectionState {
        match (&self.selected, &self.active) {
            (Some(application), Some(instance)) => SelectionState::Available {
                application: application.clone(),
                instance: instance.clone(),
            },
            (Some(application), None) => SelectionState::Unavailable {
                application: application.clone(),
            },
            (None, _) => SelectionState::Unselected,
        }
    }

    /// Select an application even when it is absent, or pass None to clear it.
    /// Selecting the same application again preserves its current instance.
    pub fn select(&mut self, application: Option<ApplicationId>) {
        if self.selected != application {
            self.selected = application;
            self.active = None;
        }
        self.resolve();
    }

    pub fn add(&mut self, application: ApplicationId, player: Player) {
        self.players.insert(
            player.service.clone(),
            RegisteredPlayer {
                application,
                player,
            },
        );
        self.resolve();
    }

    /// Ignore a delayed removal belonging to an older owner of the same name.
    pub fn remove(&mut self, service: &str, owner: &str) {
        if self
            .players
            .get(service)
            .is_some_and(|registered| registered.player.owner == owner)
        {
            self.players.remove(service);
            self.resolve();
        }
    }

    fn resolve(&mut self) {
        let Some(selected) = &self.selected else {
            self.active = None;
            return;
        };
        let eligible = |registered: &&RegisteredPlayer| {
            &registered.application == selected && registered.player.capabilities.can_control
        };

        if let Some(active) = &self.active
            && self
                .players
                .get(&active.service)
                .filter(eligible)
                .is_some_and(|registered| registered.player.owner == active.owner)
        {
            return;
        }

        // BTreeMap visits service names in order, so replacement is deterministic
        // among the currently known eligible instances of the selected application.
        self.active = self
            .players
            .values()
            .find(eligible)
            .map(|registered| InstanceId::from(&registered.player));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::{Capabilities, PlaybackStatus};

    fn application(name: &str) -> ApplicationId {
        ApplicationId::DesktopEntry(name.to_owned())
    }

    fn player(service: &str, owner: &str) -> Player {
        Player {
            service: service.to_owned(),
            owner: owner.to_owned(),
            identity: "Test Player".to_owned(),
            desktop_entry: None,
            playback_status: PlaybackStatus::Paused,
            capabilities: Capabilities {
                can_control: true,
                can_play: true,
                can_pause: true,
                can_go_next: true,
                can_go_previous: true,
                can_seek: false,
            },
        }
    }

    #[test]
    fn selection_survives_absence_and_resolves_to_a_new_instance() {
        let mut selection = Selection::default();
        selection.select(Some(application("spotify")));
        let unavailable = SelectionState::Unavailable {
            application: application("spotify"),
        };
        assert_eq!(selection.state(), unavailable);

        selection.add(application("firefox"), player("firefox", ":1.1"));
        assert_eq!(selection.state(), unavailable);
        selection.add(application("spotify"), player("spotify.old", ":1.2"));
        assert_eq!(selection.active_player().unwrap().owner, ":1.2");
        selection.remove("spotify.old", ":1.2");
        assert_eq!(selection.state(), unavailable);
        assert_eq!(
            selection.selected_application(),
            Some(&application("spotify"))
        );
        selection.add(application("spotify"), player("spotify.new", ":1.3"));
        assert_eq!(selection.active_player().unwrap().service, "spotify.new");
    }

    #[test]
    fn retains_active_instance_then_uses_service_order_among_remaining_matches() {
        let mut selection = Selection::default();
        selection.select(Some(application("browser")));
        selection.add(application("browser"), player("z", ":1.1"));
        selection.add(application("browser"), player("b", ":1.2"));
        selection.add(application("browser"), player("a", ":1.3"));
        selection.add(application("other"), player("0", ":1.4"));
        assert_eq!(selection.active_player().unwrap().service, "z");
        selection.select(Some(application("browser")));
        assert_eq!(selection.active_player().unwrap().service, "z");
        selection.remove("z", ":1.1");
        assert_eq!(selection.active_player().unwrap().service, "a");
        selection.remove("a", ":1.3");
        assert_eq!(selection.active_player().unwrap().service, "b");
        selection.remove("b", ":1.2");
        assert!(matches!(
            selection.state(),
            SelectionState::Unavailable { .. }
        ));
    }

    #[test]
    fn selection_after_discovery_uses_service_order_and_can_be_changed_or_cleared() {
        let mut selection = Selection::default();
        selection.add(application("browser"), player("z", ":1.1"));
        selection.add(application("browser"), player("a", ":1.2"));
        selection.add(application("music"), player("music", ":1.3"));
        assert_eq!(selection.state(), SelectionState::Unselected);
        selection.select(Some(application("browser")));
        assert_eq!(selection.active_player().unwrap().service, "a");
        selection.select(Some(application("music")));
        assert_eq!(selection.active_player().unwrap().service, "music");
        selection.select(None);
        assert_eq!(selection.state(), SelectionState::Unselected);
        selection.add(application("music"), player("music2", ":1.4"));
        assert_eq!(selection.state(), SelectionState::Unselected);
    }

    #[test]
    fn old_owner_removal_cannot_remove_a_replacement() {
        let mut selection = Selection::default();
        selection.select(Some(application("music")));
        selection.add(application("music"), player("same.service", ":1.1"));
        selection.add(application("music"), player("same.service", ":1.2"));
        selection.remove("same.service", ":1.1");
        assert_eq!(selection.active_player().unwrap().owner, ":1.2");
        selection.remove("same.service", ":1.2");
        assert!(selection.active_player().is_none());
        assert_eq!(
            selection.selected_application(),
            Some(&application("music"))
        );
    }

    #[test]
    fn ineligible_or_differently_identified_replacements_cannot_remain_active() {
        let mut selection = Selection::default();
        selection.select(Some(application("music")));
        selection.add(application("music"), player("same.service", ":1.1"));
        let mut disabled = player("same.service", ":1.1");
        disabled.capabilities.can_control = false;
        selection.add(application("music"), disabled);
        assert!(selection.active_player().is_none());
        selection.add(application("other"), player("same.service", ":1.2"));
        assert!(selection.active_player().is_none());
        assert_eq!(
            selection.selected_application(),
            Some(&application("music"))
        );
    }

    #[test]
    fn desktop_entry_takes_precedence_and_cannot_collide_with_fallback_identity() {
        let mut selection = Selection::default();
        selection.select(Some(ApplicationId::Identity("Test Player".into())));
        let mut desktop = player("desktop", ":1.1");
        desktop.desktop_entry = Some("Test Player".into());
        assert_eq!(ApplicationId::from(&desktop), application("Test Player"));
        assert_eq!(selection.apply_event(&DiscoveryEvent::Added(desktop)), None);
        assert!(selection.active_player().is_none());
        let fallback = player("fallback", ":1.2");
        assert!(matches!(
            selection.apply_event(&DiscoveryEvent::Added(fallback)),
            Some(SelectionState::Available {
                application: ApplicationId::Identity(_),
                ..
            })
        ));
        assert_eq!(selection.active_player().unwrap().service, "fallback");
    }

    #[test]
    fn fallback_identity_matches_exactly_and_survives_service_name_changes() {
        let mut selection = Selection::default();
        let selected = ApplicationId::Identity("My Player".into());
        selection.select(Some(selected.clone()));
        for (index, identity) in ["my player", "My Player ", "Other Player"]
            .into_iter()
            .enumerate()
        {
            let mut other = player(&format!("other{index}"), ":1.1");
            other.identity = identity.into();
            assert_eq!(selection.apply_event(&DiscoveryEvent::Added(other)), None);
        }
        let mut first = player("first.service", ":1.2");
        first.identity = "My Player".into();
        first.desktop_entry = Some(String::new());
        assert_eq!(ApplicationId::from(&first), selected);
        assert!(matches!(
            selection.apply_event(&DiscoveryEvent::Added(first.clone())),
            Some(SelectionState::Available { .. })
        ));
        assert_eq!(
            selection.apply_event(&DiscoveryEvent::Added(first.clone())),
            None
        );
        assert!(matches!(
            selection.apply_event(&DiscoveryEvent::Removed(first)),
            Some(SelectionState::Unavailable { .. })
        ));
        let mut returned = player("new.service", ":1.3");
        returned.identity = "My Player".into();
        selection.apply_event(&DiscoveryEvent::Added(returned));
        assert_eq!(selection.active_player().unwrap().service, "new.service");
        assert_eq!(selection.selected_application(), Some(&selected));
    }

    #[test]
    fn equal_fallback_identities_group_instances_and_skipped_events_do_not_change_selection() {
        let mut selection = Selection::default();
        selection.select(Some(ApplicationId::Identity("Test Player".into())));
        let first = player("first", ":1.1");
        selection.apply_event(&DiscoveryEvent::Added(first.clone()));
        assert_eq!(
            selection.apply_event(&DiscoveryEvent::Added(player("second", ":1.2"))),
            None
        );
        let before = selection.state();
        assert_eq!(
            selection.apply_event(&DiscoveryEvent::Skipped {
                service: "third".into(),
                reason: "invalid".into(),
            }),
            None
        );
        assert_eq!(selection.state(), before);
        selection.apply_event(&DiscoveryEvent::Removed(first));
        assert_eq!(selection.active_player().unwrap().service, "second");
    }
}
