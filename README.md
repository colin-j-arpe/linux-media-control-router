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

Selection lasts for the current router process and is not saved to disk.
Restart the command with another selection argument to change it, or use
the D-Bus API when running with `--serve`. With no selection argument,
startup displays players without selecting one. Selection currently
reports availability; add `--interactive` to send terminal commands or
`--capture` to route hardware transport keys.
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

Without `--capture`, terminal commands do not change desktop keybindings.
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
even across logout. Recovery runs before the next capture session, or run:

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
There is no watchdog for immediate crash recovery.

## Controlling the running daemon

```sh
cargo run --locked -- --serve
```

`--serve` exposes the session-bus service `org.mediarouter.MediaRouter1`.
It lets clients read applications and availability, change or clear the
selection, enable/disable capture, and receive state updates. Capture
starts disabled unless `--capture` is also supplied. Standard input closing
does not stop a serving daemon. Press Ctrl+C to stop and restore bindings.

For example, use these optional diagnostic commands from another terminal:

```sh
busctl --user call org.mediarouter.MediaRouter1 /org/mediarouter/MediaRouter1 org.mediarouter.MediaRouter1 SelectApplication ss desktop-entry spotify
busctl --user call org.mediarouter.MediaRouter1 /org/mediarouter/MediaRouter1 org.mediarouter.MediaRouter1 SetCaptureEnabled b true
```

Only one API server can run on a session bus. Snapshots and revision numbers
remain in RAM; selection and preferences are not yet saved across router
restarts. See the [D-Bus API reference](docs/DBUS_API.md) for the full
contract, recovery behavior, client synchronization, and examples.

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

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
