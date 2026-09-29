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

A separate tray UI may initially be implemented independently
(potentially Python or Rust), communicating with the daemon through a
stable D-Bus API. Keeping the UI separate from the routing daemon is
intentional.

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

Planned user preference:

-   optionally auto-select a newly opened media application.

A related desired setting was originally described as excluding new
browser tabs from auto-selection. Because standard Firefox MPRIS does
not expose individual tabs as separate players, browser-tab-specific
behavior cannot be implemented by the basic MPRIS layer.

Auto-selection semantics should therefore initially operate on logical
discovered applications/MPRIS players.

Important settled rule:

**The disappearance of the currently selected application does not
itself trigger selection of another application.**

Detailed auto-selection precedence/rules remain to be specified.

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

# Proposed Daemon D-Bus API

The daemon should expose its own stable D-Bus API so the tray
application is a thin client.

Conceptual operations:

``` text
ListApplications()
GetSelectedApplication()
SelectApplication(id)

GetAutoSelect()
SetAutoSelect(bool)

GetAutoSelectBrowsers()
SetAutoSelectBrowsers(bool)
```

Potential signals:

``` text
ApplicationsChanged
SelectedApplicationChanged
SelectedApplicationAvailabilityChanged
SettingsChanged
```

Exact names/signatures are not yet frozen.

The architecture should be:

``` text
Tray UI
   ↕
router's D-Bus API
   ↕
Rust daemon
   ├── player manager
   ├── selection manager
   ├── MPRIS
   └── input backend
```

This allows the tray UI implementation language to change without
rewriting the daemon.

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
Obtain user approval before Milestone 6 (the daemon D-Bus API). Capture is
opt-in, and recovery is available on capture startup or through
`--restore-bindings`. Routing refreshes relevant capabilities at dispatch
time. Selection persistence across router restarts remains Milestone 7.

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
  Milestones 1 through 5 are complete, including the user-confirmed
  physical-key test. Milestone 6 has not been authorized.
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
