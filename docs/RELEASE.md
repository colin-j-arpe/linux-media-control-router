# Release preparation

## Preparing release 0.2.1

Both Cargo packages declare version `0.2.1`. Release notes are in
[CHANGELOG.md](../CHANGELOG.md). This release includes paired daemon/tray
autostart and structured code documentation.

The existing `v0.2.0` tag points to the autostart merge and has already been
pushed, but the user reported that its release was not published. Preserve that
tag and publish `v0.2.1` from the final release commit, including the documentation
updates and version metadata.

Version `0.1.0` was published, as reported by the user on 2026-10-07:
<https://github.com/colin-j-arpe/linux-media-control-router/releases/tag/v0.1.0>.
Its autostart command registers only the daemon. Do not replace its published
assets with builds containing subsequent changes.

Capture support remains limited to Cinnamon/X11. Binary archives target Linux
Mint 22.3 x86-64; other desktop and distribution support is not claimed.

## Validation

From the repository root, run each command successfully before continuing:

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all --check
cargo build --release --workspace --locked
target/release/media-router --help
target/release/media-router-tray --help
ldd target/release/media-router
ldd target/release/media-router-tray
git diff --check
```

Check that neither `ldd` output contains a missing library. Integration tests
require the dependencies and local socket permissions listed in the README.

On 2026-10-12, the prepared 0.2.1 source passed all 96 workspace tests, Clippy
with warnings denied, formatting, rustdoc with warnings denied, and both
release builds. Both executables passed help checks, and all shared libraries
resolved on the Linux Mint 22.3 x86-64 build host. These checks preceded the
release commit; repeat validation if merged source or metadata changes.

The user has tested installing and uninstalling paired daemon/tray autostart.
Release validation must still run on the final release source; the historical
0.1.0 validation results do not validate 0.2.1. Preparing or packaging a release
does not install executables, enable autostart, or change desktop settings.

## Binary archive

Run the following on Linux Mint 22.3 x86-64 after the release build. The archive
contains native dynamically linked executables; it is not a universal Linux
binary. Package again after any executable or included-documentation change.

Use a fresh staging directory for this version so no stale files are included.

```sh
release_name=media-router-0.2.1-linuxmint22.3-x86_64
dist_dir=target/release-dist/v0.2.1

mkdir -p "$dist_dir/$release_name/bin" "$dist_dir/$release_name/docs"
cp target/release/media-router target/release/media-router-tray "$dist_dir/$release_name/bin/"
cp README.md CHANGELOG.md LICENSE "$dist_dir/$release_name/"
cp docs/DBUS_API.md docs/org.mediarouter.MediaRouter1.xml "$dist_dir/$release_name/docs/"
tar -czf "$dist_dir/$release_name.tar.gz" -C "$dist_dir" "$release_name"
(
    cd "$dist_dir"
    sha256sum "$release_name.tar.gz" > SHA256SUMS
    sha256sum -c SHA256SUMS
)
```

The archive and `SHA256SUMS` under `target/release-dist/v0.2.1` are upload assets, not
tracked source files. The project context is an internal development record
and is not included. Do not distribute the staging directory separately.

## Publication checklist

- Review and commit the release metadata and documentation. The user handles
  Git commits, merges, tags, and pushes; none are performed by the assistant.
- Merge the release preparation into `main`, update the local checkout, and
  ensure the working tree is clean. Validate, rebuild, and package from that
  exact commit if source or included documents changed after preparation.
- Confirm `v0.2.1` does not already exist locally or remotely.
- Tag the final release commit and push only the new tag:

  ```sh
  git tag -a v0.2.1 -m "Media Router 0.2.1"
  git show --no-patch v0.2.1
  git push origin v0.2.1
  ```

- On GitHub, draft a release using the existing `v0.2.1` tag and title
  `Media Router 0.2.1`. Use the `0.2.1` changelog section as its notes.
- Upload `media-router-0.2.1-linuxmint22.3-x86_64.tar.gz` and `SHA256SUMS`,
  review the complete draft, then publish. The host's source archives should
  reference the same tag. Leave `v0.2.0` unchanged.
- Download the published assets into a fresh directory and verify them:

  ```sh
  sha256sum -c SHA256SUMS
  tar -tzf media-router-0.2.1-linuxmint22.3-x86_64.tar.gz
  ```

Publication is separate from local preparation; building and packaging do not
create a remote release.
