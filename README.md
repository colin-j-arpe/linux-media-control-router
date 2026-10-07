# Media Router

A Linux media-control router intended to let users choose which running
media application receives Play/Pause, Stop, Previous, and Next commands.
The initial target is Linux Mint Cinnamon on X11.

## Installing a release archive

The initial release is `0.1.0`. See [release notes](CHANGELOG.md) for features
and limitations. Binary archives are built on Linux Mint 22.3 for x86-64;
other distributions and Wayland have not been verified. The daemon requires
GLib/GIO runtime libraries, Cinnamon's media-key settings schema, and an X11
session. The tray requires a StatusNotifier-compatible host.

Download the archive and its `SHA256SUMS` file into the same directory. Verify
and extract them:

```sh
sha256sum -c SHA256SUMS
tar -xzf media-router-0.1.0-linuxmint22.3-x86_64.tar.gz
cd media-router-0.1.0-linuxmint22.3-x86_64
install -Dm755 bin/media-router "$HOME/.local/bin/media-router"
install -Dm755 bin/media-router-tray "$HOME/.local/bin/media-router-tray"
```

When updating an existing installation, stop its daemon and quit its tray
before replacing either executable. Saved preferences are retained. Run
`$HOME/.local/bin/media-router --serve` and, in a separate terminal,
`$HOME/.local/bin/media-router-tray`. Enable **Capture media keys** in the tray;
capture is opt-in. Login startup is also opt-in; see [Starting at login](#starting-at-login).
The published `0.1.0` autostart command starts only the daemon. Paired daemon/tray
autostart described below is an unreleased change; build the updated source
to use it.

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

## Tray client

Build the separate GUI executable:

```sh
cargo build --locked -p media-router-tray
```

Run the daemon and tray in separate terminals (or launch the tray alongside
an already running daemon):

```sh
target/debug/media-router --serve
```

```sh
target/debug/media-router-tray
```

The tray requires a StatusNotifier-compatible host on the session bus.
Right-click the blue play icon on Cinnamon. Other hosts may also support
left-click; menu opening and dismissal follow the desktop's conventions.
The tray does not start the daemon or install login entries. The
`--install-autostart` command registers both the daemon and its sibling tray
executable for login startup (an unreleased change after `0.1.0`). To install
the tray executable separately, use `cargo install --locked --path crates/tray`.

The menu provides:

- Player selection, including applications without a DesktopEntry and a
  retained selection whose application has closed. “No selection” clears it.
- A capture checkbox showing saved intent, plus a separate actual capture
  status and recovery actions when capture is faulted.
- “Reclaim media keys” refreshes active listener priority without changing settings.
- MRO auto-selection and exclusions. Absent excluded applications stay in
  the submenu so their exclusions can be removed.
- Connection, request, capture, and preference-save diagnostics.
- “Quit tray,” which leaves the daemon and capture running.

The red exclamation badge indicates an unavailable selected application,
an unavailable daemon, or a reported error; inspect the menu for the reason.
Settings and selection are confirmed by the daemon, not changed optimistically
by the GUI. A failed or timed-out request is not automatically retried; inspect
the refreshed state before trying again. Preferences are saved by the daemon.
The capture backend still requires Cinnamon/X11 regardless of tray support.

The tray reconnects when the daemon starts or restarts, and discards actions
for an old daemon. Login startup uses `--wait-for-host`: the tray stays running
while the host initializes and reconnects if it disappears and returns. If no
host ever appears, the tray remains waiting without an icon. Manual launches
without that flag exit with an error if the host is absent; if it disappears
later, restart the tray when the host returns. After a session-bus failure,
restart the tray too.
Only one instance of this tray client may run on a session bus.

The v1 exclusions API replaces the whole list. The tray rereads it before an
edit, but simultaneous exclusion edits from multiple clients can still
overwrite one another; avoid editing that list concurrently.

## Known limitations and deferred features

On the tested Cinnamon/X11 system, playback keys do not respond while some
GTK tray menus are open, including Media Router and Update Manager. Cinnamon
emits no media-key notifications during that interval; closing the menu
immediately restores control. Cinnamon panel menus such as Sound do not show
this interruption. Reclaiming media keys does not resolve the missing
notifications. This desktop limitation is accepted; no menu workaround is
implemented.

Selection targets media applications rather than individual browser tabs.
Selecting individual tabs with media content is a deferred feature; standard
MPRIS does not reliably provide a separately addressable player for each tab.
Most-recently-played selection (MRP) is also deferred. The current optional
auto-selection mode is MRO, which reacts to newly available media applications.

## Short command-line options

Long options remain supported. These short aliases are also available:

| Long option | Short option |
|---|---|
| `--interactive` | `-i` |
| `--capture` | `-c` |
| `--no-capture` | — |
| `--input-backend cinnamon-dbus\|x11` | — |
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
route to the selected application. The D-Bus backend follows Cinnamon's
logical key events; held-key repetition follows the desktop. Legacy X11
capture suppresses repeated events until release. Caps Lock, Num Lock, and
Scroll Lock do not prevent plain-key capture.
Volume and mute remain under desktop control.

The default `cinnamon-dbus` backend registers for Cinnamon's media-key events
without changing keybindings. It routes Play/Pause, Stop, Previous, and Next
while unlocked and while locked when lock-screen keyboard shortcuts are allowed.
Volume and mute remain handled by Cinnamon.

Listener priority is best effort. Another application can register later and
receive the keys instead. The daemon refreshes registration on successful
selection requests, admitted player arrivals, and screen activation. It does
not refresh periodically or detect silent priority loss. Use **Reclaim media
keys** in the tray if the keys stop following your selection. This action
requires active capture and does not change saved preferences. CLI/API clients
can request the same action:

```sh
busctl --user call org.mediarouter.MediaRouter1 /org/mediarouter/MediaRouter1 org.mediarouter.MediaRouter1 ReclaimMediaKeys
```

Normal plain Cinnamon transport bindings are required. Custom or disabled
transport shortcuts are preserved and reported as a capture fault; the D-Bus
event cannot distinguish a custom accelerator from a hardware media key.
External edits to these bindings also stop capture without overwriting them.
The legacy backend remains available explicitly:

```sh
media-router --serve --input-backend x11
```

The backend choice applies to that process and is not a saved preference;
normal launches and login startup use `cinnamon-dbus`. Legacy X11 capture
removes only plain XF86 transport bindings, preserves custom shortcuts under
Cinnamon's player choice, and restores changed values on exit. It cannot receive
keys while the lock screen owns the keyboard. Reclaim is a no-op for its
existing passive grabs.

No selected/available application means received transport inputs are discarded.
`--interactive` can be combined with `--capture`; closing terminal input leaves
capture running. Ctrl+C or SIGTERM releases capture. Only one capture/recovery
process may run against the same state directory. Cinnamon also removes a
vanished D-Bus listener when its connection closes, including after a crash.
The new backend does not create a binding-recovery journal.

Screensaver idle exits and restarts do not interrupt D-Bus capture. Media-key
service loss/replacement or registration failure stops capture and reports a
fault; retry enabling after the cause is addressed. With `--serve`, discovery
and API access remain running. Without `--serve`, capture faults end the process.

## Recovering after a crash

Legacy X11 capture saves its recovery journal **before** changing Cinnamon settings.
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
shutdown releases capture without disabling that saved preference; legacy X11
capture also restores Cinnamon bindings.
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

Build and install both executables beside each other in a stable location,
then explicitly register the daemon's absolute path. For a per-user installation:

```sh
cargo build --release --workspace --locked
install -Dm755 target/release/media-router "$HOME/.local/bin/media-router"
install -Dm755 target/release/media-router-tray "$HOME/.local/bin/media-router-tray"
"$HOME/.local/bin/media-router" --install-autostart "$HOME/.local/bin/media-router"
```

This creates two managed entries under `$XDG_CONFIG_HOME/autostart` (normally
`~/.config/autostart`): `media-router.desktop` launches the daemon with `--serve`,
and `media-router-tray.desktop` launches the sibling `media-router-tray` with
`--wait-for-host`. Both start without terminals at the next Cinnamon login.
The tray can start before the daemon; it connects when the daemon is ready.
Saved preferences determine capture and selection. Installation does not
launch either program immediately. Run the command again to upgrade an old
daemon-only entry. If the sibling tray executable is missing or invalid,
installation fails without changing an existing entry. Hardware capture still
requires Cinnamon on X11 and an enabled capture preference.

Use an installed executable rather than a file under Cargo's `target`
directory, which a clean build can remove. The path must be absolute, valid
UTF-8, and point to an executable file; control characters and `=` are not
supported. Spaces and quoted characters are escaped by the installer.
If you relocate the executable, rerun installation with the new path.

The installer can update its own entries. It refuses to replace or remove
an unrecognized entry or a symlink at either filename; inspect and move such
an entry yourself if you want the router to manage it.

To remove both personal login entries:

```sh
"$HOME/.local/bin/media-router" --remove-autostart
```

Removal leaves the running daemon, tray, and saved preferences intact.
It also supports removing an old daemon-only entry. Stop the daemon cleanly
and quit the tray before removing their executables; recover outstanding binding
changes first if it crashed. Removing an already absent entry is harmless.
These commands do not require a running session bus or X server.

## Contributing

Read [the project context](docs/PROJECT_CONTEXT.md) before making
architectural changes.

The package uses Rust edition 2024. Run the checks from the repository root:

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all --check
```

Integration tests require `dbus-daemon`, `Xvfb` (`xvfb` on Linux Mint),
the Cinnamon media-key settings schema, and permission to create local
Unix sockets. They use private session buses, X servers, and settings
files; they do not send input to your desktop or change its keybindings. Unit tests alone can be run with `cargo test --lib
--locked` in environments that cannot create sockets. Tray unit tests can be
run with `cargo test -p media-router-tray --lib --locked`.

### Replacing the GUI

The root package is the daemon/CLI. `crates/tray` is an independent workspace
package; neither production package depends on the other's Rust implementation.
The root's development dependency on the tray exists only for the real-daemon
client integration test. A normal `cargo build --locked` builds the daemon/CLI;
use `--workspace` to build both packages.

The tray package separates responsibilities:

| File | Responsibility |
|---|---|
| `src/protocol.rs` | Client-side records matching the public D-Bus contract |
| `src/client.rs` | Owner tracking, snapshots, signals, and API requests |
| `src/model.rs` | Revision handling and UI state |
| `src/tray.rs`, `src/icon.rs` | Replaceable tray menu and artwork |
| `src/main.rs` | Executable lifecycle and session-bus connection |

A replacement GUI can use any language or toolkit and communicate solely
through [DBUS_API.md](docs/DBUS_API.md) and its linked interface XML. It should
not read configuration files, discover MPRIS players, or manipulate capture
settings directly. Preserve owner/revision synchronization and error semantics.
The tray has no GLib/GIO or X11 dependency of its own.

To remove this GUI from a fork, remove `crates/tray`, its workspace-member and
development-dependency entries from the root manifest, and
`tests/tray_client.rs`. The daemon source and public API need no changes.
To keep the client but replace its widgets, start with `tray.rs` and `icon.rs`.

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

### Cinnamon lock-screen media-key probe

This standalone diagnostic checks Cinnamon's D-Bus media-key delivery without
sending playback commands. Turn off **Capture media keys** in the
tray first, so Cinnamon's original transport bindings are restored. Keep
capture off throughout the experiment.

```sh
cargo build --locked --example media_keys_probe
target/debug/examples/media_keys_probe --seconds 900
```

The probe logs events only: while it owns Cinnamon's listener priority,
transport keys produce log lines rather than controlling playback. It sends
no MPRIS commands and changes no keybindings or saved preferences. Volume
and mute remain handled by the desktop. It refuses empty transport bindings
or disabled lock-screen keyboard shortcuts instead of modifying settings.

- Wait for `PROBE READY`, then press Play/Pause, Stop, Previous, and Next.
  Verify each produces the expected `KEY` line.
- Lock the screen manually, press the same keys at the password screen,
  unlock, and inspect the log for events between `SCREEN active=true` and
  `SCREEN active=false`. The active flag alone also includes an unlocked
  screensaver; confirm you were actually locked.
- Repeat the key presses after unlocking. Check volume/mute while locked too.
- With the main probe running, start a second terminal:

  ```sh
  target/debug/examples/media_keys_probe --contender --seconds 60
  ```

  Key events should move to the contender; after it exits, they should return
  to the main probe. Repeat while locked if practical. Opening/reopening actual
  players while the main probe runs also tests whether they take priority.
- Stop the main probe with Ctrl+C or wait for its lifetime to expire. Confirm
  `PROBE RELEASED`, then check Cinnamon's usual media-key behaviour. Re-enable
  Media Router capture when finished.

Default lifetime is 600 seconds; `--seconds` accepts 1–3600. SIGINT/SIGTERM
release registration; disconnect cleanup is also handled by Cinnamon. The
screensaver normally exits while idle. The probe tolerates its disappearance
and return, and does not activate or keep it alive. `screen_active=None` means
no current snapshot; `Some(true)` and `Some(false)` report the observed state.
Owner transitions identify the service and its old/new connection owners.
Media-key service ownership changes or bus loss end the probe without replaying
registration; screensaver ownership changes leave media-key registration intact.
Registration uses Cinnamon's last-registered-listener priority; this is a
compatibility experiment, not a guarantee of exclusive delivery. The production
default backend uses this service; this probe remains a separate log-only
diagnostic and does not exercise routing or automatic reclaim.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
