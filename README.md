# Media Router

A Linux media-control router intended to let users choose which running
media application receives Play/Pause, Stop, Previous, and Next commands.
The initial target is Linux Mint Cinnamon on X11.

## Building from source

Install Rust using rustup. The repository's `rust-toolchain.toml` selects
Rust 1.98.1 with rustfmt and Clippy.

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
Restart the command with another selection argument to change it, or with
no argument to observe players without selecting one. Selection currently
reports availability; add `--interactive` to send transport commands.
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
to finish pending commands and exit, or press Ctrl+C to exit immediately.
An in-flight command may already have executed when interrupted.

The application does not capture hardware keys or change desktop keybindings.
Volume and mute are outside its transport command set.

## Contributing

Read [the project context](docs/PROJECT_CONTEXT.md) before making
architectural changes.

The package uses Rust edition 2024. Run the checks from the repository root:

```sh
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --check
```

Integration tests require `dbus-daemon` and permission to create local
Unix sockets. They start isolated session buses and do not use your
desktop session bus. Unit tests alone can be run with `cargo test --lib
--locked` in environments that cannot create sockets.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
