use std::collections::HashMap;

use zbus::{Connection, fdo::PropertiesProxy, zvariant::OwnedValue};

use super::{Capabilities, PlaybackStatus, Player};

pub(crate) const PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
type Properties = HashMap<String, OwnedValue>;

/// Query the unique owner, so a name changing hands cannot mix two players' data.
pub(crate) async fn inspect(
    connection: &Connection,
    service: &str,
    owner: &str,
) -> Result<Option<Player>, String> {
    let proxy = PropertiesProxy::builder(connection)
        .destination(owner)
        .map_err(|error| error.to_string())?
        .path(PATH)
        .map_err(|error| error.to_string())?
        .build()
        .await
        .map_err(|error| error.to_string())?;
    let properties = proxy
        .get_all(PLAYER.try_into().expect("constant interface name"))
        .await
        .map_err(|error| format!("cannot read Player interface: {error}"))?;
    if !boolean(&properties, "CanControl")? {
        return Ok(None);
    }
    let root = proxy
        .get_all(ROOT.try_into().expect("constant interface name"))
        .await
        .map_err(|error| format!("cannot read MediaPlayer2 interface: {error}"))?;
    parse(service, owner, &root, &properties).map(Some)
}

fn parse(
    service: &str,
    owner: &str,
    root: &Properties,
    properties: &Properties,
) -> Result<Player, String> {
    let desktop_entry = match root.get("DesktopEntry") {
        None => None,
        Some(value) => {
            let value =
                <&str>::try_from(value).map_err(|_| "DesktopEntry must be a string".to_owned())?;
            (!value.is_empty()).then(|| value.to_owned())
        }
    };
    let playback_status = match string(properties, "PlaybackStatus")? {
        "Playing" => PlaybackStatus::Playing,
        "Paused" => PlaybackStatus::Paused,
        "Stopped" => PlaybackStatus::Stopped,
        other => return Err(format!("invalid PlaybackStatus: {other:?}")),
    };
    Ok(Player {
        service: service.to_owned(),
        owner: owner.to_owned(),
        identity: string(root, "Identity")?.to_owned(),
        desktop_entry,
        playback_status,
        capabilities: Capabilities {
            can_control: boolean(properties, "CanControl")?,
            can_play: boolean(properties, "CanPlay")?,
            can_pause: boolean(properties, "CanPause")?,
            can_go_next: boolean(properties, "CanGoNext")?,
            can_go_previous: boolean(properties, "CanGoPrevious")?,
            can_seek: boolean(properties, "CanSeek")?,
        },
    })
}

fn string<'a>(properties: &'a Properties, name: &str) -> Result<&'a str, String> {
    properties
        .get(name)
        .ok_or_else(|| format!("missing {name}"))?
        .try_into()
        .map_err(|_| format!("{name} must be a string"))
}

fn boolean(properties: &Properties, name: &str) -> Result<bool, String> {
    properties
        .get(name)
        .ok_or_else(|| format!("missing {name}"))?
        .try_into()
        .map_err(|_| format!("{name} must be a boolean"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Str;

    fn text(value: &str) -> OwnedValue {
        Str::from(value).into()
    }

    fn valid_properties() -> (Properties, Properties) {
        let root = HashMap::from([("Identity".into(), text("Test Player"))]);
        let mut player = HashMap::from([("PlaybackStatus".into(), text("Paused"))]);
        for name in [
            "CanControl",
            "CanPlay",
            "CanPause",
            "CanGoNext",
            "CanGoPrevious",
            "CanSeek",
        ] {
            player.insert(name.into(), true.into());
        }
        (root, player)
    }

    #[test]
    fn missing_or_empty_desktop_entry_stays_optional() {
        let (mut root, properties) = valid_properties();
        for value in [None, Some("")] {
            if let Some(value) = value {
                root.insert("DesktopEntry".into(), text(value));
            }
            let player = parse("org.mpris.MediaPlayer2.test", ":1.42", &root, &properties).unwrap();
            assert_eq!(player.desktop_entry, None);
            assert_eq!(player.playback_status, PlaybackStatus::Paused);
            assert_eq!(player.owner, ":1.42");
        }
    }

    #[test]
    fn missing_and_wrongly_typed_capabilities_are_rejected() {
        let (root, mut properties) = valid_properties();
        properties.remove("CanPause");
        assert!(
            parse("test", ":1.42", &root, &properties)
                .unwrap_err()
                .contains("missing CanPause")
        );
        properties.insert("CanPause".into(), text("true"));
        assert!(
            parse("test", ":1.42", &root, &properties)
                .unwrap_err()
                .contains("CanPause must be a boolean")
        );
    }

    #[test]
    fn malformed_identity_desktop_entry_and_status_are_rejected() {
        let (mut root, mut properties) = valid_properties();
        root.insert("DesktopEntry".into(), true.into());
        assert!(
            parse("test", ":1.42", &root, &properties)
                .unwrap_err()
                .contains("DesktopEntry")
        );
        root.remove("DesktopEntry");
        root.remove("Identity");
        assert!(
            parse("test", ":1.42", &root, &properties)
                .unwrap_err()
                .contains("Identity")
        );
        root.insert("Identity".into(), text("Test"));
        properties.insert("PlaybackStatus".into(), text("Buffering"));
        assert!(
            parse("test", ":1.42", &root, &properties)
                .unwrap_err()
                .contains("PlaybackStatus")
        );
    }
}
