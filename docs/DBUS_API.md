# Media Router D-Bus API, version 1

Start the service with `media-router --serve` (or `media-router -s`). It uses the user session bus:

- Bus name and interface: `org.mediarouter.MediaRouter1`
- Object path: `/org/mediarouter/MediaRouter1`
- Machine-readable contract: [interface XML](org.mediarouter.MediaRouter1.xml)

`--serve` can be combined with `--select`, `--select-identity`, `--capture`/`--no-capture`,
and `--interactive`. Serving continues after terminal input ends. Only one
server can own this bus name. A second server fails before changing capture
settings and does not replace or queue behind the current server. Diagnostic
runs without `--serve` remain available. Capture additionally uses its
existing per-state-directory lock. Serving also holds a configuration writer
lock, preventing two daemons on different buses from overwriting the same file.

## Methods and signals

| Member | Input | Output / payload |
|---|---|---|
| `GetState` | None | `State` |
| `SelectApplication` | `kind: s`, `value: s` | Applied `State` |
| `ClearSelection` | None | Applied `State` |
| `SetCaptureEnabled` | `enabled: b` | Applied `State` |
| `ReclaimMediaKeys` | None | Applied `State` |
| `StateChanged` (signal) | — | Complete `State` |
| `GetSettings` | None | `Settings` |
| `SetAutoSelectNew` | `enabled: b` | Applied `Settings` |
| `SetAutoSelectExclusions` | `exclusions: a(ss)` | Applied `Settings` |
| `SettingsChanged` (signal) | — | Complete `Settings` |

All records below have fixed field order. D-Bus `s`, `b`, `u`, and `t`
mean string, boolean, unsigned 32-bit integer, and unsigned 64-bit integer.
These are native D-Bus records, not JSON strings.

`State` has signature `(ta(sssu)(ssbss)(ss))`:

| Field | Type | Meaning |
|---|---|---|
| revision | `t` | Increasing revision within this service instance |
| applications | `a(sssu)` | Running logical applications, sorted by kind/value |
| selected | `(ssbss)` | Selection and its resolved instance |
| capture | `(ss)` | Capture status and diagnostic error |

Each application `(sssu)` contains `kind`, `value`, `name`, `instances`.
Kinds are `desktop-entry` and `identity`; the value preserves the exact
identifier. Matching is case-sensitive, without trimming. Instances sharing
an identifier are grouped. The label comes from the MPRIS Identity of the
instance with the lexicographically first service name. An empty fallback
Identity is valid. Invalid/uncontrollable players rejected by discovery
are not included.

Selection `(ssbss)` contains `kind`, `value`, `available`, `service`, `owner`.
With no selection, kind is `none`, all other strings are empty, and
availability is false. An absent selected application retains kind/value,
with availability false and empty service/owner. An available selection
includes its MPRIS service name and unique D-Bus owner. The selected
application need not appear in the running inventory.

`SelectApplication` accepts `desktop-entry` with a nonempty value, or
`identity` with any string, including empty. Selecting an absent application
is supported. Selecting the current application again preserves its active
instance. `ClearSelection` is the explicit way to clear user intent.
Selection never switches to a different application on player exit.

Availability means an instance admitted by discovery is present. Playback
status and capabilities are not exposed as live API properties; transport
routing checks relevant capabilities when dispatching each command.

Capture `(ss)` contains `status` and `error`:

- `disabled`: capture is off; no error.
- `starting`: acquisition is in progress; no error.
- `enabled`: input registration/grabs are active; no error. Cinnamon D-Bus
  registration is best effort and does not prove current listener priority.
- `faulted`: capture is not active; error explains the failure. Restoration
  may still require recovery. Do not interpret this as clean disablement.

Clients should treat unknown future capture statuses as unavailable for
capture and display an appropriate diagnostic.

## Mutation and recovery semantics

Successful mutation replies contain the state after application of the
request, not merely its acceptance into a queue. Repeating an unchanged
selection, clearing an already empty selection, or setting capture to its
current healthy state does not increment the revision.

Capture defaults to off and follows saved intent on serving startup.
Explicit `--capture`/`--no-capture` override and save that intent.
The default `cinnamon-dbus` backend registers with Cinnamon without changing
keybindings. It supports locked-screen delivery when Cinnamon allows keyboard
shortcuts there. Normal plain transport bindings are required; custom/disabled
transport settings are preserved and reported as a capture fault. Use the
process option `--input-backend x11` for legacy capture, which uses the existing
settings journal and passive grabs. This backend option is not persisted and
is not changed by tray actions; login startup uses the default backend.
Disabling releases registration/grabs and retains selection.

`ReclaimMediaKeys()` refreshes active Cinnamon listener registration. It does
not enable capture, change selection/settings, or save preferences. With capture
inactive it returns CaptureFailed. With legacy X11 capture it is a no-op because
the daemon retains its passive grabs. Success means the registration call was
acknowledged, not that exclusive priority is guaranteed. Another registering
listener can supersede it without a notification or priority query.

While capture is active, successful selection requests (including clearing or
reselecting), admitted player arrivals, and screen activation automatically
refresh priority. There is no periodic refresh. Screen activation includes an
unlocked screensaver. Screensaver owner changes alone do not fault capture;
its signal subscription follows exit/restart. Media-key service loss/replacement
faults capture; retry SetCaptureEnabled(true) after service availability returns.
An automatic/manual reclaim failure stops input and publishes a fault while
retaining saved intent. A successful saved selection can therefore be applied
even when its subsequent reclaim returns CaptureFailed; inspect state on errors.
Selection persistence errors prevent that request's reclaim attempt.

Before applying saved capture intent at serving startup, the daemon recovers
any outstanding binding journal, even when capture is disabled. No journal
means no Cinnamon settings access is needed for that recovery check. Recovery
requires the capture lock and does not need X. A recovery failure publishes
`faulted`, retains the journal and desired preference, and prevents capture
startup. The API remains available; `SetCaptureEnabled(false)` retries
recovery after the cause is addressed. A live capturer's lock is never stolen.

While serving, input-backend failure stops capture and publishes a fault;
discovery and the API remain available. A startup `--capture` failure also
leaves a serving daemon available with a fault. Disabling a faulted backend
retries recovery; successful recovery clears the fault. Enabling retries
acquisition and recovery. Failed restoration retains recovery data and
returns an error. Session-bus loss or state-signal publication failure ends
the daemon and attempts capture cleanup.

The request queue holds at most 32 waiting mutations. Requests are applied
serially by the main event loop. `GetState` remains readable during capture
startup. A timeout or disconnected client does not prove that an accepted
mutation was not applied: reconnect and inspect current state. Mutations
are not automatically retried by the service.

Error names under `org.mediarouter.MediaRouter1.Error`:

| Error | Meaning |
|---|---|
| `InvalidArgument` | Unknown identity kind or empty desktop-entry value |
| `Busy` | Mutation request queue is full |
| `Unavailable` | Router is stopping or stopped before replying |
| `CaptureFailed` | Capture acquisition or restoration failed; read current state |
| `PersistenceFailed` | Saving preferences failed; read state and settings before retrying |

Standard D-Bus errors also apply for malformed calls, absent service, or
unknown members. Error text is diagnostic, not a stable machine-readable
identifier. There are no transport invocation methods in this API.

## Settings and persistence

The existing `State` wire signature is unchanged. The additive `Settings`
record has signature `(tbba(ss)s)` and these ordered fields:

| Field | Type | Meaning |
|---|---|---|
| revision | `t` | Independent settings revision within this service instance |
| capture_enabled | `b` | Saved capture intent, even when actual capture is faulted |
| auto_select_new | `b` | MRO auto-selection enabled |
| exclusions | `a(ss)` | Exact `(kind, value)` identities excluded from MRO |
| persistence_error | `s` | Last save failure, or empty after a successful save |

`SetAutoSelectExclusions` replaces the complete list. Identity validation is
the same as for selection; empty fallback identities are valid. The setter
sorts by kind/value and removes duplicates. Excluded players remain eligible
for manual selection. Repeating unchanged settings does not advance their
revision unless it clears an earlier save error. Both setters return the
applied `Settings` snapshot after saving.

MRO selects a logical application's first validated instance when it appears
after the startup inventory. Startup candidates retain that classification
through delayed validation. Candidates observed before MRO was enabled do
not become eligible merely because validation finishes afterwards. Extra
instances of an available application do not trigger selection; reopening
when all instances have disappeared can. Among eligible admissions, the
latest processed one wins. Manual selection and clearing do not disable MRO.
Departure never selects a different application. Playback changes and tab
changes within an existing MPRIS player are not selection events.

Serving reads and writes versioned JSON in
`$XDG_CONFIG_HOME/media-router/config.json`, falling back to
`~/.config/media-router/config.json`. Diagnostic runs without `--serve` do
not use this file. Explicit serving selection/capture arguments replace
and save the corresponding preferences. Absent arguments preserve them.
Missing configuration means defaults; invalid/unsupported files are
preserved and cause serving startup to fail before capture changes.

Example configuration:

```json
{
  "version": 1,
  "selected": {"kind": "desktop-entry", "value": "spotify"},
  "capture_enabled": false,
  "auto_select_new": false,
  "exclusions": []
}
```

`selected: null` represents no selection. Only logical identities and
preferences are persisted, never active service names, owners, revisions,
or inventories. Stop the daemon before manually editing configuration.

Selection/clear, capture intent, MRO setters, and automatic selections are
saved before success. Capture then applies the external desktop operation;
its failure returns `CaptureFailed` while retaining the requested preference.
Input faults and clean shutdown do not overwrite capture intent. Disabling
capture preserves selection, even if restoring the desktop fails.

A save failure before file replacement leaves the requested preference and
capture/selection change unapplied; discovery continues independently.
If replacement succeeds but directory sync fails, the new intent is visible
and selection/settings reflect it, but the method still returns
`PersistenceFailed` because crash durability is uncertain. Capture work is
not started on a save error. Read both snapshots before retrying; resending
the request performs a full save even for unchanged values. Automatic-save
failures have no method caller, so they are logged and exposed in
`persistence_error`. A later successful save clears that diagnostic; failed
automatic selections are not queued for later replay.

## Client synchronization and memory

Subscribe to `StateChanged` before reading `GetState`, and to
`SettingsChanged` before reading `GetSettings`. Track each record's revision
independently; the two records are not one atomic combined snapshot. Track the service's
unique owner as well as the latest revision. For the same owner, replace
local state only with a higher revision; ignore older or equal snapshots.
Successful mutation replies are snapshots too. All fields within a snapshot
are consistent with the same router state.

On service-owner loss, mark the daemon unavailable. On owner replacement,
discard the old revision, subscribe for the new owner, and fetch fresh state.
Pin initialization calls and signals to that unique owner (or retry if the
owner changes during initialization). Revisions start at zero and are not
comparable across daemon instances. Startup capture/discovery can advance
the revision before the first client reads it.

The publisher may coalesce intermediate revisions if the bus is slow. These
are complete state updates, not an event log; clients must tolerate revision
gaps. A current snapshot is enough to reconstruct the UI.

The daemon keeps the latest state and settings snapshots and their `u64`
revisions in RAM. A signal
in flight and concurrent calls can hold temporary copies; there is no
snapshot history on disk or in memory. Memory scales with the current
application inventory and bounded pending work. The capture recovery
journal is separate and contains only settings-restoration information.

## Diagnostic examples

These commands use `busctl` as an optional development/client utility; it
is not an application runtime dependency.

```sh
busctl --user call org.mediarouter.MediaRouter1 /org/mediarouter/MediaRouter1 org.mediarouter.MediaRouter1 GetState
busctl --user call org.mediarouter.MediaRouter1 /org/mediarouter/MediaRouter1 org.mediarouter.MediaRouter1 SelectApplication ss desktop-entry spotify
busctl --user call org.mediarouter.MediaRouter1 /org/mediarouter/MediaRouter1 org.mediarouter.MediaRouter1 SetCaptureEnabled b true
busctl --user call org.mediarouter.MediaRouter1 /org/mediarouter/MediaRouter1 org.mediarouter.MediaRouter1 SetCaptureEnabled b false
busctl --user call org.mediarouter.MediaRouter1 /org/mediarouter/MediaRouter1 org.mediarouter.MediaRouter1 ClearSelection
busctl --user monitor org.mediarouter.MediaRouter1
```
