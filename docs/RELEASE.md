# Release preparation

Version `0.1.0` has been published, as reported by the user on 2026-10-07:
<https://github.com/colin-j-arpe/linux-media-control-router/releases/tag/v0.1.0>.
The instructions below describe that release. Paired daemon/tray autostart is
an unreleased change recorded in CHANGELOG.md; select a new version and update
archive names before packaging the next release. Do not replace the published
0.1.0 assets with a build containing subsequent changes.

## Preparing release 0.1.0

Both Cargo packages already declare version `0.1.0`. Release notes are in
[CHANGELOG.md](../CHANGELOG.md). This is an initial Cinnamon/X11 release;
other desktop and distribution support is not claimed.

## Validation

From the repository root:

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all --check
cargo build --release --workspace --locked
target/release/media-router --help
target/release/media-router-tray --help
git diff --check
```

On 2026-10-05, all 94 workspace tests, Clippy with warnings denied, formatting,
and both release builds passed. Both release executables passed help smoke
checks, and their shared libraries resolved on the build host. User-reported
login startup and removal checks also passed. Autostart is currently disabled
on the user's system; preparing a release does not change that setting.

## Binary archive

Run the following on Linux Mint 22.3 x86-64 after the release build. The archive
contains native dynamically linked executables; it is not a universal Linux
binary. Package again after any executable or included-documentation change.

```sh
mkdir -p target/release-dist/media-router-0.1.0-linuxmint22.3-x86_64/bin
mkdir -p target/release-dist/media-router-0.1.0-linuxmint22.3-x86_64/docs
cp target/release/media-router target/release/media-router-tray target/release-dist/media-router-0.1.0-linuxmint22.3-x86_64/bin/
cp README.md CHANGELOG.md LICENSE target/release-dist/media-router-0.1.0-linuxmint22.3-x86_64/
cp docs/DBUS_API.md docs/org.mediarouter.MediaRouter1.xml target/release-dist/media-router-0.1.0-linuxmint22.3-x86_64/docs/
tar -czf target/release-dist/media-router-0.1.0-linuxmint22.3-x86_64.tar.gz -C target/release-dist media-router-0.1.0-linuxmint22.3-x86_64
(cd target/release-dist && sha256sum media-router-0.1.0-linuxmint22.3-x86_64.tar.gz > SHA256SUMS)
(cd target/release-dist && sha256sum -c SHA256SUMS)
```

The archive and `SHA256SUMS` under `target/release-dist` are upload assets, not
tracked source files. The project context is an internal development record
and is not included. Do not distribute the staging directory separately.

## Publication checklist

- Review and commit the release documentation. The user handles Git commits,
  merges, tags, and pushes; none are performed by the assistant.
- Ensure the release commit is on the intended branch and the working tree is
  clean. Rebuild and repackage from that commit if source or included documents
  changed after preparation.
- Tag the release commit `v0.1.0` and push the tag when ready to publish.
- Create the repository release titled `Media Router 0.1.0`, using the `0.1.0`
  release notes. Upload the archive and its `SHA256SUMS` file. The repository
  host's source archives should reference the same tag.
- Download the published assets and verify their checksum. Publication remains
  a separate step; local preparation does not create a remote release.
