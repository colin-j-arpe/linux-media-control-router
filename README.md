# Media Router

A Linux media-control router intended to let users choose which running
media application receives Play/Pause, Stop, Previous, and Next commands.
The initial target is Linux Mint Cinnamon on X11.

## Building from source

Install Rust using rustup. The repository's `rust-toolchain.toml` selects
Rust 1.98.1 with rustfmt and Clippy. Building also requires a C toolchain,
`pkg-config`, and GLib/GIO development files (`build-essential`, `pkg-config`,
and `libglib2.0-dev` on Linux Mint). Running capture requires Cinnamon’s
media-key settings schema and an X11 session.

From the repository root:

```sh
cargo build --locked
cargo run --locked
cargo fmt --check
```

The debug executable is written to `target/debug/media-router`.

## Short command-line options

Long options remain supported. These short aliases are also available:

| Long option | Short option |
|---|---|
| `--interactive` | `-i` |
| `--capture` | `-c` |
| `--no-capture` | — |
| `--serve` | `-s` |
| `--select DESKTOP_ENTRY` | `-p DESKTOP_ENTRY` |
| `--select-identity IDENTITY` | `-I IDENTITY` |
| `--help` | `-h` |
| `--install-autostart PATH` | — |
| `--remove-autostart` | — |

Options are case-sensitive: `-i` enables interactive input, while `-I`
takes a fallback Identity. `--no-capture`, `--restore-bindings`, and both
autostart commands are long-only.
`--capture` and `--no-capture` cannot be combined.

Short flags can be grouped. These commands are equivalent:

```sh
media-router --interactive --capture --serve --select spotify
media-router -ics -p spotify
media-router -icsp spotify
media-router -icspspotify
```

A value-taking short option consumes the remainder of its group as its
value, or the next argument if the group ends there. Thus `-pics` selects
the desktop entry `ics`; it does not enable interactive input, capture,
or serving. Values containing spaces must be quoted:

```sh
media-router -cs -I "Example Player"
media-router -csI"Example Player"
```

Pass option-like values beginning with `-` attached to the short option,
for example `-I--help` selects the exact Identity `--help`. Empty fallback
identities remain supported with `-I ""`; empty desktop-entry IDs are
invalid. No abbreviation of long option names is inferred: use `--serve`,
not `--server`.

Unknown options, missing values, duplicate options (including mixed forms
such as `-c --capture`), and multiple selection options are errors. Help,
recovery, and autostart commands must each be used alone, outside a group
or other options.
When running through Cargo, put app arguments after `--`, for example
`cargo run --locked -- -ics -p spotify`.

## Discovering media applications

Run `cargo run --locked` from a terminal in your Linux desktop session.
The application connects to the user session D-Bus and reports eligible
MPRIS players as they appear and disappear. Press Ctrl+C to stop.

`PLAYER ADDED` includes the application's identity, service name, playback
status, and control capabilities. Status and capabilities are snapshots
taken at discovery time. `PLAYER REMOVED` indicates that a previously
discovered instance is no longer available. `PLAYER SKIPPED` explains why
a service could not be admitted, such as missing MPRIS properties or
`CanControl` being false.

Each player is labelled with its application identifier: `DesktopEntry`
when supplied, or its exact MPRIS `Identity` otherwise. The two kinds of
identifier are distinct even when their text is identical.

This command observes players; it does not send playback commands or
change desktop keybindings.

## Selecting an application

Select by desktop-entry identifier:

```sh
cargo run --locked -- --select spotify
```

For a player without `DesktopEntry`, select by its exact displayed identity:

```sh
cargo run --locked -- --select-identity "Example Player"
```

Matching is case-sensitive. Identity values are not trimmed or normalized.
Players without `DesktopEntry` that report identical identities are treated
as the same application; a changed identity requires reselection.
`--select-identity` does not match players that have a desktop-entry ID.

The output reports `SELECTION AVAILABLE` with the active service and owner,
or `SELECTION UNAVAILABLE` while the selected application is absent or has
not yet been validated. The selection stays in place when the application
closes and becomes available again when it returns.

The first validated matching instance stays active while available. If it
disappears, another matching instance is chosen by service-name ordering.
The selection never switches to a different application because one closes.

With `--serve`, selection is saved across router restarts and can be changed
through the D-Bus API. Explicit selection arguments override and save the
new selection. Without `--serve`, selection is temporary, saved preferences
are ignored, and omitting a selection argument leaves nothing selected.
Add `--interactive` to send terminal commands or `--capture` to route
hardware transport keys.
Use `--help` for usage.

## Sending transport commands

```sh
cargo run --locked -- --select spotify --interactive
```

Wait for `SELECTION AVAILABLE`, then enter one command per line:

```text
play-pause
next
previous
stop
```

`--interactive` also works with `--select-identity`. Each input targets the
selected instance, with its current capabilities checked before dispatch.
Commands received while no instance is available are discarded; they are
not saved for the application's return. If the target changes before
dispatch, that command is skipped rather than sent to a different instance.

Commands run in input order, with up to 32 waiting commands. Additional
commands are discarded with a diagnostic when that queue is full. Discovery
continues while a command is being checked or sent.

The output distinguishes:

- `COMMAND ACKNOWLEDGED`: the player returned successfully; playback may
  still be unchanged, for example if Stop was sent while already stopped.
- `COMMAND SKIPPED`: no selection, unavailable/changed target, or an
  unsupported action.
- `COMMAND NOT SENT`: a pre-dispatch check failed or timed out.
- `COMMAND PLAYER ERROR`: the player returned a D-Bus error.
- `COMMAND UNCERTAIN`: no definitive method reply was received; the command
  may already have executed. It is not retried.

Checks and the method reply each have a two-second timeout. No transport
method is automatically retried. Close standard input (Ctrl+D in a terminal)
to finish pending commands and exit when neither `--serve` nor `--capture`
is enabled, or press Ctrl+C to exit immediately.
An in-flight command may already have executed when interrupted.

In diagnostic runs without `--serve`, omitting `--capture` leaves desktop
keybindings unchanged. Serving runs also honor saved capture preferences.
Volume and mute are outside the transport command set.

## Capturing hardware transport keys

Run inside your Cinnamon X11 desktop session:

```sh
cargo run --locked -- --select spotify --capture
```

Wait for `CAPTURE READY` and `SELECTION AVAILABLE`, then use your media
keys. Play and Pause keys both toggle play/pause. Stop, Previous, and Next
route to the selected application. Holding a key sends one command per
press. Caps Lock, Num Lock, and Scroll Lock do not prevent capture.
Volume and mute remain under desktop control.

Capture temporarily removes only the five plain `XF86Audio*` transport
shortcuts from their corresponding Cinnamon settings. Custom shortcuts
remain active in Cinnamon and follow **Cinnamon’s player choice**, which
may differ from Media Router’s selection. Modified transport shortcuts
(such as Ctrl+Play) are not captured.

No selected/available application means transport inputs are discarded.
`--interactive` can be combined with `--capture`; closing terminal input
then leaves hardware capture running. Ctrl+C or SIGTERM stops capture and
restores the prior settings. Only one capture/recovery process may run
against the same state directory.

If another application holds the required keys, startup fails and rolls
back the changes. Capture also stops and attempts to restore bindings if the relevant
keyboard mappings change, the X connection fails, or a binding changed by
the router is edited externally. Restart after a keyboard mapping change.
External edits are preserved during restoration, with a diagnostic.
With `--serve`, capture failures leave discovery and API access running,
with a fault status. Without `--serve`, capture failures end the process.

### Recovering after a crash

The router saves its recovery journal **before** changing Cinnamon settings.
After a crash, SIGKILL, or power loss, those settings can remain changed,
even across logout. Recovery runs at the next serving startup, including
when saved capture is disabled, and before a new capture session. To
recover manually, run:

```sh
cargo run --locked -- --restore-bindings
```

Recovery does not require an X connection or a selected player. Run it as
the same user, with the same settings backend and state directory. Run it
before uninstalling if capture did not shut down normally.

The journal is `$XDG_STATE_HOME/media-router/bindings.json`, or
`~/.local/state/media-router/bindings.json` when `XDG_STATE_HOME` is unset
or relative. Do not delete an outstanding journal: it contains the original
shortcut settings. Failed restoration retains it for another attempt.
When external edits are preserved, the original backup is retained as
`bindings-conflict-*.json` for inspection, not automatically replayed.

Restoration preserves explicit settings and inherited defaults. Detecting
external edits is best effort: comparison and writing are not atomic, and
an edit identical to the router’s temporary value is indistinguishable.
Serving recovery failure leaves the API available with a capture fault;
the journal is retained and capture does not start. After addressing the
cause, `SetCaptureEnabled(false)` retries recovery without acquiring keys.
Recovery respects the capture lock and cannot restore bindings owned by
a running capturer. There is no watchdog for immediate crash recovery.

## Controlling the running daemon

```sh
cargo run --locked -- --serve
```

`--serve` exposes the session-bus service `org.mediarouter.MediaRouter1`.
It lets clients read applications and availability, change or clear the
selection, enable/disable capture, configure MRO auto-selection, and receive
state and settings updates. Capture follows its saved preference, which
defaults to disabled. Use `--capture` or `--no-capture` to override and save
that preference at startup. Standard input closing
does not stop a serving daemon. Press Ctrl+C to stop and restore bindings.

For example, use these optional diagnostic commands from another terminal:

```sh
busctl --user call org.mediarouter.MediaRouter1 /org/mediarouter/MediaRouter1 org.mediarouter.MediaRouter1 SelectApplication ss desktop-entry spotify
busctl --user call org.mediarouter.MediaRouter1 /org/mediarouter/MediaRouter1 org.mediarouter.MediaRouter1 SetCaptureEnabled b true
```

Only one API server can run on a session bus. Snapshots and revision numbers
remain in RAM; selection and preferences are saved across router restarts. See the [D-Bus API reference](docs/DBUS_API.md) for the full
contract, recovery behavior, client synchronization, and examples.

## Saved preferences and automatic selection

Serving runs use `$XDG_CONFIG_HOME/media-router/config.json`, falling back
to `~/.config/media-router/config.json`. Empty or relative XDG values are
ignored. Missing configuration defaults to no selection, capture disabled,
and automatic selection disabled. Invalid configuration causes serving
startup to fail with an error; the file is preserved for correction.
Stop the daemon before editing the file directly.

Selection changes, capture requests, and automatic-selection preferences
are saved before success is reported. A capture failure does not erase
the enabled preference: another daemon start will retry capture. Normal
shutdown restores Cinnamon bindings without disabling that saved preference.
Use `SetCaptureEnabled(false)` or start with `--serve --no-capture` to save
capture as disabled. The binding-recovery journal remains separate.

Optional **most-recently-opened (MRO)** selection chooses a newly discovered
eligible media application. It observes MPRIS availability, not process
creation: a browser may first appear when a media page opens. Startup
players do not overwrite the saved selection, even when validation is slow.
Enabling MRO does not select an already discovered application.

Additional instances of an available application do not trigger MRO.
Reopening after all its instances disappear can trigger it again. Among
eligible arrivals, the latest validated arrival wins, and may override a
manual selection. Clearing selection does not disable MRO. Closing the
selected application never selects another application automatically.
Playback changes do not trigger automatic selection.

Enable MRO and optionally exclude applications using their exact typed IDs:

```sh
busctl --user call org.mediarouter.MediaRouter1 /org/mediarouter/MediaRouter1 org.mediarouter.MediaRouter1 SetAutoSelectNew b true
busctl --user call org.mediarouter.MediaRouter1 /org/mediarouter/MediaRouter1 org.mediarouter.MediaRouter1 SetAutoSelectExclusions 'a(ss)' 1 desktop-entry firefox
busctl --user call org.mediarouter.MediaRouter1 /org/mediarouter/MediaRouter1 org.mediarouter.MediaRouter1 GetSettings
```

The exclusions call replaces the entire list; pass a count of `0` to clear
it. Excluded applications remain manually selectable. Save failures appear
as API errors and in `GetSettings().persistence_error`; automatic-selection
save failures are also logged. See the API reference for retry semantics.

## Starting at login

Build and install the executable in a stable location, then explicitly
register its absolute path. For a per-user installation:

```sh
cargo build --release --locked
install -Dm755 target/release/media-router "$HOME/.local/bin/media-router"
"$HOME/.local/bin/media-router" --install-autostart "$HOME/.local/bin/media-router"
```

This creates `$XDG_CONFIG_HOME/autostart/media-router.desktop`, normally
`~/.config/autostart/media-router.desktop`. The entry launches the supplied
executable with `--serve` at the next Cinnamon login, using saved preferences.
Installation does not start a daemon immediately. Hardware capture still
requires Cinnamon on X11 and an enabled capture preference.

Use an installed executable rather than a file under Cargo's `target`
directory, which a clean build can remove. The path must be absolute, valid
UTF-8, and point to an executable file; control characters and `=` are not
supported. Spaces and quoted characters are escaped by the installer.
If you relocate the executable, rerun installation with the new path.

The installer can update its own entry. It refuses to replace or remove
an unrecognized entry or a symlink at that filename; inspect and move such
an entry yourself if you want the router to manage it.

To remove the personal login entry:

```sh
"$HOME/.local/bin/media-router" --remove-autostart
```

Removal leaves the running daemon and saved preferences intact. Stop the
daemon cleanly before removing its executable; recover outstanding binding
changes first if it crashed. Removing an already absent entry is harmless.
These commands do not require a running session bus or X server.

## Contributing

Read [the project context](docs/PROJECT_CONTEXT.md) before making
architectural changes.

The package uses Rust edition 2024. Run the checks from the repository root:

```sh
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --check
```

Integration tests require `dbus-daemon`, `Xvfb` (`xvfb` on Linux Mint),
the Cinnamon media-key settings schema, and permission to create local
Unix sockets. They use private session buses, X servers, and settings
files; they do not send input to your desktop or change its keybindings. Unit tests alone can be run with `cargo test --lib
--locked` in environments that cannot create sockets.

### Tray compatibility probe

To check tray support in a desktop session, run the standalone sample:

```sh
cargo run --locked --example tray_probe -- --seconds 600
```

Use this visual checklist. The host controls opening and dismissal for this
example: on Cinnamon, left-click may do nothing and selections may close the
menu. These are acceptable host-dependent behaviours; reopen with right-click
to test the remaining controls.

- [ ] The blue play icon appears without clipping or an opaque background.
- [ ] Hovering shows the probe title, selected sample, and availability.
- [ ] The menu opens using the host's supported action (right-click on Cinnamon).
      Record whether left-click also opens it; this is optional.
- [ ] Clicking outside or pressing Escape dismisses the menu; it reopens normally.
- [ ] Radio and checkbox changes work whether the host keeps the menu open or
      closes it. Reopening preserves the updated state.
- [ ] The menu stays on-screen; text and separators are readable. The heading
      and selected-status label are disabled and cannot be activated.
- [ ] Selecting each sample moves the radio indicator so exactly one is marked;
      the selected-status label and tooltip follow the selection.
- [ ] “Simulate unavailable player” gains a checkmark and shows a red exclamation
      badge on the icon. The status label and tooltip say unavailable. Turning
      it off removes the badge and restores available status.
- [ ] “Demo MRO toggle (memory only)” alternates its checkmark and retains its
      state when the menu is reopened. It does not implement selection policy.
- [ ] “Show extra sample player” adds “Sample player — 音楽” with readable Unicode.
      Select it, then hide it: the row disappears and selection returns to the
      sample music player. Showing it again does not create duplicate rows.
- [ ] “Demo exclusions” opens a submenu whose checkbox toggles and retains its
      state on reopening. This sample toggle does not remove the browser row.
- [ ] Repeated selections and toggles leave no stale labels or extra indicators.
- [ ] “Quit compatibility probe” removes the icon and ends the process.

Callbacks are logged in the terminal. If multiple displays or scaling settings
are part of your normal setup, repeat the icon/menu checks there too; this probe
does not establish compatibility with configurations that have not been tested.

This example uses sample data only and does not control the daemon, media
players, capture, saved preferences, or autostart. Quit from the menu or use
Ctrl+C. It exits automatically after the requested duration (default 300
seconds). A compatible StatusNotifier tray host must be running.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
