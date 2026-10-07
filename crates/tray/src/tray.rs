//! Host-rendered menu. Callbacks enqueue API requests; they never mutate daemon state locally.
use crate::{
    model::{Action, Command, View, identity_label, label},
    protocol::Identity,
};
use ksni::menu::*;
use std::sync::Arc;
use tokio::sync::{Notify, mpsc};

pub struct Tray {
    pub view: View,
    pub commands: mpsc::Sender<Command>,
    pub quit: Arc<Notify>,
    pub wait_for_host: bool,
}
impl Tray {
    fn send(&mut self, action: Action) {
        let Some(owner) = self.view.owner.clone() else {
            return;
        };
        if self.view.busy {
            return;
        }
        match self.commands.try_send(Command { owner, action }) {
            Ok(()) => self.view.busy = true,
            Err(_) => self.view.error = "Request queue unavailable; try again".into(),
        }
    }
    fn status(&self) -> String {
        let Some(state) = &self.view.state else {
            return "Daemon unavailable — start media-router --serve".into();
        };
        if state.selected.kind == "none" {
            return "No application selected".into();
        }
        let id = Identity {
            kind: state.selected.kind.clone(),
            value: state.selected.value.clone(),
        };
        format!(
            "Selected: {}{}",
            identity_label(&id),
            if state.selected.available {
                ""
            } else {
                " — unavailable"
            }
        )
    }
}
fn text(value: impl Into<String>) -> MenuItem<Tray> {
    StandardItem {
        label: value.into(),
        enabled: false,
        ..Default::default()
    }
    .into()
}
impl ksni::Tray for Tray {
    // Optional host convenience; Cinnamon currently opens on right-click only.
    const MENU_ON_ACTIVATE: bool = true;
    fn id(&self) -> String {
        "media-router-tray".into()
    }
    fn title(&self) -> String {
        "Media Router".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [24, 32, 48]
            .into_iter()
            .map(|size| crate::icon::icon(size, self.view.warning()))
            .collect()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.title(),
            description: self.status(),
            ..Default::default()
        }
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let enabled = self.view.connected() && !self.view.busy;
        let mut menu = vec![text("Media Router"), text(self.status())];
        if self.view.busy {
            menu.push(text("Applying request…"));
        }
        if !self.view.error.is_empty() {
            menu.push(text(label(&self.view.error)));
        }
        if let Some(state) = &self.view.state {
            menu.push(text(format!(
                "Media-key capture: {}",
                label(&state.capture.status)
            )));
            if !state.capture.error.is_empty() {
                menu.push(text(label(&state.capture.error)));
            }
            let mut identities = vec![None];
            let mut options = vec![RadioItem {
                label: "No selection".into(),
                enabled,
                ..Default::default()
            }];
            for app in &state.applications {
                let id = Identity {
                    kind: app.kind.clone(),
                    value: app.value.clone(),
                };
                options.push(RadioItem {
                    label: format!(
                        "{} — {} ({} instance{})",
                        label(&app.name),
                        identity_label(&id),
                        app.instances,
                        if app.instances == 1 { "" } else { "s" }
                    ),
                    enabled,
                    ..Default::default()
                });
                identities.push(Some(id));
            }
            let selected = if state.selected.kind == "none" {
                None
            } else {
                Some(Identity {
                    kind: state.selected.kind.clone(),
                    value: state.selected.value.clone(),
                })
            };
            if !identities.contains(&selected) {
                options.push(RadioItem {
                    label: format!(
                        "{} — unavailable",
                        identity_label(selected.as_ref().unwrap())
                    ),
                    enabled,
                    ..Default::default()
                });
                identities.push(selected.clone());
            }
            let index = identities
                .iter()
                .position(|id| id == &selected)
                .unwrap_or(0);
            menu.push(MenuItem::Separator);
            menu.push(
                RadioGroup {
                    selected: index,
                    options,
                    select: Box::new(move |tray: &mut Self, index| {
                        if let Some(id) = identities.get(index) {
                            tray.send(Action::Select(id.clone()));
                        }
                    }),
                }
                .into(),
            );
        }
        if let Some(settings) = &self.view.settings {
            menu.push(MenuItem::Separator);
            if !settings.persistence_error.is_empty() {
                menu.push(text(format!(
                    "Preferences: {}",
                    label(&settings.persistence_error)
                )));
            }
            let capture = !settings.capture_enabled;
            menu.push(
                CheckmarkItem {
                    label: "Capture media keys".into(),
                    checked: settings.capture_enabled,
                    enabled,
                    activate: Box::new(move |tray: &mut Self| tray.send(Action::Capture(capture))),
                    ..Default::default()
                }
                .into(),
            );
            if self
                .view
                .state
                .as_ref()
                .is_some_and(|s| s.capture.status == "faulted")
            {
                menu.push(
                    StandardItem {
                        label: "Retry enabling capture".into(),
                        enabled,
                        activate: Box::new(|tray: &mut Self| tray.send(Action::Capture(true))),
                        ..Default::default()
                    }
                    .into(),
                );
                menu.push(
                    StandardItem {
                        label: "Retry disabling / restoring bindings".into(),
                        enabled,
                        activate: Box::new(|tray: &mut Self| tray.send(Action::Capture(false))),
                        ..Default::default()
                    }
                    .into(),
                );
            }
            menu.push(
                StandardItem {
                    label: "Reclaim media keys".into(),
                    enabled: enabled
                        && self
                            .view
                            .state
                            .as_ref()
                            .is_some_and(|s| s.capture.status == "enabled"),
                    activate: Box::new(|tray: &mut Self| tray.send(Action::Reclaim)),
                    ..Default::default()
                }
                .into(),
            );
            let auto = !settings.auto_select_new;
            menu.push(
                CheckmarkItem {
                    label: "Auto-select newly opened applications (MRO)".into(),
                    checked: settings.auto_select_new,
                    enabled,
                    activate: Box::new(move |tray: &mut Self| tray.send(Action::AutoSelect(auto))),
                    ..Default::default()
                }
                .into(),
            );
            // Keep absent exclusions visible so users can remove them.
            let mut ids = settings.exclusions.clone();
            if let Some(state) = &self.view.state {
                ids.extend(state.applications.iter().map(|app| Identity {
                    kind: app.kind.clone(),
                    value: app.value.clone(),
                }));
                if state.selected.kind != "none" {
                    ids.push(Identity {
                        kind: state.selected.kind.clone(),
                        value: state.selected.value.clone(),
                    });
                }
            }
            ids.sort_by(|a, b| (&a.kind, &a.value).cmp(&(&b.kind, &b.value)));
            ids.dedup();
            let mut exclusions: Vec<MenuItem<Self>> = ids
                .into_iter()
                .map(|id| {
                    let checked = settings.exclusions.contains(&id);
                    CheckmarkItem {
                        label: identity_label(&id),
                        checked,
                        enabled,
                        activate: Box::new(move |tray: &mut Self| {
                            tray.send(Action::Exclude(id.clone(), !checked))
                        }),
                        ..Default::default()
                    }
                    .into()
                })
                .collect();
            if exclusions.is_empty() {
                exclusions.push(text("No applications known"));
            }
            menu.push(
                SubMenu {
                    label: "Exclude from MRO".into(),
                    enabled,
                    submenu: exclusions,
                    ..Default::default()
                }
                .into(),
            );
        }
        menu.push(MenuItem::Separator);
        menu.push(
            StandardItem {
                label: "Quit tray (daemon keeps running)".into(),
                activate: Box::new(|tray: &mut Self| tray.quit.notify_one()),
                ..Default::default()
            }
            .into(),
        );
        menu
    }
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        if self.wait_for_host {
            eprintln!("Tray host unavailable: {reason:?}. Waiting for the desktop tray host.");
            return true;
        }
        eprintln!(
            "Tray host unavailable: {reason:?}. The daemon is unaffected; restart the tray when a host is available."
        );
        self.quit.notify_one();
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Application, CaptureState, Selected, Settings, State};
    use ksni::Tray as _;

    fn fixture() -> (Tray, mpsc::Receiver<Command>) {
        let (commands, inbox) = mpsc::channel(1);
        let view = View {
            owner: Some(":1.7".into()),
            state: Some(State {
                revision: 4,
                applications: vec![Application {
                    kind: "identity".into(),
                    value: "".into(),
                    name: "".into(),
                    instances: 2,
                }],
                selected: Selected {
                    kind: "desktop-entry".into(),
                    value: "missing".into(),
                    available: false,
                    service: "".into(),
                    owner: "".into(),
                },
                capture: CaptureState {
                    status: "disabled".into(),
                    error: "".into(),
                },
            }),
            settings: Some(Settings {
                revision: 2,
                capture_enabled: false,
                auto_select_new: false,
                exclusions: vec![Identity {
                    kind: "identity".into(),
                    value: "closed_app".into(),
                }],
                persistence_error: "".into(),
            }),
            ..Default::default()
        };
        (
            Tray {
                view,
                commands,
                quit: Arc::new(Notify::new()),
                wait_for_host: false,
            },
            inbox,
        )
    }
    #[test]
    fn reclaim_is_only_enabled_for_active_capture_and_uses_the_api_owner() {
        let (mut tray, mut inbox) = fixture();
        let find = |tray: &Tray| {
            tray.menu()
                .into_iter()
                .find_map(|item| match item {
                    MenuItem::Standard(item) if item.label == "Reclaim media keys" => Some(item),
                    _ => None,
                })
                .unwrap()
        };
        assert!(!find(&tray).enabled);
        tray.view.state.as_mut().unwrap().capture.status = "enabled".into();
        let item = find(&tray);
        assert!(item.enabled);
        (item.activate)(&mut tray);
        let command = inbox.try_recv().unwrap();
        assert_eq!(command.owner, ":1.7");
        assert!(matches!(command.action, Action::Reclaim));
    }

    #[test]
    fn absent_selection_is_retained_and_empty_identity_is_selectable() {
        let (mut tray, mut inbox) = fixture();
        assert!(tray.view.warning());
        let group = tray
            .menu()
            .into_iter()
            .find_map(|item| match item {
                MenuItem::RadioGroup(group) => Some(group),
                _ => None,
            })
            .unwrap();
        assert_eq!(group.options.len(), 3);
        assert_eq!(group.selected, 2);
        assert!(group.options[2].label.contains("unavailable"));
        (group.select)(&mut tray, 1);
        let command = inbox.try_recv().unwrap();
        assert_eq!(command.owner, ":1.7");
        assert!(
            matches!(command.action, Action::Select(Some(Identity { kind, value })) if kind == "identity" && value.is_empty())
        );
        // Sending does not optimistically replace the daemon's selected state.
        assert_eq!(tray.view.state.unwrap().selected.value, "missing");
    }
    #[test]
    fn absent_exclusions_remain_removable_with_exact_wire_identity() {
        let (mut tray, mut inbox) = fixture();
        let submenu = tray
            .menu()
            .into_iter()
            .find_map(|item| match item {
                MenuItem::SubMenu(menu) => Some(menu),
                _ => None,
            })
            .unwrap();
        let checkbox = submenu
            .submenu
            .into_iter()
            .find_map(|item| match item {
                MenuItem::Checkmark(item) if item.label.contains("closed__app") => Some(item),
                _ => None,
            })
            .unwrap();
        assert!(checkbox.checked);
        (checkbox.activate)(&mut tray);
        assert!(
            matches!(inbox.try_recv().unwrap().action, Action::Exclude(Identity { value, .. }, false) if value == "closed_app")
        );
    }
    #[test]
    fn disconnected_menu_exposes_no_mutations_but_keeps_quit() {
        let (mut tray, _) = fixture();
        tray.view = View::default();
        assert!(tray.view.warning());
        let menu = tray.menu();
        assert!(!menu.iter().any(|item| matches!(
            item,
            MenuItem::Checkmark(_) | MenuItem::RadioGroup(_) | MenuItem::SubMenu(_)
        )));
        assert!(menu.iter().any(|item| matches!(item, MenuItem::Standard(item) if item.enabled && item.label.starts_with("Quit"))));
    }
}
