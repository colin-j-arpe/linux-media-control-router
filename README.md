# Media Router

A Linux media-control router intended to let users choose which running
media application receives Play/Pause, Stop, Previous, and Next commands.
The initial target is Linux Mint Cinnamon on X11.

## Building from source

Install Rust and Cargo, with the rustfmt component for formatting checks.

From the repository root:

```sh
cargo build --locked
cargo run --locked
cargo fmt --check
```

The debug executable is written to `target/debug/media-router`.

## Contributing

Read [the project context](docs/PROJECT_CONTEXT.md) before making
architectural changes.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
