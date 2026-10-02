# Media Router --- Project Context

## Purpose

This document is the development handoff/context for a Linux
media-control routing project. It records the requirements, architecture
decisions, and experimentally verified behavior established before
implementation began.

Treat items identified as **settled** or **verified** as established
project requirements/evidence. Do not reopen them without new
implementation evidence showing a problem.

## Project Goal

Build a Linux background daemon plus tray UI that lets the user
explicitly choose which currently available media application receives
hardware media transport controls when multiple media-capable
applications are open.

The initial target environment is Linux Mint 22.3 Cinnamon on X11, but
the architecture should avoid unnecessary Cinnamon/X11 coupling so that
other Linux desktops and Wayland can be supported later.

## Version 1 Scope

Intercept and route only these transport controls:

-   Play/Pause
-   Stop
-   Previous
-   Next

Do **not** intercept volume up, volume down, or mute. Existing desktop
behavior for those controls should remain unchanged.

The application should eventually:

-   run automatically in the background at login;
-   provide a system-tray icon;
-   list eligible running media applications;
-   let the user explicitly select one application as the media-key
    destination;
-   route transport commands only to that selected application;
-   persist the selected application identity when the selected
    application closes;
-   automatically resume routing to that application when it returns;
-   visually indicate when the persistently selected application is
    currently unavailable;
-   support an optional auto-select-new-media-application behavior;
-   ultimately be distributable conveniently to other Linux users.

Individual browser-tab selection is **not** part of the basic MPRIS v1
implementation.

## Development Direction

### Language

The daemon will be written in **Rust**.

The user chose **Rust** for the separate tray UI. It will communicate with
the daemon through the stable D-Bus API. Keeping the UI separate from the
routing daemon is intentional. The first authorized tray step is a Cinnamon
compatibility probe before committing to a tray library and full UI design.

### Development Environment

Development is intended to take place in **JetBrains RustRover**, using
Cargo and Git.

## Target Test System

Verified development/test environment:

-   Linux Mint 22.3
-   Cinnamon desktop
-   X11 session
-   Intel Core i9-11900K
-   ASUS Prime Z590-A
-   128 GB RAM
-   Radeon HD 7750 graphics
-   Multiple physical media-control devices

Relevant physical input devices tested:

-   Logitech G510s Gaming Keyboard
-   Creative Prodikeys PC-MIDI
-   OZU OZU8 standalone USB media controller

The daemon must not depend on these specific devices.

------------------------------------------------------------------------

# Verified Input Pipeline

## Linux evdev

Working media controls from the tested devices converge on the standard
Linux input events:

  Action       EV_KEY
  ------------ --------------------------
  Play/Pause   `KEY_PLAYPAUSE` (164)
  Previous     `KEY_PREVIOUSSONG` (165)
  Next         `KEY_NEXTSONG` (163)
  Stop         `KEY_STOPCD` (166)

Observed HID scan codes:

  Action       Scan code
  ------------ -----------
  Play/Pause   `c00cd`
  Previous     `c00b6`
  Next         `c00b5`
  Stop         `c00b7`

The OZU8 has no physical Stop control. The old Creative keyboard's Next
button generated no event, apparently due to hardware failure. Neither
exception requires software accommodation.

### Settled conclusion

The router does **not** need per-device knowledge for Cinnamon/X11.
Working hardware converges to standard media-key events before the
daemon needs to interact with them.

The daemon therefore should not require:

-   raw `/dev/input` access;
-   root privileges;
-   input-device permissions;
-   device hotplug enumeration;
-   per-keyboard mappings.

------------------------------------------------------------------------

# Verified X11/Cinnamon Input Behavior

## X11 keysyms

Relevant X11 mappings observed:

-   `XF86AudioNext`
-   `XF86AudioPlay`
-   `XF86AudioPause`
-   `XF86AudioPrev`
-   `XF86AudioStop`

Volume controls map separately to:

-   `XF86AudioMute`
-   `XF86AudioLowerVolume`
-   `XF86AudioRaiseVolume`

The G510s combined Play/Pause key produced X11 keycode 172 /
`XF86AudioPlay`.

Other hardware may emit a dedicated `XF86AudioPause`; the daemon should
normalize both Play and Pause keysyms to the logical `PLAY_PAUSE`
action.

## Cinnamon global grabs

With Cinnamon's normal configuration active, focused
`xev -event keyboard` did **not** receive transport media keys.

The relevant active Cinnamon schema is:

`org.cinnamon.desktop.keybindings.media-keys`

Verified default values:

``` text
play      ['XF86AudioPlay']
pause     ['XF86AudioPause']
stop      ['XF86AudioStop']
previous  ['XF86AudioPrev']
next      ['XF86AudioNext']
```

Volume/mute bindings are also present in this schema but are outside
this project's routing scope.

Temporarily setting a transport binding to an empty array causes
Cinnamon to release that X11 grab dynamically.

Example:

``` bash
gsettings set org.cinnamon.desktop.keybindings.media-keys play "[]"
```

After doing this, `xev` received the G510s Play/Pause key as
`XF86AudioPlay`.

Releasing all five transport bindings allowed `xev` to receive all
tested transport keys:

-   Play/Pause → `XF86AudioPlay`
-   Stop → `XF86AudioStop`
-   Previous → `XF86AudioPrev`
-   Next → `XF86AudioNext`

Volume controls continued to work normally while transport bindings were
released.

### Settled conclusion

For Cinnamon/X11, the input architecture is:

``` text
Physical controller
    ↓
Linux evdev standard media key
    ↓
X11 XF86Audio*
    ↓
Cinnamon global keybinding grab
    ↓
future router daemon after selectively releasing Cinnamon transport grabs
```

The daemon should take ownership only of the transport controls and
leave volume/mute under Cinnamon control.

## Configuration-safety requirement

The daemon must **not** blindly reset Cinnamon keybindings to defaults
on exit.

It must:

1.  read and preserve the exact pre-existing values for the transport
    bindings;
2.  release only the bindings it intentionally owns;
3.  restore the exact prior values when routing is disabled/uninstalled;
4.  account for abnormal termination/crash recovery;
5.  avoid overwriting user changes made while the daemon is running
    where practical.

This needs deliberate design before the X11/Cinnamon backend is
considered production-ready.

------------------------------------------------------------------------

# Verified MPRIS Behavior

`playerctl` was used only as an initial diagnostic utility. It is **not
intended as a runtime dependency**.

Observed MPRIS bus names included:

``` text
org.mpris.MediaPlayer2.spotify
org.mpris.MediaPlayer2.firefox.instance_1_109
org.mpris.MediaPlayer2.chromium.instance552725
org.mpris.MediaPlayer2.brave.instance20833
```

## Valid players versus phantom names

Spotify and Firefox exposed complete MPRIS objects at:

`/org/mpris/MediaPlayer2`

including the standard:

`org.mpris.MediaPlayer2.Player`

interface.

The Chromium and Brave bus names, in the tested state, existed on D-Bus
but exposed no interfaces at the standard MPRIS object path.

Therefore **bus-name presence alone does not make a player eligible**.

Initial eligibility model:

``` text
bus name matches org.mpris.MediaPlayer2.*
    ↓
standard /org/mpris/MediaPlayer2 object is usable
    ↓
org.mpris.MediaPlayer2.Player is usable
    ↓
CanControl == true
    ↓
eligible routing target
```

This may be refined for standards-compliant edge cases during
implementation.

## Useful MPRIS properties

Verified properties include:

### `org.mpris.MediaPlayer2`

-   `Identity`
-   `DesktopEntry`

Observed:

``` text
Spotify:
  Identity     = "Spotify"
  DesktopEntry = "spotify"

Firefox:
  Identity     = "Mozilla firefox"
  DesktopEntry = "firefox"
```

### `org.mpris.MediaPlayer2.Player`

Relevant properties/methods include:

-   `PlaybackStatus`
-   `Metadata`
-   `CanControl`
-   `CanPlay`
-   `CanPause`
-   `CanGoNext`
-   `CanGoPrevious`
-   `CanSeek`
-   `PlayPause()`
-   `Play()`
-   `Pause()`
-   `Stop()`
-   `Next()`
-   `Previous()`

MPRIS does **not** define a `CanStop` property. Do not invent one as
though it were part of the standard.

## Direct D-Bus routing verified

Direct `busctl` calls to:

`org.mpris.MediaPlayer2.Player.PlayPause`

successfully controlled Spotify and Firefox independently.

Examples tested conceptually:

``` text
target Spotify
  PlayPause → Spotify changes state; Firefox unaffected

target Firefox
  PlayPause → Firefox changes state; Spotify unaffected
```

Therefore the production daemon can invoke MPRIS directly and does not
require `playerctl`.

------------------------------------------------------------------------

# Browser Behavior

Firefox exposed one MPRIS player for the browser instance even with
multiple media tabs open.

Experiment:

1.  A YouTube tab was played/paused using its web UI.
2.  Direct MPRIS commands to Firefox controlled YouTube.
3.  A SoundCloud tab was then made the active/recent media tab and
    played/paused.
4.  The same Firefox MPRIS commands then controlled SoundCloud instead.
5.  YouTube no longer responded to those commands.

### Settled conclusion for v1

Firefox/browser selection is at the **logical MPRIS application/player
level**, not individual browser-tab level.

For v1, selecting Firefox means routing to Firefox's MPRIS player;
Firefox decides which internal media session/tab responds.

Individual browser-tab routing would require a later browser-integration
layer and is not part of the basic MPRIS daemon.

------------------------------------------------------------------------

# Event-Driven Player Discovery

Polling is not required for normal player discovery/state tracking.

## Appearance/disappearance

D-Bus `org.freedesktop.DBus.NameOwnerChanged` was verified when Spotify
exited and restarted.

On disappearance, conceptually:

``` text
name      = org.mpris.MediaPlayer2.spotify
old_owner = :1.xxx
new_owner = ""
```

On appearance:

``` text
name      = org.mpris.MediaPlayer2.spotify
old_owner = ""
new_owner = :1.yyy
```

The daemon should:

### At startup

1.  enumerate current `org.mpris.MediaPlayer2.*` names;
2.  validate candidates;
3.  construct player-instance records.

### While running

Subscribe to `NameOwnerChanged`:

``` text
MPRIS name gains owner
    → validate
    → add/update instance

MPRIS name loses owner
    → remove instance
```

A newly appearing player may still be initializing. Validation should
tolerate short-lived startup errors rather than permanently rejecting a
service after one failed query.

## Player state changes

MPRIS players emit:

`org.freedesktop.DBus.Properties.PropertiesChanged`

Spotify was verified to emit changes for:

-   `PlaybackStatus`
-   `Metadata`
-   `CanSeek`

A media transition can generate several successive signals rather than
one atomic "track changed" event.

The daemon should update only the properties supplied by each event and
retain the rest of its current state.

`Seeked` signals were also observed but are not required for v1 routing.

------------------------------------------------------------------------

# Selection Model

## Settled: Persistent Application Selection

Selection belongs to a **persistent logical application identity**, not
to an ephemeral MPRIS bus instance.

`DesktopEntry` is the preferred persistent application identity when
available.

Settled during Milestone 3: players without a nonempty `DesktopEntry`
remain selectable using their exact MPRIS `Identity` string as a fallback.
Desktop-entry IDs and fallback identities are distinct identifier types.
Identity matching is case-sensitive, with no trimming or normalization.
Equal fallback identities group instances as one logical application;
a changed Identity string requires reselection. This limitation was
explicitly accepted by the user.

For multiple instances of the selected application, keep the first
validated matching instance while it is available. When it disappears,
choose among the remaining known eligible instances of that same
application using service-name ordering. If selection is made after
instances are already known, also use service-name ordering. New arrivals
do not displace a still-available active instance.

Conceptually:

``` text
selected_application = "spotify"
```

while Spotify is running:

``` text
active_instance = Some("org.mpris.MediaPlayer2.spotify")
```

after Spotify exits:

``` text
selected_application = "spotify"
active_instance = None
```

when Spotify returns:

``` text
selected_application = "spotify"
active_instance = Some(current Spotify MPRIS service)
```

Transport keys should do nothing while the selected application is
unavailable. The daemon must **not silently select another application
merely because the selected application closed**.

When the selected application returns, routing resumes automatically.

## Tray unavailable-state requirement

When the persistently selected media application is closed/unavailable,
the tray icon should visibly indicate that condition.

Suggested design:

-   red exclamation-point overlay in the upper-right corner of the
    normal icon; or
-   a similarly clear warning/unavailable visual treatment.

The warning clears automatically when the selected application becomes
available again.

The exact artwork is not yet fixed; the **requirement for a clear visual
unavailable-state indicator is fixed**.

------------------------------------------------------------------------

# Auto-Selection

Approved v1 preference:

-   optionally auto-select a newly opened media application (MRO).

Most-recently-played selection (MRP) is deferred to a future version; see
the proposal below. It is not part of Milestone 7 or the v1 settings schema.

A related desired setting was originally described as excluding new
browser tabs from auto-selection. Because standard Firefox MPRIS does
not expose individual tabs as separate players, browser-tab-specific
behavior cannot be implemented by the basic MPRIS layer.

Auto-selection semantics should therefore initially operate on logical
discovered applications/MPRIS players.

Important settled rule:

**The disappearance of the currently selected application does not
itself trigger selection of another application.**

Approved MRO precedence and exclusions are recorded in the Milestone 7
policies below.

------------------------------------------------------------------------

# Proposed Daemon Architecture

Keep desktop-independent routing logic separate from platform input
backends.

Conceptual structure:

``` text
media-routerd
│
├── player/
│   ├── discovery
│   ├── mpris
│   └── selection
│
├── input/
│   ├── abstraction
│   └── x11_cinnamon
│
├── config/
│
└── dbus_api/
```

## Player manager

Responsibilities:

-   enumerate MPRIS candidates;
-   validate real usable players;
-   read Identity/DesktopEntry/status/capabilities;
-   subscribe to appearance/disappearance;
-   subscribe to relevant property changes;
-   map transient instances to logical applications.

## Selection manager

Maintain the distinction:

``` text
SelectedApplication
    ≠
ActivePlayerInstance
```

It should resolve the persistent selection to a current validated MPRIS
instance when one exists.

## Normalized input actions

Internal transport actions should be desktop/device independent:

``` text
PLAY_PAUSE
STOP
PREVIOUS
NEXT
```

Both `XF86AudioPlay` and `XF86AudioPause` should normalize to
`PLAY_PAUSE`.

## MPRIS command routing

Conceptually:

``` text
route(action):
    if no selected application:
        ignore

    if selected application has no current instance:
        ignore

    if instance cannot be controlled:
        ignore/report state

    invoke corresponding MPRIS Player method
```

Mappings:

``` text
PLAY_PAUSE → PlayPause()
STOP       → Stop()
PREVIOUS   → Previous()
NEXT       → Next()
```

## Input backend abstraction

Initial backend:

-   Cinnamon/X11

Future possible backends:

-   other X11 desktop environments;
-   Wayland/desktop-specific mechanisms;
-   evdev fallback only if genuinely necessary.

Do not spread Cinnamon/X11-specific assumptions throughout the core
daemon.

------------------------------------------------------------------------

# Daemon D-Bus API

Milestone 6 implements the versioned session-bus API documented in
[DBUS_API.md](DBUS_API.md), with its machine-readable contract in
[org.mediarouter.MediaRouter1.xml](org.mediarouter.MediaRouter1.xml).

The bus name and interface are `org.mediarouter.MediaRouter1`; the object
path is `/org/mediarouter/MediaRouter1`. `--serve` exposes the API explicitly.
It provides `GetState`, `SelectApplication`, `ClearSelection`, and
`SetCaptureEnabled`, with complete revisioned `StateChanged` signals.

The daemon owns discovery, selection, routing, and capture. A future tray
client reads state and sends requests over D-Bus. Snapshot/revision storage
is in RAM, without history; clients resynchronize when the owner changes.
Milestone 7 adds GetSettings, SetAutoSelectNew, SetAutoSelectExclusions,
and SettingsChanged using the approved MRO semantics; existing State wire
signatures remain unchanged. There are no browser-tab controls in this API.

------------------------------------------------------------------------

# Implementation Order

## Milestone 1 --- Rust/Cargo skeleton

Create a normal Rust binary project and Git repository.

Do not prematurely add large frameworks.

## Milestone 2 --- MPRIS discovery prototype

First working component should:

1.  connect to the user session D-Bus;
2.  enumerate `org.mpris.MediaPlayer2.*`;
3.  validate actual MPRIS Player interfaces;
4.  read `Identity`, `DesktopEntry`, `PlaybackStatus`, and relevant
    capabilities;
5.  print clean player-added/player-removed events;
6.  subscribe to `NameOwnerChanged`.

Expected diagnostic behavior should resemble:

``` text
PLAYER ADDED
  application: spotify
  identity: Spotify
  service: org.mpris.MediaPlayer2.spotify

PLAYER ADDED
  application: firefox
  identity: Mozilla firefox
  service: org.mpris.MediaPlayer2.firefox.instance_1_109

PLAYER REMOVED
  application: spotify

PLAYER ADDED
  application: spotify
  service: org.mpris.MediaPlayer2.spotify
```

## Milestone 3 --- Persistent selection

Implement persistent logical application selection separately from
runtime instance identity.

Initially diagnostic output is sufficient for unavailable state.

## Milestone 4 --- MPRIS command routing

Implement the four normalized transport actions and direct MPRIS method
calls.

A temporary CLI/testing surface may be used before the permanent daemon
D-Bus API exists.

## Milestone 5 --- Cinnamon/X11 input backend

Acquire only transport media keys after safely releasing Cinnamon's
corresponding global bindings.

Leave volume/mute untouched.

Design restoration/crash-recovery behavior carefully before considering
this complete.

## Milestone 6 --- Daemon D-Bus API

Expose player inventory, persistent selection, availability state,
settings, and change signals.

## Milestone 7 --- Persistence/startup

Add:

-   configuration storage;
-   selected-application persistence;
-   auto-selection preferences;
-   desktop-session startup;
-   clean shutdown;
-   abnormal-termination recovery for Cinnamon keybindings.

## Milestone 8 --- Tray client

Implement the tray UI against the daemon API.

Include the selected-application-unavailable visual warning state.

Portability takes priority over identical interaction behaviour across Linux
distributions and desktop environments (user decision, 2026-10-02). Use the
standard StatusNotifierItem/D-Bus menu approach with Rust/ksni and let the host
render the menu. Left-click opening and keeping the menu open after selections
are optional, host-dependent conveniences, not acceptance requirements.
Right-click access and reopening after selections are acceptable. Do not add
Cinnamon-specific workarounds or an application-owned popup to enforce these
conveniences. Tray availability still requires a compatible host; compatibility
must be tested per environment rather than inferred from protocol support.

------------------------------------------------------------------------

# Runtime Dependency Principle

`playerctl`, `busctl`, `xev`, `evtest`, `xinput`, and similar utilities
were diagnostic tools used to establish behavior.

The finished application should not depend on them merely because they
were used during investigation.

Use native Rust/D-Bus/X11 APIs for production functionality.

------------------------------------------------------------------------

# Immediate Development Task

Milestones 1 through 5 are complete. The user confirmed a successful
physical-key test through the CLI and that all changes were committed and
merged into `main`.
Milestone 6 is complete; the user confirmed that CLI service commands
function as expected. The user committed, pushed, and merged Milestone 6
into `main`, then authorized Milestone 6.1 for CLI abbreviations on
`feature/6.1-cli-switches`. Milestone 6.1 is complete; the user confirmed
that its changes were committed, pushed, and merged into `main`. The user
authorized Milestone 7 (persistence/startup) on 2026-10-01, on branch
`feature/7-startup`. The user approved the original persistence, MRO
auto-selection, and startup policies, and deferred MRP to a future version.
Milestone 7 implementation and isolated automated verification are complete:
configuration storage, daemon/API persistence, MRO selection, opt-in login
startup commands, and recovery on serving startup even with capture disabled.
The user confirmed that all three implementation steps are committed;
merging to main has not been reported. The user will test installation after
the system-tray interface is complete. No live autostart installation or
actual login check has been performed. MRP remains a documented future proposal.
The user chose Rust for Milestone 8 and authorized a tray compatibility
test. Full tray-client implementation remains a later approval checkpoint.

Before adding dependencies, verify current Rust crate choices and
versions rather than relying on old examples.

Likely areas to evaluate:

-   `zbus` for D-Bus/MPRIS and the daemon's own D-Bus service;
-   an appropriate maintained X11/XCB Rust crate for the Cinnamon/X11
    input backend.

The first implementation should **not** attempt to solve the tray UI,
Wayland support, browser-tab integration, or production Cinnamon
keybinding recovery simultaneously.

Build and verify one layer at a time.

------------------------------------------------------------------------

# Development Progress

## Milestone 1 completed — 2026-09-26

The repository already contained a normal Cargo binary skeleton and an
initialized Git repository. The existing skeleton was inspected and
verified without regenerating it.

- Package: `media-router`, version `0.1.0`, Rust edition `2021`.
- Entry point: `src/main.rs`, currently prints `Hello, world!` and exits.
- No external dependencies have been added.
- `Cargo.lock` is tracked; `.gitignore` excludes `/target`.
- The placeholder README was replaced with a project description,
  build commands, and a link to the architectural context. Following
  user review, progress reports and tutorial explanations were removed
  from the README.
- Verified using installed `rustc 1.82.0` and `cargo 1.82.0`:
  `cargo build --locked --offline`, `cargo run --locked --offline`, and
  `cargo fmt --check` all succeeded. The run printed `Hello, world!`.

This verifies the skeleton only; MPRIS discovery, selection, routing,
input capture, and the tray UI remain unimplemented. The tested toolchain
version is not a minimum supported Rust version commitment. Check current
crate versions and their compiler requirements before adding dependencies
in Milestone 2.

## Milestone 2 completed — 2026-09-26

The original installed toolchain was Rust/Cargo 1.82.0. Published `zbus`
documentation identifies version 5.19.0, whose manifest requires Rust
1.87 and uses Rust edition 2024:
`https://docs.rs/crate/zbus/5.19.0/source/Cargo.toml`.

The original compiler cannot build that release. The user approved a
toolchain upgrade. Rust 1.98.1, identified by the official stable channel
manifest, was installed alongside the existing toolchain. The repository's
`rust-toolchain.toml` selects 1.98.1 with rustfmt and Clippy; the global
default toolchain was left unchanged.

Verified `rustc`, Cargo, rustfmt, and Clippy versions, then successfully
ran `cargo build --locked --offline`, `cargo run --locked --offline`, and
`cargo fmt --check` with the new toolchain. Rustup reported an error during
its subsequent self-update step, after installing the toolchain; the
installed tools and project build were independently verified to work.

The user approved Rust edition 2024. Ran `cargo fix --edition` with the
new toolchain, changed `Cargo.toml` to edition 2024, and verified the build
and formatting. The skeleton required no source changes.

### Approved implementation choices

- Use `zbus` with Tokio for asynchronous discovery, timeouts, and retries.
- Preserve missing `DesktopEntry` as `None`. Display the service name as
  a diagnostic label without assigning it persistent identity semantics;
  fallback identity rules remain a Milestone 3 decision.
- Direct dependencies, verified before addition and resolved in the
  lockfile: `zbus 5.19.0`, `tokio 1.53.1`, and `futures-util 0.3.34`.
  Tokio uses the current-thread runtime; zbus's default runtime features
  are disabled in favor of its Tokio integration.

### Implemented behavior and structure

- `src/player/mod.rs`: desktop-independent player snapshots, an optional
  desktop-entry identifier, playback status, and capabilities. Distinct
  service instances remain distinct even if they share `DesktopEntry`.
- `src/player/mpris.rs`: native D-Bus property queries at the standard
  MPRIS object path, strict property type validation, and `CanControl`
  eligibility. No `CanStop` property is assumed.
- `src/player/discovery.rs`: subscribe to `NameOwnerChanged` before
  enumerating existing services, then reconcile additions, removals,
  and owner replacements. No periodic discovery polling is used.
- Validation queries target unique owners. Generation numbers, task
  cancellation, and an owner check before admission prevent obsolete
  validation results from being accepted after an ownership change.
- Startup validation permits five attempts with a two-second limit per
  attempt and delays of 100, 250, 500, and 1000 milliseconds between
  attempts. A fully unresponsive service can therefore take about twelve
  seconds to reject. Other player validations continue concurrently.
- An explicitly uncontrollable player is skipped without retries.
  Exhausted failures are reported once and retried when ownership changes
  or the prototype restarts, not indefinitely in the background.
- `src/main.rs`: diagnostic added/removed/skipped events, a two-second
  D-Bus method timeout, and Ctrl+C shutdown. Bus loss ends discovery with
  an error; automatic reconnection is not implemented.

### Verification

- Final checks passed: `cargo test --locked --offline` (all nine tests),
  `cargo clippy --all-targets --locked --offline -- -D warnings`,
  `cargo fmt --check`, and `git diff --check`.
- Three unit tests cover missing/empty desktop entries, malformed property
  types, missing capabilities/identity, and invalid playback status.
- Six integration tests use private D-Bus daemons to cover existing
  instances, shared desktop-entry values, unrelated names, missing desktop
  entries, delayed initialization, departure/return, phantom services,
  uncontrollable players, slow validation, owner replacement, and bus loss.
- A read-only desktop-session run discovered Spotify, reporting its
  `spotify` desktop entry, identity, paused status, and capabilities. The
  process exited successfully on SIGINT. No playback commands were sent.
- Integration tests require socket creation outside the restricted
  sandbox. `dbus-daemon` is a test dependency, not an application runtime
  subprocess dependency.

### Scope remaining

The prototype takes property snapshots at admission. Continuous
`PropertiesChanged` tracking, including partial updates and invalidated
properties, still needs implementation before relying on live capability
or playback state for routing/UI. Metadata tracking is also deferred.
At completion of Milestone 2, persistent selection, command routing, key
capture, the daemon control API, and tray UI were unimplemented. Selection
was subsequently added in Milestone 3. The existing settled requirements
for the remaining layers are unchanged.

## Milestone 3 completed — 2026-09-26

The user authorized work on persistent logical application selection.
The discovery implementation has been reviewed. The established rule
remains: losing the selected application must preserve its logical
selection and must not select a different application.

Approved selection requirements:

- Players without `DesktopEntry` must remain available for persistent
  selection. Do not exclude them merely because that property is absent.
- Keep the first validated matching instance while it remains available.
  When it disappears, choose another currently known instance of the same
  application using service-name ordering. If no matching instance remains,
  preserve the selected application and report it unavailable.
- This milestone retains selection across player restarts while the
  router runs. Disk persistence across router restarts remains in
  Milestone 7. A temporary CLI selection argument is approved.

The user approved exact MPRIS `Identity` matching as the fallback when
`DesktopEntry` is absent, with separate identifier types. Equal fallback
identities group instances; a changed identity requires reselection.

### Implemented behavior and structure

- `src/player/selection.rs` defines `ApplicationId::DesktopEntry` and
  `ApplicationId::Identity`, preferring a nonempty desktop entry when
  available. Missing/empty desktop entries use the exact identity string.
- The selection manager retains user intent independently of the active
  service/owner pair. It consumes discovery events and reports only
  changes in selection state: unselected, unavailable, or available.
- A `BTreeMap` provides service-name ordering when resolving a selection
  without an active instance. A still-eligible active instance is retained.
- Stale removal events cannot delete a newer owner of the same service.
  Discovering other applications never changes the selected application.
- The core supports selecting an absent application, changing selection,
  and clearing it. It performs no D-Bus I/O or disk writes.
- `src/cli.rs` adds `--select DESKTOP_ENTRY`, `--select-identity IDENTITY`,
  and `--help`, using the standard library without new dependencies.
  Invalid arguments fail before attempting a bus connection.
- `src/main.rs` applies discovery events to the selection manager and
  prints availability changes. The initial unavailable state means the
  selected application has not yet been discovered/validated.
- The CLI chooses selection at startup. To change/clear it through the
  CLI, restart with a different argument or no selection argument.
  Runtime control through the daemon API remains a later milestone.

### Verification

- All 22 tests passed: 11 library unit tests, three CLI parser unit tests,
  and eight integration tests. Eight library tests exercise selection.
- The executable-level lifecycle test runs both desktop-entry and fallback
  selection on private buses. It verifies absent selection, first arrival,
  retaining the active instance as more arrive, ordered replacement,
  complete disappearance, return under a new service name/owner, and a
  router restart without saved selection. Other applications stay present
  during these transitions and are never selected as replacements.
- Additional tests cover exact/case-sensitive fallback matching,
  identifier-type separation, stale removals, clearing/changing selection,
  malformed arguments, and help without a usable session bus.
- `cargo build --locked --offline`,
  `cargo clippy --all-targets --locked --offline -- -D warnings`,
  `cargo fmt --check`, and `git diff --check` passed.
- A read-only desktop run with `--select spotify` transitioned from
  unavailable to available after validating Spotify. It exited on SIGINT
  without sending playback commands.

Selection survives media-player restarts while the router process runs.
It is not saved across router restarts. Command routing and property
refresh at dispatch were subsequently added in Milestone 4; continuous
property tracking remains unimplemented.

## Milestone 4 completed — 2026-09-27

The user approved this milestone after merging Milestones 1–3 into main.
The working tree was clean at the start. Selection and discovery APIs
have been reviewed, and the current MPRIS Player specification has been
checked against the four required actions.

The mappings remain settled: Play/Pause to `PlayPause`, Stop to `Stop`,
Previous to `Previous`, and Next to `Next`. Volume and mute remain outside
scope. Missing selection or an unavailable selected application must not
cause a command to be delivered to another application.

### Approved decisions

- Refresh only relevant properties at dispatch time, without rediscovering
  all players. Check the known instance's service ownership and address
  its unique owner directly. Continuous property tracking is deferred.
- Add opt-in interactive terminal commands to the running prototype.
- Discard unavailable/unsupported commands with diagnostics. Do not replay
  unavailable inputs when an application returns or retry a sent transport
  method after an error/timeout. A missing reply does not prove that the
  player failed to execute the command.

### Implemented behavior and structure

- `src/player/routing.rs` defines the four normalized `TransportAction`
  variants and dispatches native MPRIS calls using the existing zbus/Tokio
  dependencies. No new dependencies were added.
- Dispatch uses a captured selection/instance. It verifies service
  ownership before and after the uncached property reads, then checks
  that the captured target is still selected before sending the method.
  It never retargets a pending input to a replacement instance.
- All actions require current `CanControl`. Next/Previous additionally
  require `CanGoNext`/`CanGoPrevious`. Play/Pause requires `CanPause` per
  the method contract, plus `CanPlay` when paused or stopped. Invalid
  playback status or failed property reads prevent dispatch. Stop has
  no additional capability property.
- Pre-dispatch work has a two-second overall timeout, and the transport
  method reply has a separate two-second timeout. Outcomes distinguish
  acknowledgement, skip, failure before sending, player-reported errors,
  and uncertain execution after a missing reply. No automatic retries.
- Checks and command execution cannot be atomic across processes. The
  player may change state after a check. A successful reply is reported
  as acknowledgement, not proof that playback changed.
- `--interactive` enables `play-pause`, `stop`, `previous`, and `next`,
  one per stdin line. With no flag, the prototype remains observation-only.
- The main event loop continues discovery and input handling while one
  dispatch task runs. Commands are serialized, with at most 32 waiting
  requests retaining their original targets. Excess requests are dropped
  with a diagnostic. Unavailable/unselected inputs are dropped immediately.
- A bounded channel and detached standard input thread avoid blocking
  Tokio shutdown on idle stdin. SIGINT is registered before announcing
  readiness. EOF drains pending commands; Ctrl+C exits immediately and
  warns if a command is in flight. No hardware keys are captured.

### Verification

- All 34 tests passed: 12 library unit tests, four CLI parser tests,
  eight discovery/selection integration tests, and ten routing integration
  tests in `tests/routing.rs`.
- Cargo build, Clippy with warnings denied, formatting, and whitespace
  checks passed. RustRover's build also succeeded; its inspections reported
  no errors or warnings in the changed routing, CLI, main, and module files
  or the new routing integration tests.
- Private-bus recording players verify all four methods, isolation of
  other players, changing capabilities without notifications, playback
  state checks, missing selection, invalid properties, owner changes,
  selection changes during preflight, timeouts, explicit method errors,
  and no automatic retries.
- Executable tests verify input order, dropping unavailable inputs,
  application return, discovery/input responsiveness during an in-flight
  check, EOF, Ctrl+C with idle stdin, and observation-only mode.
- Automated playback commands were sent only to private test players.
  A live desktop playback check was not performed.

Continuous property tracking, daemon API, hardware input, tray UI, and disk
persistence remain future work at the completion of Milestone 4.

## Milestone 5 completed — 2026-09-29

The user authorized this milestone on branch `feature/5-read-input`.
The working tree was clean at the start. Initial investigation confirmed
the installed Cinnamon schema's five transport keys and GIO 2.80.0
development metadata. Initial investigation did not change desktop settings.

### Approved decisions and implementation

- Keep capture opt-in with `--capture`; preserve observation-only startup.
  Feed normalized hardware actions into the existing routing queue.
- Use native `x11rb` for X11 and `gio` for Cinnamon GSettings access.
  Published documentation and compatible dependency versions were checked
  before addition. No runtime `gsettings` subprocess is used.
- Before any settings write, durably save a recovery journal in the user's
  XDG state directory. Preserve exact arrays and whether each setting was
  explicitly set or inherited. Use a process lock to prevent competing
  router instances from modifying the same settings/recovery record.
- Acquire only the five specified transport keys, normalize Play and Pause,
  and leave volume/mute untouched. If acquisition fails, release partial
  grabs and restore the settings this attempt changed.
- On clean exit (including SIGINT/SIGTERM), release grabs and restore prior
  settings only where the current value still matches the router's expected
  temporary value. Preserve intervening user changes; report conflicts.
  An identical user-written value cannot reliably be distinguished from
  the router's own value, and settings comparison/write is not atomic.
- Approved crash policy: recover an unfinished journal before the next
  capture session, with a separate `--restore-bindings` recovery command.
  This does not restore Cinnamon bindings immediately after SIGKILL or a
  crash. A separate watchdog would be needed for prompt process-crash
  recovery and is an alternative requiring a user decision.
- Approved custom-shortcut policy: remove only the five plain XF86
  transport accelerators from their corresponding arrays, leaving custom
  shortcuts under Cinnamon control. Those custom shortcuts would therefore
  retain Cinnamon's target-selection behavior, not the router's selection.
  Routing custom accelerators too would require expanded input handling.

The user approved all three recommendations: opt-in capture, recovery on
next capture startup or by a manual command, and preservation of custom
Cinnamon shortcuts. Implementation, isolated verification, and the
user-reported physical-key test are complete. The user committed all changes
and merged them into `main`.
Selection persistence remains Milestone 7; the journal stores only the
information needed to undo this backend's settings changes.

### Structure and implementation details

- `src/input/settings.rs` implements native GSettings access and a settings
  lease. A small settings trait supports deterministic failure tests.
  The journal is written via a synced temporary file, atomic rename, and
  directory sync before any settings changes. A held file lock excludes
  competing capture/recovery processes using the same state directory.
- Restoration is repeatable after partial failure. Errors retain the
  journal. Externally changed values are preserved, with the original
  journal archived as `bindings-conflict-*.json` for inspection.
- `src/input/x11.rs` uses native X11 passive grabs, discovers keycodes from
  the current map, accounts for lock modifiers, and uses XKB detectable
  repeat to send one action per press. Tokio file-descriptor readiness
  drives input. Keyboard notifications trigger a mapping comparison;
  a relevant change stops capture rather than using obsolete keycodes.
- `src/input/mod.rs` coordinates settings and X11 ownership. Startup allows
  two seconds for Cinnamon to release grabs, with rollback on failure.
  GLib notifications are serviced at least every 100 milliseconds while
  idle. External edits to the settings the router changed stop capture.
- Main integrates hardware and terminal input into the existing bounded
  routing queue, handles SIGINT/SIGTERM, and explicitly restores settings
  on exit. Drop provides fallback cleanup; it is not crash recovery.
- `--restore-bindings` works without an X connection. Capture requires
  the Cinnamon/X11 environment. Observation-only startup is unchanged.
- New direct dependencies resolved to `gio 0.22.10`, `x11rb 0.14.0`,
  `serde 1.0.229`, and `serde_json 1.0.151`. Tokio's `net` feature supplies
  readiness integration. X11 XTEST is enabled only for tests.
- Tests use fake settings, a native GIO memory backend, and private Xvfb,
  keyfile settings, and D-Bus instances. Automated verification did not
  change live desktop settings or send commands to live media players.

### Verification

- All 46 tests passed: 19 library unit tests, five CLI parser tests, eight
  discovery integration tests, four input integration tests, and ten routing
  integration tests. Cargo Clippy with warnings denied, formatting, and
  whitespace checks passed. RustRover's build succeeded and inspections
  reported no errors or warnings in the new/changed Rust files checked.
- Input integration tests drive actual XTEST key events through the binary
  to a private recording MPRIS player. They verify all five keysyms/four
  actions, held-key suppression with repeat enabled, lock modifiers,
  unclaimed volume and modified transport grabs, unavailable-input drops,
  SIGINT/SIGTERM restoration, SIGKILL recovery manually and on restart,
  competing-process exclusion, grab-failure rollback, external edits, and
  restoration after relevant keyboard mapping changes.
- Settings unit tests cover exact arrays, explicit versus inherited values,
  interrupted acquisition/restoration, malformed journals, and conflict
  archival. Native GIO behavior is also exercised with a memory backend.
- X11 testing showed that a new-keyboard notification need not mean the
  transport mapping changed. The backend compares mappings before deciding
  to stop. The test harness retains unmatched stdout/stderr messages because
  their delivery order is not guaranteed.
- The user confirmed that the physical-key test through the CLI was
  successful and that all changes were committed and merged into `main`.
  This completes the outstanding manual validation for this milestone.
- Immediate crash recovery, automatic startup, and selection storage remain
  outside this implementation. A crash may leave transport bindings released
  until the next capture startup or manual recovery, as explicitly approved.

## Milestone 6 completed — 2026-09-30

The user authorized the daemon D-Bus API on branch `feature/6-daemon-api`.
The working tree was clean at the start. Existing selection, routing, and
capture code was reviewed. The user approved all three recommendations
(service lifecycle/naming, revisioned snapshots and runtime selection,
and capture control with auto-selection deferred). The user also confirmed
the in-memory snapshot/revision approach after discussing memory usage.
No dependencies were added or upgraded.

### Approved and implemented contract

1. Service lifecycle and naming:
   - Add explicit `--serve` to publish the API on the user session bus.
     It can be combined with existing selection, capture, and interactive
     flags. Closing stdin does not stop a serving daemon.
   - Bus name and interface: `org.mediarouter.MediaRouter1`.
     Object path: `/org/mediarouter/MediaRouter1`.
   - Only one API server owns this name per session bus. A second server
     fails without replacing or queuing behind the first, and before any
     capture settings changes. Ordinary diagnostic runs remain possible.
     Capture ownership remains protected by the existing settings lock.
   - Reuse the installed zbus dependency. Version the public contract from
     its first release and document its introspection XML and data types.

2. State and selection:
   - `GetState()` returns one coherent snapshot: revision, logical
     applications, selected identity, availability, active instance, and
     capture state/error information.
   - Each application record contains identity kind, exact identity value,
     display name, and running instance count. Group instances according
     to the established selection rules. An absent selected application
     remains in the separate selection record, not the running inventory.
   - `SelectApplication(kind, value)` changes the selection at runtime,
     including selection of an absent application. `ClearSelection()`
     explicitly clears it. Desktop-entry and fallback Identity namespaces
     remain distinct; an empty fallback identity is not a no-selection
     marker. Reject unknown kinds and empty desktop-entry identifiers.
   - `StateChanged(snapshot)` carries a complete snapshot with a monotonic
     revision for this service instance. Clients subscribe before reading
     initial state, ignore older revisions, and resynchronize when the
     service owner changes. This replaces the earlier conceptual collection
     of independent change signals with one consistent update.
   - Do not expose discovery-time playback/capability snapshots as live
     properties. Routing continues to refresh them when dispatching.

3. Settings scope and capture control:
   - `SetCaptureEnabled(bool)` enables/disables hardware capture at runtime
     using the existing journal and restoration rules. Disabling preserves
     application selection. Capture remains opt-in at startup.
   - While serving, input-backend failure stops capture and reports a fault
     through API state, while discovery and API access remain available.
     Report restoration failures explicitly and preserve recovery data;
     do not report clean disablement when restoration failed.
   - Defer auto-selection settings and behavior to Milestone 7, where their
     precedence and browser rules need approval. Do not expose setters for
     unimplemented behavior. Selection and settings are not saved across
     router restarts in Milestone 6.
   - Keep transport-method invocation on the existing hardware/terminal
     paths for this milestone; the new API controls selection and capture.

### Implementation and verification

The main event loop remains the single owner of mutable router state.
D-Bus handlers send bounded requests to that loop and await replies;
wire-format records remain separate from core selection types. Publish
coherent snapshots after state changes. A successful selection reply means
that the change has been applied, not merely queued. Existing routing
checks still prevent pending commands from being redirected to replacements.

- `src/dbus_api/mod.rs` defines typed D-Bus records, service methods,
  errors, and snapshot publication using the existing zbus/Serde crates.
  `Selection::players()` provides a read-only inventory iterator.
- The main loop applies mutation requests from a bounded 32-entry queue;
  replies contain the applied snapshot. GetState reads the latest snapshot
  directly, including during capture startup. Invalid requests and repeated
  unchanged values do not advance the revision.
- A Tokio watch channel retains the current snapshot; the signal publisher
  can coalesce intermediate revisions. No history is stored. Each daemon
  starts its `u64` revision at zero; clients track the unique bus owner.
- Capture states are disabled, starting, enabled, and faulted. While serving,
  capture faults leave discovery and API access running. Restoration failures
  are returned explicitly, retain recovery data, and can be retried.
  Non-serving capture runs retain their existing exit-on-failure behavior.
- `docs/DBUS_API.md` and the versioned interface XML document wire signatures,
  identity/availability semantics, client synchronization, errors, recovery,
  and optional diagnostic busctl examples. README contains user instructions.
- All 50 tests passed: 19 library unit tests, six CLI parser tests, two API
  integration tests, eight discovery integration tests, five input integration
  tests, and ten routing integration tests. Private-bus API tests verify
  introspection, signals/revisions, identity validation, grouping, sticky
  selection, departure/return, multiple clients, exclusive name ownership,
  stdin EOF, SIGTERM, and revision/selection reset after restart.
- The isolated Xvfb/settings test verifies API-controlled enable/disable,
  routing after runtime selection, unavailable-input drops, unchanged
  selection on disable, restoration failure and retry, preserved external
  edits, and continued API access after input-backend failure.
- Cargo Clippy with warnings denied, formatting, and whitespace checks passed.
  RustRover's build succeeded and inspections of the changed/new Rust files
  reported no errors or warnings. No live desktop settings or playback were
  used for automated verification.
- The user confirmed that CLI service commands function as expected,
  completing the manual service-command check for this milestone.

D-Bus API Design Guidelines and the installed zbus 5.19.0 APIs informed the
contract. Selection/preference disk storage, auto-selection policy, startup
integration, and the tray UI remain future milestones. Milestone 7 has not
been authorized.

## Milestone 6.1 completed — CLI abbreviations — 2026-09-30

The user authorized this change on `feature/6.1-cli-switches` for a separate
commit and merge before Milestone 7. The working tree was clean at the start.

Approved aliases: `-i` for `--interactive`, `-c` for `--capture`, `-s` for
`--serve`, `-p` for `--select`, and `-I` for `--select-identity`. Existing
`-h` remains help; `--restore-bindings` stays long-only. Flags can group as
`-ics`. Value-taking short options consume the rest of their group or the
next argument, allowing `-p spotify`, `-pspotify`, and `-icsp spotify`.

The parser maps short forms to the same validation path as long options.
Duplicates, multiple selections, unknown options, and missing values remain
errors. Help and recovery remain standalone. Option-like separate arguments
starting with `-` are rejected as missing selection values; attach such
literal values to `-p` or `-I` instead. Exact fallback identities, including
Unicode, whitespace, and an explicitly empty value, remain supported.

No dependencies or application runtime behavior are changed. CLI unit tests
cover grouping, values, identity preservation, and rejection cases. Executable
tests exercise selection lifecycles using long and short forms and verify
help/argument errors before bus access. Help, README, and the API reference
are updated.

Verification passed: all 55 tests (including 11 CLI parser tests), Clippy
with warnings denied, formatting, and whitespace checks. RustRover's build
succeeded; inspections of `src/cli.rs` and `tests/discovery.rs` reported no
errors or warnings. No live desktop settings were changed. The user confirmed
that the changes were committed, pushed, and merged into `main` separately
from Milestone 7. Milestone 7 is not authorized.

## Milestone 7 started — 2026-10-01

The user authorized this milestone after updating main and creating
`feature/7-startup`. The working tree was clean. Current startup, capture,
selection, discovery, and D-Bus API paths were reviewed. The user approved
the original policies after deferring MRP on 2026-10-01. No live login
configuration changes have been requested.

### Approved policies

1. Durable configuration:
   - Store versioned JSON in `$XDG_CONFIG_HOME/media-router/config.json`,
     falling back to `~/.config/media-router/config.json`. Reuse Serde/JSON.
   - Persist the selected logical identity (including no selection), desired
     capture enablement, MRO auto-selection enablement, and an explicit list
     of application identities excluded from MRO auto-selection.
   - Do not persist snapshots, revisions, inventories, service names, or
     unique bus owners. Keep the existing recovery journal in XDG state.
   - Use a single-writer lock, synced temporary file, rename, and directory
     sync. Missing configuration means defaults: no selection, capture off,
     auto-selection off, and no exclusions. Malformed/unsupported files are
     reported and preserved, never silently overwritten with defaults.
   - Successful preference changes must be durably recorded. Save failures
     are explicit API errors. Capture is an external side effect, so its
     requested preference and actual/faulted state must remain distinct;
     never claim that disk writes and desktop changes are one transaction.

2. Persistent versus diagnostic execution:
   - `--serve` loads and saves preferences. Non-serving CLI runs retain their
     temporary behavior and do not read or write these preferences.
   - Explicit serving startup selection/capture flags override saved values
     and become the new saved preferences. Add `--no-capture` for an explicit
     startup override to off; reject combining it with `--capture`/`-c`.
   - API selection/clear and capture requests update saved intent. Capture
     faults, normal shutdown, and temporary player absence do not erase that
     intent. Auto-selection changes, when enabled, also save the new identity.
   - Extend version 1 with separate settings methods/signals; preserve existing
     GetState/StateChanged wire signatures and keep actual capture state there.

3. Auto-selection:
   - Default off. When enabled, a logical application's transition from no
     validated instances to its first validated instance after the startup
     inventory is eligible to become selected. Already-running startup
     applications do not replace the restored selection, even if validation
     completes later. Enabling auto-selection does not retroactively choose
     an existing application. Additional instances of an available application
     do not trigger it; reopening after all instances disappear does.
   - Among eligible arrivals, the latest validation/admission event wins.
     Manual selection takes effect immediately but can be superseded by a
     later eligible arrival while auto-selection remains enabled. Clearing
     selection does not disable auto-selection. Player disappearance alone
     still never selects another application.
   - Use exact per-application exclusions rather than inferring browser
     identity. Users can exclude Firefox or any other application using its
     typed identity; excluded applications remain manually selectable.
     Exclusions default to empty. This replaces the conceptual browser toggle
     with an explicit, general policy.
   - Browser tab changes within an existing MPRIS instance do not trigger
     auto-selection. MPRIS appearance is the event the router can observe;
     it does not prove a browser process was just launched.

4. Login startup and recovery:
   - Provide opt-in install/remove commands for a per-user XDG autostart
     entry, normally `~/.config/autostart/media-router.desktop`. Target an
     installed executable by absolute path, with `--serve`, and restrict
     automatic startup to Cinnamon. The capture backend still checks X11.
   - Implement and test installation using temporary directories first.
     Enabling startup in the user's actual session is a separate explicit
     action; no live installation has been requested yet.
   - Before a serving daemon applies saved capture intent, recover an
     outstanding journal, including when the saved preference is capture off.
     Respect the capture lock and never recover settings owned by an active
     capturing process. Recovery failure is exposed as a fault through the
     API and retains recovery data.
   - Preserve clean signal shutdown and manual recovery. Retain the approved
     next-start/manual crash recovery policy; no watchdog or immediate crash
     restart is proposed for this milestone.

Primary references checked: XDG Base Directory Specification
(`https://specifications.freedesktop.org/basedir/latest/`) and Desktop
Application Autostart Specification
(`https://specifications.freedesktop.org/autostart/latest/`). Installed
Cinnamon autostart entries use `OnlyShowIn=X-Cinnamon;`.

Implementation proceeds incrementally: configuration storage, daemon
persistence/settings and MRO selection, then startup/recovery integration.
Each step receives isolated verification and a user review checkpoint.

### Step 1 implemented — configuration storage — 2026-10-01

The configuration foundation was implemented and verified at this checkpoint.
The user subsequently committed Step 1 on the current branch, without
merging into main, and authorized Step 2. Daemon integration follows below.

- `src/config.rs` defines the version 1 JSON preferences: `version`,
  `selected` (typed logical identity or null), `capture_enabled`,
  `auto_select_new`, and `exclusions`. There are no MRP fields. Application
  identities serialize as objects with `kind` and exact `value`; an empty
  fallback Identity is valid, while an empty desktop-entry identifier is not.
- Configuration resolves under the absolute XDG configuration home, falling
  back to the absolute HOME plus `.config`. Relative/empty XDG values are
  ignored. Path handling supports non-Unicode filesystem paths.
- `Store` holds a separate `config.lock` file for its lifetime. Missing
  configuration yields defaults without creating `config.json`; malformed,
  unknown-field, and unsupported-version configuration is preserved and
  reported as an error. No migration is needed for the first format version.
- Saves validate first, write and sync a private temporary file, rename it,
  then sync the directory. Newly created directories and files use modes
  0700 and 0600 respectively. Incomplete temporary files are not loaded.
- Save errors distinguish failure before replacement from an uncertain
  durability result after replacement. In the latter case the store's
  in-memory preferences reflect the visible new file, and the caller must
  still report failure. A retry performs a full save even for equal values.
- Eight new unit tests cover defaults, exact identity round trips and clear
  selection, invalid-file preservation, writer exclusion, interrupted-save
  remnants, failures before replacement, uncertain final directory sync,
  permissions, and XDG path resolution. Tests use temporary directories.
- All 27 library tests passed, as did Clippy across all targets with warnings
  denied, formatting, and whitespace checks. RustRover's build succeeded;
  inspections of all three changed/new Rust files reported no problems.
  Integration tests were not rerun for this unconnected storage module.
- No dependencies were added. README and the public D-Bus contract are
  unchanged because no new executable behavior is exposed in this step.
  No live preferences, login entries, or Cinnamon settings were changed.

### Step 2 implemented — daemon persistence and MRO — 2026-10-01

The user authorized integration after committing Step 1 on the same branch.
Step 2 was implemented and verified at this checkpoint. The user then
committed it on the current branch without merging and authorized Step 3.

- Serving opens the configuration store and restores logical selection,
  desired capture state, and MRO preferences. Diagnostic runs remain
  temporary and ignore configuration. Explicit serving selection/capture
  flags override and save the corresponding preferences only after gaining
  the API bus name. Invalid configuration prevents serving startup.
- CLI capture is now `Option<bool>` so omission differs from an explicit
  override. `--no-capture` is long-only and conflicts with `--capture`/`-c`.
  Duplicate forms remain errors. Help explains persistent versus temporary
  operation. No dependencies were added.
- Selection/clear, capture intent, MRO enablement, exclusions, and automatic
  selections are saved before acknowledging success. Save failures before
  replacement do not apply the requested change. After replacement with an
  uncertain final sync, visible intent is reflected and PersistenceFailed
  is still returned; external capture work does not start on a save error.
- Desired capture remains distinct from actual capture. Failed acquisition,
  input faults, and clean shutdown do not disable the saved preference.
  Restarting serving retries enabled capture; disabling through the API or
  `--serve --no-capture` saves an explicit disabled preference.
- The additive D-Bus Settings record has signature `(tbba(ss)s)`: independent
  revision, desired capture, MRO enablement, typed exclusions, and the latest
  persistence error. GetSettings, SetAutoSelectNew, SetAutoSelectExclusions,
  and SettingsChanged are implemented. The exclusions setter replaces the
  list, validates identities, sorts, and deduplicates it. State signatures
  are unchanged. Both snapshot publishers can coalesce intermediate revisions;
  clients synchronize them independently. Save errors clear after a
  successful save and are surfaced for automatic changes as well as requests.
- `src/player/auto_selection.rs` contains MRO policy. Discovery carries a
  startup classification and monotonic observation time through validation
  retries. Existing startup players and candidates observed before MRO is
  enabled cannot steal selection when validation completes later. Extra
  instances of an available logical application do not qualify. Reopening
  after all instances disappear can qualify. Manual selection and clear do
  not disable MRO; disappearance does not trigger automatic fallback.
- The first eligible newly admitted application is selected unless excluded;
  later eligible admissions can supersede it. Exclusions use exact typed
  identities and do not prevent manual selection. Failed automatic saves are
  reported without queuing a later replay. No MRP behavior is implemented.
- README, CLI help, DBUS_API.md, and the interface XML document the executable
  behavior, settings contract, configuration format, and failure semantics.
- All 71 tests passed: 29 library units, 12 CLI units, seven API integrations,
  eight discovery integrations, five input integrations, and ten routing
  integrations. New coverage includes restart/CLI overrides, saved clear,
  desired capture retained through faults, settings signals and validation,
  MRO lifecycle/exclusions, delayed startup and pre-enable admission, failed
  manual/automatic saves, and malformed configuration versus diagnostics.
- The isolated X11 test verifies clean shutdown restores Cinnamon while
  retaining capture intent, and a subsequent serving run resumes capture and
  selection without CLI overrides. All serving test processes use temporary
  configuration/state directories and isolated GSettings backends.
- Clippy across all targets with warnings denied, formatting, and whitespace
  checks passed. RustRover's build succeeded; inspections of all eight changed
  or new Rust files reported no warnings or errors. No live desktop settings,
  preferences, or login entries were changed by automated verification.

### Step 3 implemented — login startup and recovery — 2026-10-01

The user authorized Step 3 after committing Step 2 without merging to main.
Milestone 7 implementation and automated verification are complete. The
user's actual login session has not been modified or used for a login test.
The user subsequently confirmed Step 3 is committed and deferred installation
testing until the system-tray interface is complete. This manual check remains
outstanding; it is not a prerequisite for implementing the tray interface.

- Add standalone long-only CLI commands `--install-autostart PATH` and
  `--remove-autostart`. Installation takes the explicitly chosen installed
  executable's absolute UTF-8 path and verifies it is an executable file.
  It never launches that executable. Invalid targets fail before creating
  startup files. Paths containing control characters or `=` are rejected.
- `src/autostart.rs` manages the personal XDG entry, normally
  `~/.config/autostart/media-router.desktop`. Exec invokes the chosen path
  with `--serve`; TryExec names the same file. OnlyShowIn is `X-Cinnamon;`,
  and Terminal is false. Capture still enforces Cinnamon/X11 at runtime.
- Exec arguments are quoted using Desktop Entry rules, including literal
  percent signs, with GLib KeyFile handling the additional string-escaping
  layer. No shell is invoked. Existing GLib/GIO is reused; no new dependencies.
- A held lock serializes autostart changes. Installation writes a synced
  temporary file, renames, and syncs the directory. Removal syncs the
  directory and reports post-removal sync failures explicitly. Files carry
  `X-MediaRouter-Managed=true`; unrecognized entries and symlink entries are
  preserved and reported. Reinstall updates a managed entry; removing an
  absent entry succeeds without creating an autostart directory. Removing
  an entry does not stop a running daemon or erase saved preferences.
- After acquiring the API bus name and recording any explicit CLI overrides,
  serving checks for an outstanding binding journal before applying capture
  intent. With no journal, recovery needs no Cinnamon schema or settings
  access. With a journal, it uses the existing settings lease and lock and
  can restore without X, including when capture is disabled.
- Recovery failure keeps the API/discovery available, publishes a capture
  fault, retains recovery data and desired capture intent, and skips capture
  startup. Retrying SetCaptureEnabled(false) runs recovery after the cause
  is addressed. Active capture locks prevent stealing another process's
  settings. Existing clean shutdown and manual recovery remain unchanged;
  no watchdog or immediate crash restart was added.
- README and CLI help document installation/removal and recovery. The D-Bus
  reference explains startup faults and retry semantics. The interface wire
  format is unchanged in this step.
- All 81 tests passed: 34 library units, 13 CLI units, seven API integrations,
  one autostart CLI integration, eight discovery integrations, eight input
  integrations, and ten routing integrations. New tests cover quoting,
  managed-entry replacement/removal, invalid targets, locks, failed writes,
  symlink preservation, XDG/HOME resolution, standalone CLI commands,
  disabled-capture recovery without X, malformed startup recovery with API
  retry, and respecting a live capturer's lock.
- A generated entry in a temporary directory passed desktop-file-validate.
  All startup files and settings used by tests were isolated. Installation
  in the user's actual session remains a separate explicit action.
- `src/filesystem.rs` holds shared private-directory creation, stale-file
  removal, and temporary-file cleanup helpers used by configuration,
  autostart, and journal storage. This removes duplicated file-handling
  fragments identified by RustRover without changing the save ordering.
- After that extraction, all 81 tests passed again. Clippy with warnings
  denied, formatting, and whitespace checks passed. RustRover's build
  succeeded and inspections of all ten changed/new Rust files were clean.

References checked: XDG Autostart Specification and Desktop Entry Exec/value
escaping rules at `https://specifications.freedesktop.org/autostart/latest/`,
`https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html`,
and `https://specifications.freedesktop.org/desktop-entry/latest/value-types.html`.

### Future-version proposal: most-recently-played selection (MRP)

Deferred by the user on 2026-10-01 because standard MPRIS cannot reliably
separate autoplay from an explicit user Play action. Do not implement an
MRP toggle, settings fields, playback-driven selection, or an exception to
sticky instance selection in Milestone 7. Revisit only with user approval
and a satisfactory way to address playback provenance (potentially a later
application/browser integration layer).

The original request was for independent MRO and MRP toggles, usable alone
or together. Example: opening a browser selects it through MRO; opening
Spotify selects Spotify; navigating to YouTube leaves Spotify selected;
starting Spotify playback keeps Spotify selected; starting YouTube while
Spotify is still playing selects the browser through MRP; pausing Spotify
does not change selection. Only Play should trigger MRP, not Pause or other
media-control commands.

The following feasibility details and suggested semantics are retained as
an **unapproved future proposal**, not requirements for the current version:

- MRO and MRP are independent toggles, both defaulting to off.
- MRO: a logical application's transition from no
  validated instances to its first validated instance after the startup
  inventory is eligible to become selected. Already-running startup
  applications do not replace the restored selection, even if validation
  completes later. Enabling either mode does not retroactively choose
  an existing application. Additional instances of an available application
  do not trigger MRO; reopening after all instances disappear does.
- MRP: observe each validated instance's PlaybackStatus transitions from
  Paused or Stopped to Playing. Duplicate Playing notifications, transitions
  to Paused/Stopped, and metadata-only changes do not qualify. This detects
  playback starting/resuming, not the literal Play command or its cause;
  autoplay can also qualify. A Play action while already Playing is not
  detectable through a status transition. Do not infer playback from
  metadata, seek, volume, or command acknowledgements.
- Baseline startup players' current status without selecting them. Recommend
  counting a newly appearing instance already Playing after startup as an
  MRP event, so playback begun before admission is not missed. This is an
  observation-time approximation, not proof of the actual playback order.
  Instances present when MRP is enabled are baselined without selection.
- When both modes are enabled, the latest eligible event processed by the
  router wins; neither mode has permanent priority. MRP does not require
  other applications to stop playing. Pause, Stop, and disappearance never
  cause automatic fallback to another playing application.
  Manual selection takes effect immediately but can be superseded by a
  later eligible event while the relevant mode remains enabled. Clearing
  selection does not disable auto-selection. Player disappearance alone
  still never selects another application.
- Recommend exact per-application, per-mode exclusions rather than inferring browser
  identity. Users can exclude Firefox or any other application using its
  typed identity; excluded applications remain manually selectable.
  Separate lists allow excluding a browser from MRO while allowing MRP.
  Exclusions default to empty. This replaces the conceptual browser toggle
  with an explicit, general policy and requires user approval.
- Browser tab changes within an existing MPRIS instance do not trigger
  MRO. They can trigger MRP if exposed as a qualifying playback transition.
  MPRIS appearance does not prove a process was just launched: a browser
  first exposing a player when a page loads can trigger MRO then. A new tab
  starting while the shared player remains Playing cannot be distinguished
  using PlaybackStatus alone. Per-tab routing remains outside scope.
- Recommend a narrow exception to sticky instance selection: an eligible
  MRP event selects the instance that started playing, including a second
  instance of the already-selected logical application. Persist only the
  logical identity; instance targeting remains in memory. Existing sticky
  instance behavior otherwise remains unchanged. This requires approval.
- Implementation requires live PropertiesChanged subscriptions, validated
  against the current owner, with duplicate suppression and safe initial
  baselining. Track current playback status per instance in RAM, without a
  playback history or persisted playback timestamps. Tests should cover the
  user's combined-mode sequence, startup, exclusions, repeated status,
  multiple instances, and stale-owner events.

The MRP feasibility review checked the MPRIS Player specification
(`https://specifications.freedesktop.org/mpris/latest/Player_Interface.html`):
PlaybackStatus changes emit PropertiesChanged; the interface provides no
general notification identifying an explicit user Play command. Autoplay
can cause the same transition, so the proposed status-based approximation
was not accepted for the current version.

## Milestone 8 — Rust tray compatibility probe — 2026-10-01

The user chose Rust and authorized a compatibility test before full tray
implementation. This is a standalone experiment, not the production tray
client, and it does not connect to the Media Router daemon API.

- Verified `ksni 0.3.6` from its published documentation and downloaded source.
  It supports Rust 1.80+, Tokio, and zbus 5, fitting the existing toolchain
  and dependencies. Added it as a development-only dependency for the Cargo
  example. The lockfile adds only ksni and its pastey 0.2.3 dependency; existing
  locked versions are unchanged. ksni's declared license is Unlicense.
- `examples/tray_probe.rs` exports a StatusNotifierItem and D-Bus menu using
  sample data. It exercises radio selection, checkboxes, a submenu, dynamic
  player-list changes, Unicode labels, tooltips, and normal/unavailable icons.
  Controls affect RAM only; no media commands, capture, preferences, or
  autostart entries are changed.
- The temporary icon is a blue play symbol with a composed red exclamation
  badge in its upper-right corner when unavailable. Pixmaps are supplied at
  24, 32, and 48 pixels. The badge is part of the main icon rather than relying
  on a desktop's separate overlay support. Final artwork remains undecided.
- The example exits on its Quit action, SIGINT/SIGTERM, host loss, or a bounded
  lifetime (300 seconds by default; `--seconds N` accepts 1–3600).
- The actual Cinnamon/X11 session exposes org.kde.StatusNotifierWatcher with
  an active host. A live probe was launched with a 600-second lifetime, and
  Cinnamon registered it. Read-only introspection confirmed the expected
  StatusNotifierItem interface and com.canonical.dbusmenu version 3.
- Cinnamon requested the menu and delivered callbacks for unavailable-state,
  MRO, exclusion, and dynamic-inventory controls. The Quit action was received
  and the probe shut down cleanly with exit status 0. Visual rendering and full
  mouse-interaction confirmation were requested from the user. The user
  subsequently confirmed that the display appeared correct and requested
  another run with a comprehensive visual checklist and left-click menu opening.
  `MENU_ON_ACTIVATE = true` was already set; it remains enabled, with explicit
  left-click verification included in the README checklist. Detailed results
  from the second run are recorded below. No shell restart or panel setting was
  changed for the test.
- Launched the second probe with a 900-second lifetime for the checklist;
  it reported ready. Cargo and RustRover builds passed before launch.
- All 81 existing tests passed after adding the development dependency.
  The example builds, and the full-target Clippy check passed with warnings
  denied. RustRover's final build and example inspection reported no problems.
  README contains contributor instructions to reproduce the probe.

References: `https://docs.rs/ksni/0.3.6/ksni/` and the crate's downloaded
README/Cargo manifest. Compatibility with other desktops, panel layouts,
scaling configurations, host restarts, and the production daemon is not
established by this probe.

### Second-run results and interaction investigation — 2026-10-02

The user confirmed every checklist item except left-click opening. Right-click,
icon/tooltip rendering, selection indicators, badge changes, dynamic inventory,
Unicode, submenu checkboxes, dismissal, repeated changes, and Quit all passed.
The menu closes after each radio/checkbox activation; the user initially
requested that it remain open between changes. This requirement was later
relaxed in favour of portability, as recorded below.

Installed versions: Cinnamon 6.6.9+zena and XApp 3.2.3+zena. Reviewing the
upstream XApp 3.2.3 source explains the left-click failure: `update_menu`
assigns a primary menu only for its AppIndicator path; ordinary SNI primary
clicks call `Activate` without a menu fallback. ksni 0.3.6 with
`MENU_ON_ACTIVATE = true` returns `UnknownMethod("ItemIsMenu")`, relying on
the host to open the menu. That fallback does not work here. The flag alone
must not be described as a verified left-click solution for Cinnamon.

XApp renders the exported menu with libdbusmenu GTK. ksni exports menu data
and callbacks, not the widgets, and exposes no keep-open option for activated
items. The initial recommendation was a second, isolated Rust prototype
with an application-owned popup for the controls, using SNI activation to
open it. This would let the application control dismissal and selection
behaviour. Toolkit/dependency choice and popup focus, placement, outside-click,
and Escape handling must be validated before production integration. This
prototype was not implemented or approved and is superseded by the portability
decision below. No desktop settings or
system libraries were modified to work around the host behaviour.

Source: <https://github.com/linuxmint/xapp/blob/3.2.3/xapp-sn-watcher/sn-item.c>
(`update_menu`, `xapp_icon_button_press`), and the downloaded ksni 0.3.6
`src/dbus_interface.rs` (`activate`) and `src/menu.rs`.

### Portability priority decision — 2026-10-02

The user explicitly prioritised multiple-distribution/desktop portability over
left-click menu opening and persistence after selection. Retain the standard
host-rendered Rust/ksni tray approach. The proposed application-owned popup
and subsequent control-window alternative are not the selected direction.
Accept host-native opening and dismissal behaviour rather than introducing
desktop-specific workarounds. Keep the CLI available independently of tray
support and report an unavailable tray host clearly in the future client.

Under these revised criteria, the Cinnamon/X11 visual compatibility probe
passes based on the user's reported results. This does not establish support
on other desktops, distributions, or Wayland, nor change the current
Cinnamon/X11 limitation of the media-key capture backend. Further validation
should cover different tray hosts and display protocols, including graceful
handling of absent tray support. Full Milestone 8 implementation remains the
next approval checkpoint; no additional GUI toolkit is needed for this choice.

## License decision — 2026-09-26

The user selected Apache License 2.0 for the project. The full, unmodified
license text from `https://www.apache.org/licenses/LICENSE-2.0.txt` is
included in `LICENSE`. `Cargo.toml` declares the SPDX identifier
`Apache-2.0`, and the README links to the license.

A personalized copyright notice is pending the user's preferred
copyright-holder name; do not infer ownership from the local username.
The example placeholders in the standard license appendix are part of
the original license text and should remain unchanged.

## Development workflow

- Proceed incrementally, with user approval before each new milestone.
  Milestones 1 through 6 are complete, including the user-confirmed
  physical-key and CLI service-command checks. Milestone 6.1 is complete
  and merged into `main`. Milestone 7 is authorized with its original
  persistence, MRO auto-selection, and startup policies approved. MRP is
  deferred to a future version. Milestone 7 implementation and automated
  verification are complete and all three steps are committed. The user has
  deferred installation testing until the system-tray interface is complete;
  actual login startup has not been tested. The Rust tray compatibility test
  is authorized; full Milestone 8 implementation requires further approval.
- Explain each step as a Rust development tutorial, including the purpose
  of code, tools, and verification commands. Keep tutorial explanations
  in the chat exchange, not in `README.md`.
- Ask the user at decision points rather than silently choosing product
  behavior or architectural policy.
- Follow the standard Rust testing layout: colocated `#[cfg(test)]` unit
  test modules and external integration tests under `tests/`. The user
  reviewed and approved the current structure; keep it as-is.
- Update this tracked document after each milestone and periodically
  after other major changes.
- Keep development progress reports in this document. `README.md` should
  contain only information relevant to end users and developers who wish
  to modify the application following launch.
- Do not make Git commits or push upstream. The user handles all Git
  updates; leave file changes for the user to review and commit.

------------------------------------------------------------------------

# Guidance for AI Coding Assistants

Read this document completely before making architectural changes.

When working on this repository:

1.  Treat verified experimental findings as evidence, not speculation.
2.  Treat settled product decisions as requirements unless the user
    explicitly changes them.
3.  Preserve the separation between persistent application identity and
    transient MPRIS instance identity.
4.  Keep the core routing logic independent of Cinnamon/X11 wherever
    practical.
5.  Do not introduce raw evdev/root requirements for the Cinnamon/X11
    implementation without new evidence that they are necessary.
6.  Do not intercept volume or mute in v1.
7.  Do not add `playerctl` as a runtime dependency.
8.  Do not assume every `org.mpris.MediaPlayer2.*` name is a usable
    player; validate it.
9.  Do not assume browser tabs are separately selectable through MPRIS.
10. Do not silently replace a closed persistently selected application
    with another application.
11. Prefer small, testable milestones and explain any architectural
    deviation before implementing it.
