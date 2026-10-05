use std::{ffi::OsString, path::PathBuf};

use media_router::{input::Backend, player::selection::ApplicationId};

pub const HELP: &str =
    "Usage: media-router [--select DESKTOP_ENTRY | --select-identity IDENTITY] [--interactive] [--capture | --no-capture] [--serve]
       media-router --restore-bindings
       media-router --install-autostart /absolute/path/to/media-router
       media-router --remove-autostart

Watch MPRIS players and report the selected application's availability.

  -p, --select DESKTOP_ENTRY    Select by DesktopEntry (for example: spotify).
  -I, --select-identity NAME    Select by exact Identity when DesktopEntry is absent.
  -i, --interactive             Accept transport commands from standard input.
  -c, --capture                 Capture hardware transport keys on Cinnamon/X11.
      --no-capture              Disable capture, overriding a saved preference.
      --input-backend NAME      cinnamon-dbus (default) or x11 (legacy).
  -s, --serve                   Expose the session-bus API; keep running after stdin EOF.
      --restore-bindings        Recover saved Cinnamon bindings and exit.
      --install-autostart PATH  Install a Cinnamon login entry for this executable.
      --remove-autostart        Remove the managed personal login entry.
  -h, --help                    Show this help.

Short flags can be grouped: -ics means --interactive --capture --serve.
Values may follow or attach: -p spotify, -pspotify, -icsp spotify.
-p and -I consume the rest of their group as a value, or the next argument.
Attach values beginning with '-' (for example: -I-name). Options are case-sensitive.
Help, recovery, and autostart commands must be used alone. Duplicate options are errors.

With --serve, selection and preferences are loaded and saved across restarts.
Explicit selection/capture flags override and save those preferences.
Without --serve, preferences are ignored and selection defaults to none.
Quote Identity values containing spaces. Matching is exact and case-sensitive.
Selection is retained while this process runs, including when a player exits
and returns. Press Ctrl+C to stop.
Interactive commands: play-pause, stop, previous, next (one per line).
Without --serve, --interactive, or --capture this command only observes players.
End terminal input to finish pending commands; serving or capture keeps running.
Press Ctrl+C to release capture. Legacy X11 capture restores saved bindings.";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Watch {
        selected: Option<ApplicationId>,
        interactive: bool,
        capture: Option<bool>,
        serve: bool,
        backend: Backend,
    },
    Help,
    RestoreBindings,
    InstallAutostart(PathBuf),
    RemoveAutostart,
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
    if arguments == ["--remove-autostart"] {
        return Ok(Command::RemoveAutostart);
    }
    if arguments
        .first()
        .is_some_and(|arg| arg == "--install-autostart")
    {
        if arguments.len() != 2 || arguments[1].starts_with('-') {
            return Err(
                "--install-autostart requires one absolute executable path and must be used alone"
                    .into(),
            );
        }
        return Ok(Command::InstallAutostart(PathBuf::from(&arguments[1])));
    }
    let mut options = Options::default();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if let Some(short) = argument
            .strip_prefix('-')
            .filter(|s| !s.starts_with('-') && !s.is_empty())
        {
            for (offset, character) in short.char_indices() {
                let option = match character {
                    'i' => "--interactive",
                    'c' => "--capture",
                    's' => "--serve",
                    'p' => "--select",
                    'I' => "--select-identity",
                    'h' => return Err("-h/--help must be used alone".into()),
                    _ => return Err(format!("unknown short option -{character} in {argument:?}")),
                };
                let takes_value = matches!(character, 'p' | 'I');
                let attached = takes_value
                    .then(|| &short[offset + character.len_utf8()..])
                    .filter(|s| !s.is_empty());
                options.apply(option, attached, &mut arguments)?;
                // Once a value-taking option occurs, the rest is its value,
                // even if it resembles flags. char_indices preserves UTF-8.
                if takes_value {
                    break;
                }
            }
        } else {
            options.apply(&argument, None, &mut arguments)?;
        }
    }
    Ok(Command::Watch {
        selected: options.selected,
        interactive: options.interactive,
        capture: options.capture,
        serve: options.serve,
        backend: options.backend.unwrap_or_default(),
    })
}

#[derive(Default)]
struct Options {
    selected: Option<ApplicationId>,
    interactive: bool,
    capture: Option<bool>,
    serve: bool,
    backend: Option<Backend>,
}
impl Options {
    fn apply(
        &mut self,
        option: &str,
        attached: Option<&str>,
        arguments: &mut impl Iterator<Item = String>,
    ) -> Result<(), String> {
        match option {
            "--interactive" => set_flag(&mut self.interactive, option),
            "--capture" | "--no-capture" => {
                if let Some(previous) = self.capture {
                    return Err(if previous == (option == "--capture") {
                        format!("duplicate option {option}")
                    } else {
                        "--capture/-c conflicts with --no-capture".into()
                    });
                }
                self.capture = Some(option == "--capture");
                Ok(())
            }
            "--input-backend" => {
                if self.backend.is_some() {
                    return Err("duplicate option --input-backend".into());
                }
                self.backend = Some(match arguments.next().as_deref() {
                    Some("cinnamon-dbus") => Backend::CinnamonDbus,
                    Some("x11") => Backend::X11,
                    _ => return Err("--input-backend requires cinnamon-dbus or x11".into()),
                });
                Ok(())
            }
            "--serve" => set_flag(&mut self.serve, option),
            "--select" | "--select-identity" => {
                if self.selected.is_some() {
                    return Err("specify only one selection option".into());
                }
                let value = match attached {
                    Some(value) => value.to_owned(),
                    None => arguments.next().filter(|value| !(value.starts_with('-') && value.len() > 1))
                        .ok_or_else(|| format!("expected a value after {option}; attach values starting with '-' to -p or -I"))?,
                };
                self.selected = Some(if option == "--select" {
                    if value.is_empty() {
                        return Err("--select/-p requires a nonempty DesktopEntry".into());
                    }
                    ApplicationId::DesktopEntry(value)
                } else {
                    ApplicationId::Identity(value)
                });
                Ok(())
            }
            "--help" | "--restore-bindings" | "--install-autostart" | "--remove-autostart" => {
                Err(format!("{option} must be used alone"))
            }
            _ => Err(format!("unexpected argument {option:?}")),
        }
    }
}
fn set_flag(flag: &mut bool, option: &str) -> Result<(), String> {
    if *flag {
        return Err(format!("duplicate option {option}"));
    }
    *flag = true;
    Ok(())
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
                capture: None,
                serve: false,
                backend: Backend::CinnamonDbus,
            }
        );
        assert_eq!(
            args(&["--select", "spotify"]).unwrap(),
            Command::Watch {
                selected: Some(ApplicationId::DesktopEntry("spotify".into())),
                interactive: false,
                capture: None,
                serve: false,
                backend: Backend::CinnamonDbus,
            }
        );
        for identity in ["Spotify", "Player With Spaces", " Lecteur 音楽 ", ""] {
            assert_eq!(
                args(&["--select-identity", identity]).unwrap(),
                Command::Watch {
                    selected: Some(ApplicationId::Identity(identity.into())),
                    interactive: false,
                    capture: None,
                    serve: false,
                    backend: Backend::CinnamonDbus,
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
            capture: None,
            serve: false,
            backend: Backend::CinnamonDbus,
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
                capture: None,
                serve: false,
                backend: Backend::CinnamonDbus,
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
                capture: Some(true),
                serve: false,
                backend: Backend::CinnamonDbus,
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

    #[test]
    fn serving_is_opt_in_and_combines_with_existing_options() {
        assert_eq!(
            args(&[
                "--serve",
                "--capture",
                "--interactive",
                "--select",
                "spotify"
            ])
            .unwrap(),
            Command::Watch {
                selected: Some(ApplicationId::DesktopEntry("spotify".into())),
                interactive: true,
                capture: Some(true),
                serve: true,
                backend: Backend::CinnamonDbus,
            }
        );
        for arguments in [
            vec!["--serve", "--serve"],
            vec!["--restore-bindings", "--serve"],
            vec!["--select", "--serve"],
        ] {
            assert!(args(&arguments).is_err());
        }
    }

    #[test]
    fn short_flags_and_groups_match_long_options_in_any_order() {
        let expected = args(&["--interactive", "--capture", "--serve"]).unwrap();
        for input in [
            vec!["-i", "-c", "-s"],
            vec!["-ics"],
            vec!["-isc"],
            vec!["-cis"],
            vec!["-csi"],
            vec!["-sic"],
            vec!["-sci"],
            vec!["-ic", "--serve"],
        ] {
            assert_eq!(args(&input).unwrap(), expected, "{input:?}");
        }
        for (short, long) in [
            ("-i", "--interactive"),
            ("-c", "--capture"),
            ("-s", "--serve"),
        ] {
            assert_eq!(args(&[short]).unwrap(), args(&[long]).unwrap());
        }
    }

    #[test]
    fn selection_values_can_follow_attach_or_end_a_group() {
        let expected = args(&[
            "--interactive",
            "--capture",
            "--serve",
            "--select",
            "spotify",
        ])
        .unwrap();
        for input in [
            vec!["-ics", "-p", "spotify"],
            vec!["-icsp", "spotify"],
            vec!["-icspspotify"],
            vec!["-pspotify", "-ics"],
            vec!["-i", "-pspotify", "--capture", "-s"],
        ] {
            assert_eq!(args(&input).unwrap(), expected, "{input:?}");
        }
        assert_eq!(
            args(&["-p", "spotify"]).unwrap(),
            args(&["--select", "spotify"]).unwrap()
        );
        // p consumes 'ics' as the value; these are not additional flags.
        assert_eq!(
            args(&["-pics"]).unwrap(),
            args(&["--select", "ics"]).unwrap()
        );
    }

    #[test]
    fn identity_values_remain_exact_including_unicode_and_option_like_text() {
        for value in ["Example Player", " Lecteur 音楽 ", "", "ics"] {
            let expected = args(&["--select-identity", value, "--capture", "--serve"]).unwrap();
            assert_eq!(args(&["-csI", value]).unwrap(), expected);
            if !value.is_empty() {
                assert_eq!(args(&[&format!("-csI{value}")]).unwrap(), expected);
            }
        }
        for value in ["--help", "--restore-bindings", "-ics", "-", "-音楽"] {
            assert_eq!(
                args(&[&format!("-I{value}")]).unwrap(),
                Command::Watch {
                    selected: Some(ApplicationId::Identity(value.into())),
                    interactive: false,
                    capture: None,
                    serve: false,
                    backend: Backend::CinnamonDbus,
                }
            );
        }
    }

    #[test]
    fn aliases_cannot_bypass_duplicate_or_selection_conflicts() {
        for input in [
            vec!["-ii"],
            vec!["-cc"],
            vec!["-ss"],
            vec!["-c", "--capture"],
            vec!["--interactive", "-i"],
            vec!["-ics", "--serve"],
            vec!["-pfoo", "--select", "foo"],
            vec!["--select", "foo", "-Ibar"],
            vec!["-Ifoo", "-pbar"],
            vec!["-Ifoo", "--select-identity", "foo"],
        ] {
            assert!(args(&input).is_err(), "accepted {input:?}");
        }
    }

    #[test]
    fn invalid_groups_missing_values_and_combined_standalone_options_fail() {
        for input in [
            vec!["-p"],
            vec!["-I"],
            vec!["-icsp"],
            vec!["-p", ""],
            vec!["-p", "-ics"],
            vec!["-I", "--help"],
            vec!["-I", "-x"],
            vec!["-x"],
            vec!["-icx"],
            vec!["-C"],
            vec!["-音"],
            vec!["-"],
            vec!["--"],
            vec!["--server"],
            vec!["-ih"],
            vec!["-hi"],
            vec!["-h", "-s"],
            vec!["--help", "-c"],
            vec!["-ics", "--restore-bindings"],
            vec!["--restore-bindings", "-s"],
        ] {
            assert!(args(&input).is_err(), "accepted {input:?}");
        }
        assert_eq!(args(&["-h"]).unwrap(), Command::Help);
        assert_eq!(
            args(&["--restore-bindings"]).unwrap(),
            Command::RestoreBindings
        );
    }

    #[test]
    fn capture_override_distinguishes_absent_enabled_and_disabled() {
        assert_eq!(
            args(&["-s", "--no-capture"]).unwrap(),
            Command::Watch {
                selected: None,
                interactive: false,
                capture: Some(false),
                serve: true,
                backend: Backend::CinnamonDbus,
            }
        );
        for input in [
            vec!["-cs", "--no-capture"],
            vec!["--no-capture", "-c"],
            vec!["--no-capture", "--no-capture"],
        ] {
            assert!(args(&input).is_err());
        }
    }

    #[test]
    fn autostart_commands_are_explicit_and_standalone() {
        assert_eq!(
            args(&["--install-autostart", "/opt/My App/media-router"]).unwrap(),
            Command::InstallAutostart(PathBuf::from("/opt/My App/media-router"))
        );
        assert_eq!(
            args(&["--remove-autostart"]).unwrap(),
            Command::RemoveAutostart
        );
        for values in [
            vec!["--install-autostart"],
            vec!["--install-autostart", "--serve"],
            vec!["--install-autostart", "/bin/router", "-s"],
            vec!["-s", "--install-autostart", "/bin/router"],
            vec!["--remove-autostart", "-s"],
        ] {
            assert!(args(&values).is_err());
        }
    }

    #[test]
    fn backend_option_is_explicit_validated_and_not_a_capture_override() {
        let command = args(&["--serve", "--input-backend", "x11"]).unwrap();
        assert!(matches!(
            command,
            Command::Watch {
                backend: Backend::X11,
                capture: None,
                ..
            }
        ));
        assert_eq!(
            args(&["--input-backend", "cinnamon-dbus"]).unwrap(),
            args(&[]).unwrap()
        );
        for input in [
            vec!["--input-backend"],
            vec!["--input-backend", "unknown"],
            vec!["--input-backend", "x11", "--input-backend", "x11"],
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
