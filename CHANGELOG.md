# Release notes

## Unreleased

## 0.2.1

- Autostart registration now installs both daemon and tray login entries using
  executables from the same directory. Removing autostart removes both entries;
  existing daemon-only entries can be upgraded by rerunning installation.
- The tray's new `--wait-for-host` mode tolerates a desktop tray host that starts
  late or restarts. Autostart uses this mode so neither program needs a terminal
  after login. Manual launches retain their existing missing-host behavior.
- Added structured documentation comments throughout the daemon, tray client,
  and runnable compatibility probes, including argument, return-value, error,
  and relevant panic descriptions.

### Upgrading from 0.1.0

Stop the daemon and quit the tray before replacing both executables. Saved
preferences are retained. If login startup was previously enabled, rerun
`--install-autostart` with the absolute installed daemon path to upgrade the
daemon-only entry and add the tray entry. The tray executable must be beside
the daemon. Capture and login startup remain opt-in.

### Supported environment and limitations

The binary archive targets Linux Mint 22.3 x86-64 and is dynamically linked.
Capture support remains limited to Cinnamon/X11; other desktops, distributions,
and Wayland have not been verified. A StatusNotifier-compatible tray host is
required. Listener priority remains best effort, and some GTK tray menus can
suppress media-key notifications while open. MRP and individual browser-tab
selection remain deferred.

## 0.1.0 — initial release

Media Router lets you choose which media application receives Play/Pause,
Stop, Previous, and Next on Linux Mint Cinnamon/X11. Volume and mute remain
under desktop control.

### Features

- Validated MPRIS player discovery and application selection, including
  applications that do not supply a DesktopEntry.
- Persistent selection across player closure and return, with an unavailable
  warning instead of automatic fallback to another application.
- Opt-in transport-key capture through Cinnamon's D-Bus service, including
  control while the screen is locked when lock-screen shortcuts are allowed.
- Optional auto-selection of most recently opened applications (MRO), with exclusions.
- Automatic listener reclaim on selection, player arrival, and screen activation;
  manual reclaim is available in the tray and daemon API.
- An independent StatusNotifier tray client with capture controls, fault
  diagnostics, and reconnection after daemon restart.
- Saved preferences, a session-bus API, CLI controls, opt-in daemon autostart,
  and an explicit legacy X11 backend with binding recovery.

### Verified environment

Linux Mint 22.3, Cinnamon 6.6.9, XApp 3.2.3, X11, x86-64. Live checks passed
routing, MRO/exclusions, unavailable selection and return, lock-screen control,
contender takeover/reclaim, volume/mute, held keys, capture lifecycle, daemon
restart/tray reconnection, login startup, and autostart removal.

### Known limitations

- Capture support is currently limited to Cinnamon/X11. Other desktops and
  Wayland have not been verified. Tray support depends on the host.
- Some GTK tray menus prevent Cinnamon from emitting media-key notifications
  while open. Closing the menu restores control; reclaim cannot fix the
  missing notifications.
- Listener priority is best effort: another application can register later
  and take priority without notifying Media Router.
- Browser media players may appear only after playback starts. Selection is
  by application, not individual browser tab.
- The default backend requires normal plain transport shortcuts; incompatible
  custom/disabled shortcuts are preserved and reported as capture faults.
- Autostart starts the daemon only; launch the tray separately.

Most-recently-played selection (MRP) and individual browser-tab selection are
deferred features. This release contains no implementation of either.

### Distribution

The binary archive contains both executables, documentation, the API contract,
and the Apache-2.0 license. It is dynamically linked and built on Linux Mint
22.3; compatibility with other distributions is not established. Source builds
use the committed lockfile and pinned Rust toolchain. See the README for
installation and usage.
