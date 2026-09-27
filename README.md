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

If a player does not supply `DesktopEntry`, its service name is used only
as a diagnostic label. It is not a persistent application identifier.

This command observes players; it does not send playback commands or
change desktop keybindings.

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
