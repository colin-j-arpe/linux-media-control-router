use std::ffi::OsString;

use media_router::player::selection::ApplicationId;

pub const HELP: &str =
    "Usage: media-router [--select DESKTOP_ENTRY | --select-identity IDENTITY] [--interactive] [--capture]
       media-router --restore-bindings

Watch MPRIS players and report the selected application's availability.

  --select DESKTOP_ENTRY    Select by DesktopEntry (for example: spotify).
  --select-identity NAME   Select by exact Identity when DesktopEntry is absent.
  --interactive            Accept transport commands from standard input.
  --capture                Capture hardware transport keys on Cinnamon/X11.
  --restore-bindings       Recover saved Cinnamon bindings and exit.
  -h, --help               Show this help.

With no selection argument, players are displayed without selecting one.
Quote Identity values containing spaces. Matching is exact and case-sensitive.
Selection is retained while this process runs, including when a player exits
and returns. It is not saved across router restarts. Press Ctrl+C to stop.
Interactive commands: play-pause, stop, previous, next (one per line).
Without --interactive or --capture this command only observes players.
End terminal input to finish pending commands (capture continues if enabled).
Press Ctrl+C to stop and restore captured bindings.";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Watch {
        selected: Option<ApplicationId>,
        interactive: bool,
        capture: bool,
    },
    Help,
    RestoreBindings,
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
    if arguments.len() == 1 && matches!(arguments[0].as_str(), "--help" | "-h") {
        return Ok(Command::Help);
    }
    if arguments == ["--restore-bindings"] {
        return Ok(Command::RestoreBindings);
    }
    let mut selected = None;
    let mut interactive = false;
    let mut capture = false;
    let mut arguments = arguments.into_iter();
    while let Some(option) = arguments.next() {
        match option.as_str() {
            "--interactive" if !interactive => interactive = true,
            "--capture" if !capture => capture = true,
            "--select" | "--select-identity" if selected.is_none() => {
                let value = arguments
                    .next()
                    .ok_or_else(|| format!("expected a value after {option}"))?;
                if matches!(
                    value.as_str(),
                    "--capture"
                        | "--interactive"
                        | "--restore-bindings"
                        | "--select"
                        | "--select-identity"
                        | "--help"
                        | "-h"
                ) {
                    return Err(format!("expected a value after {option}"));
                }
                if option == "--select" {
                    if value.is_empty() {
                        return Err("--select requires a nonempty DesktopEntry".into());
                    }
                    selected = Some(ApplicationId::DesktopEntry(value));
                } else {
                    selected = Some(ApplicationId::Identity(value));
                }
            }
            _ => {
                return Err(format!(
                    "expected a single selection option and optional --interactive/--capture; unexpected argument {option:?}"
                ));
            }
        }
    }
    Ok(Command::Watch {
        selected,
        interactive,
        capture,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Result<Command, String> {
        parse(values.iter().map(OsString::from))
    }

    #[test]
    fn distinguishes_selection_namespaces_and_preserves_exact_identity() {
        assert_eq!(
            args(&[]).unwrap(),
            Command::Watch {
                selected: None,
                interactive: false,
                capture: false
            }
        );
        assert_eq!(
            args(&["--select", "spotify"]).unwrap(),
            Command::Watch {
                selected: Some(ApplicationId::DesktopEntry("spotify".into())),
                interactive: false,
                capture: false
            }
        );
        for identity in ["Spotify", "Player With Spaces", " Lecteur 音楽 ", ""] {
            assert_eq!(
                args(&["--select-identity", identity]).unwrap(),
                Command::Watch {
                    selected: Some(ApplicationId::Identity(identity.into())),
                    interactive: false,
                    capture: false
                }
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
            vec!["--interactive", "--interactive"],
        ] {
            assert!(args(&arguments).is_err(), "accepted {arguments:?}");
        }
    }

    #[test]
    fn interactive_is_opt_in_and_can_precede_or_follow_selection() {
        let expected = Command::Watch {
            selected: Some(ApplicationId::DesktopEntry("spotify".into())),
            interactive: true,
            capture: false,
        };
        assert_eq!(
            args(&["--select", "spotify", "--interactive"]).unwrap(),
            expected
        );
        assert_eq!(
            args(&["--interactive", "--select", "spotify"]).unwrap(),
            expected
        );
        assert_eq!(
            args(&["--interactive"]).unwrap(),
            Command::Watch {
                selected: None,
                interactive: true,
                capture: false
            }
        );
    }

    #[test]
    fn capture_and_recovery_are_explicit_and_recovery_is_standalone() {
        assert_eq!(
            args(&["--capture"]).unwrap(),
            Command::Watch {
                selected: None,
                interactive: false,
                capture: true
            }
        );
        assert_eq!(
            args(&["--restore-bindings"]).unwrap(),
            Command::RestoreBindings
        );
        for input in [
            vec!["--capture", "--capture"],
            vec!["--capture", "--restore-bindings"],
            vec!["--restore-bindings", "--select", "spotify"],
            vec!["--select", "--capture"],
        ] {
            assert!(args(&input).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_utf8_arguments() {
        use std::os::unix::ffi::OsStringExt;
        assert!(parse([OsString::from_vec(vec![0xff])]).is_err());
    }
}
