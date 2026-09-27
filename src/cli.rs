use std::ffi::OsString;

use media_router::player::selection::ApplicationId;

pub const HELP: &str = "Usage: media-router [--select DESKTOP_ENTRY | --select-identity IDENTITY]

Watch MPRIS players and report the selected application's availability.

  --select DESKTOP_ENTRY    Select by DesktopEntry (for example: spotify).
  --select-identity NAME   Select by exact Identity when DesktopEntry is absent.
  -h, --help               Show this help.

With no selection argument, players are displayed without selecting one.
Quote Identity values containing spaces. Matching is exact and case-sensitive.
Selection is retained while this process runs, including when a player exits
and returns. It is not saved across router restarts. Press Ctrl+C to stop.
This command does not send playback commands or capture media keys.";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Watch(Option<ApplicationId>),
    Help,
}

pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let arguments = arguments
        .into_iter()
        .map(|argument| {
            argument
                .into_string()
                .map_err(|_| "arguments must be valid UTF-8".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    match arguments.as_slice() {
        [] => Ok(Command::Watch(None)),
        [option] if option == "--help" || option == "-h" => Ok(Command::Help),
        [option, value] if option == "--select" => {
            if value.is_empty() {
                return Err("--select requires a nonempty DesktopEntry".into());
            }
            Ok(Command::Watch(Some(ApplicationId::DesktopEntry(
                value.clone(),
            ))))
        }
        [option, value] if option == "--select-identity" => {
            Ok(Command::Watch(Some(ApplicationId::Identity(value.clone()))))
        }
        _ => Err(
            "expected no arguments, --help, --select DESKTOP_ENTRY, or --select-identity IDENTITY"
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Result<Command, String> {
        parse(values.iter().map(OsString::from))
    }

    #[test]
    fn distinguishes_selection_namespaces_and_preserves_exact_identity() {
        assert_eq!(args(&[]).unwrap(), Command::Watch(None));
        assert_eq!(
            args(&["--select", "spotify"]).unwrap(),
            Command::Watch(Some(ApplicationId::DesktopEntry("spotify".into())))
        );
        for identity in ["Spotify", "Player With Spaces", " Lecteur 音楽 ", ""] {
            assert_eq!(
                args(&["--select-identity", identity]).unwrap(),
                Command::Watch(Some(ApplicationId::Identity(identity.into())))
            );
        }
        assert_eq!(args(&["--help"]).unwrap(), Command::Help);
        assert_eq!(args(&["-h"]).unwrap(), Command::Help);
    }

    #[test]
    fn rejects_missing_unknown_and_conflicting_arguments() {
        for arguments in [
            vec!["--select"],
            vec!["--select-identity"],
            vec!["--select", ""],
            vec!["--unknown"],
            vec!["spotify"],
            vec!["--select", "spotify", "--select-identity", "Spotify"],
            vec!["--select", "spotify", "--select", "firefox"],
        ] {
            assert!(args(&arguments).is_err(), "accepted {arguments:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_utf8_arguments() {
        use std::os::unix::ffi::OsStringExt;
        assert!(parse([OsString::from_vec(vec![0xff])]).is_err());
    }
}
