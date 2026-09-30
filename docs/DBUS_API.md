# Media Router D-Bus API, version 1

Start the service with `media-router --serve`. It uses the user session bus:

- Bus name and interface: `org.mediarouter.MediaRouter1`
- Object path: `/org/mediarouter/MediaRouter1`
- Machine-readable contract: [interface XML](org.mediarouter.MediaRouter1.xml)

`--serve` can be combined with `--select`, `--select-identity`, `--capture`,
and `--interactive`. Serving continues after terminal input ends. Only one
server can own this bus name. A second server fails before changing capture
settings and does not replace or queue behind the current server. Diagnostic
runs without `--serve` remain available. Capture additionally uses its
existing per-state-directory lock.

## Methods and signals

| Member | Input | Output / payload |
|---|---|---|
| `GetState` | None | `State` |
| `SelectApplication` | `kind: s`, `value: s` | Applied `State` |
| `ClearSelection` | None | Applied `State` |
| `SetCaptureEnabled` | `enabled: b` | Applied `State` |
| `StateChanged` (signal) | — | Complete `State` |

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
- `enabled`: hardware keys have been acquired; no error.
- `faulted`: capture is not active; error explains the failure. Restoration
  may still require recovery. Do not interpret this as clean disablement.

Clients should treat unknown future capture statuses as unavailable for
capture and display an appropriate diagnostic.

## Mutation and recovery semantics

Successful mutation replies contain the state after application of the
request, not merely its acceptance into a queue. Repeating an unchanged
selection, clearing an already empty selection, or setting capture to its
current healthy state does not increment the revision.

Capture remains opt-in at startup. Enabling uses the same settings journal
as CLI capture and may take about two seconds while Cinnamon releases grabs.
Disabling releases grabs and restores prior settings, preserving external
edits. It retains the application selection.

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

Standard D-Bus errors also apply for malformed calls, absent service, or
unknown members. Error text is diagnostic, not a stable machine-readable
identifier. There are no transport invocation methods or auto-selection
setters in version 1. Auto-selection and disk-backed preferences are not
implemented.

## Client synchronization and memory

Subscribe to `StateChanged` before reading `GetState`. Track the service's
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

The daemon keeps the latest snapshot and a `u64` revision in RAM. A signal
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
