//! Snapshot handling and exact identities, independent of tray widgets.
use crate::protocol::{Identity, Settings, State};

#[derive(Clone, Debug, Default)]
pub struct View {
    pub owner: Option<String>,
    pub state: Option<State>,
    pub settings: Option<Settings>,
    pub error: String,
    pub busy: bool,
}
impl View {
    pub fn connected(&self) -> bool {
        self.owner.is_some() && self.state.is_some() && self.settings.is_some()
    }
    pub fn apply_state(&mut self, state: State) {
        if self
            .state
            .as_ref()
            .is_none_or(|old| state.revision > old.revision)
        {
            self.state = Some(state);
        }
    }
    pub fn apply_settings(&mut self, settings: Settings) {
        if self
            .settings
            .as_ref()
            .is_none_or(|old| settings.revision > old.revision)
        {
            self.settings = Some(settings);
        }
    }
    pub fn warning(&self) -> bool {
        !self.connected()
            || !self.error.is_empty()
            || self.state.as_ref().is_some_and(|s| {
                (s.selected.kind != "none" && !s.selected.available)
                    || !matches!(
                        s.capture.status.as_str(),
                        "enabled" | "disabled" | "starting"
                    )
            })
            || self
                .settings
                .as_ref()
                .is_some_and(|s| !s.persistence_error.is_empty())
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    Select(Option<Identity>),
    Capture(bool),
    AutoSelect(bool),
    Exclude(Identity, bool),
}
#[derive(Clone, Debug)]
pub struct Command {
    /// Never replay an action against a replacement daemon.
    pub owner: String,
    pub action: Action,
}

/// Escape menu mnemonics and control characters without changing wire identities.
pub fn label(text: &str) -> String {
    if text.is_empty() {
        return "(empty identity)".into();
    }
    text.chars()
        .flat_map(|c| {
            if c == '_' {
                "__".chars().collect::<Vec<_>>()
            } else if c.is_control() {
                vec![' ']
            } else {
                vec![c]
            }
        })
        .collect()
}
pub fn identity_label(id: &Identity) -> String {
    format!("{} [{}]", label(&id.value), id.kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings(revision: u64, enabled: bool) -> Settings {
        Settings {
            revision,
            capture_enabled: false,
            auto_select_new: enabled,
            exclusions: vec![],
            persistence_error: String::new(),
        }
    }
    #[test]
    fn stale_or_equal_revisions_cannot_roll_back_settings() {
        let mut view = View::default();
        view.apply_settings(settings(8, true));
        view.apply_settings(settings(7, false));
        view.apply_settings(settings(8, false));
        assert!(view.settings.unwrap().auto_select_new);
    }
    #[test]
    fn new_owner_starts_with_fresh_revision_space() {
        let mut view = View {
            owner: Some(":1.1".into()),
            ..Default::default()
        };
        view.apply_settings(settings(99, true));
        view = View {
            owner: Some(":1.2".into()),
            ..Default::default()
        };
        view.apply_settings(settings(0, false));
        assert_eq!(view.settings.unwrap().revision, 0);
    }
    #[test]
    fn display_escaping_preserves_distinct_empty_and_unicode_labels() {
        assert_eq!(label(""), "(empty identity)");
        assert_eq!(label("A_B\n音楽"), "A__B 音楽");
    }
}
