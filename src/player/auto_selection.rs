//! Most-recently-opened policy; no playback-driven selection.
use std::time::Instant;

use super::{
    discovery::{DiscoveryEvent, DiscoveryOrigin},
    selection::{ApplicationId, Selection},
};
use crate::config::Preferences;

#[derive(Default)]
pub struct AutoSelection {
    enabled_since: Option<Instant>,
}

impl AutoSelection {
    pub fn set_enabled(&mut self, enabled: bool) {
        self.set_enabled_at(enabled, Instant::now());
    }

    fn set_enabled_at(&mut self, enabled: bool, now: Instant) {
        match (self.enabled_since, enabled) {
            (None, true) => self.enabled_since = Some(now),
            (_, false) => self.enabled_since = None,
            _ => (),
        }
    }

    /// Evaluate before adding the player to Selection's registry.
    pub fn candidate(
        &self,
        selection: &Selection,
        preferences: &Preferences,
        event: &DiscoveryEvent,
        origin: DiscoveryOrigin,
    ) -> Option<ApplicationId> {
        let since = self.enabled_since?;
        let DiscoveryEvent::Added(player) = event else {
            return None;
        };
        if origin.startup || origin.observed_at < since || !player.capabilities.can_control {
            return None;
        }
        let id = ApplicationId::from(player);
        (!preferences.exclusions.contains(&id)
            && !selection.players().any(|(known, _)| known == &id))
        .then_some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::{Capabilities, PlaybackStatus, Player};
    use std::time::Duration;

    #[test]
    fn startup_and_pre_enable_admissions_are_not_new_arrivals() {
        let mut policy = AutoSelection::default();
        let before = Instant::now();
        let after = before + Duration::from_secs(1);
        let selection = Selection::default();
        let preferences = Preferences::default();
        let event = added("one", "music");
        assert!(
            policy
                .candidate(&selection, &preferences, &event, origin(after))
                .is_none()
        );
        policy.set_enabled_at(true, after);
        assert!(
            policy
                .candidate(&selection, &preferences, &event, origin(before))
                .is_none()
        );
        assert!(
            policy
                .candidate(
                    &selection,
                    &preferences,
                    &event,
                    DiscoveryOrigin {
                        startup: true,
                        observed_at: after
                    }
                )
                .is_none()
        );
        // An idempotent enable must not move the observation cutoff.
        policy.set_enabled_at(true, after + Duration::from_secs(1));
        assert!(
            policy
                .candidate(&selection, &preferences, &event, origin(after))
                .is_some()
        );
        policy.set_enabled_at(false, after);
        assert!(
            policy
                .candidate(&selection, &preferences, &event, origin(after))
                .is_none()
        );
    }

    #[test]
    fn only_first_eligible_instance_qualifies_and_exclusions_are_typed() {
        let now = Instant::now();
        let mut policy = AutoSelection::default();
        policy.set_enabled_at(true, now);
        let mut selection = Selection::default();
        let mut preferences = Preferences::default();
        let first = added("one", "music");
        preferences
            .exclusions
            .push(ApplicationId::Identity("music".into()));
        assert!(
            policy
                .candidate(&selection, &preferences, &first, origin(now))
                .is_some()
        );
        selection.apply_event(&first);
        let second = added("two", "music");
        assert!(
            policy
                .candidate(&selection, &preferences, &second, origin(now))
                .is_none()
        );
        let DiscoveryEvent::Added(player) = first else {
            unreachable!()
        };
        let removed = DiscoveryEvent::Removed(player);
        assert!(
            policy
                .candidate(&selection, &preferences, &removed, origin(now))
                .is_none()
        );
        selection.apply_event(&removed);
        assert!(
            policy
                .candidate(&selection, &preferences, &second, origin(now))
                .is_some()
        );
        preferences
            .exclusions
            .push(ApplicationId::DesktopEntry("music".into()));
        assert!(
            policy
                .candidate(&selection, &preferences, &second, origin(now))
                .is_none()
        );
    }

    fn origin(observed_at: Instant) -> DiscoveryOrigin {
        DiscoveryOrigin {
            startup: false,
            observed_at,
        }
    }
    fn added(service: &str, id: &str) -> DiscoveryEvent {
        DiscoveryEvent::Added(Player {
            service: service.into(),
            owner: ":1.1".into(),
            identity: id.into(),
            desktop_entry: Some(id.into()),
            playback_status: PlaybackStatus::Stopped,
            capabilities: Capabilities {
                can_control: true,
                can_play: true,
                can_pause: true,
                can_go_next: true,
                can_go_previous: true,
                can_seek: false,
            },
        })
    }
}
