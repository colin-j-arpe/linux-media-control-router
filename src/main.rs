mod cli;

use std::{collections::VecDeque, io::BufRead, process::ExitCode};

use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
};

use media_router::{
    config::{self, Preferences, Store},
    dbus_api::{self, CaptureState, Operation, Response, Server, Settings, State},
    input::{Backend, Capture, Event as InputEvent},
};

use media_router::player::{
    Player,
    auto_selection::AutoSelection,
    discovery::{self, DiscoveryEvent},
    routing::{self, RouteOutcome, SkipReason, TransportAction},
    selection::{ApplicationId, Selection, SelectionState},
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match cli::parse(std::env::args_os().skip(1)) {
        Ok(cli::Command::Help) => {
            println!("{}", cli::HELP);
            ExitCode::SUCCESS
        }
        Ok(cli::Command::Watch {
            selected,
            interactive,
            capture,
            serve,
            backend,
        }) => match watch(selected, interactive, capture, serve, backend).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("media-router: {error}");
                ExitCode::FAILURE
            }
        },
        Ok(cli::Command::RestoreBindings) => match media_router::input::restore_bindings() {
            Ok(()) => {
                println!("Binding recovery complete.");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("media-router: {error}");
                ExitCode::FAILURE
            }
        },
        Ok(cli::Command::InstallAutostart(executable)) => {
            match media_router::autostart::directory()
                .and_then(|directory| media_router::autostart::install(&directory, &executable))
            {
                Ok(path) => {
                    println!(
                        "Daemon and tray autostart installed in: {}",
                        path.parent().unwrap().display()
                    );
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("media-router: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Ok(cli::Command::RemoveAutostart) => {
            match media_router::autostart::directory()
                .and_then(|directory| media_router::autostart::remove(&directory))
            {
                Ok(removed) => {
                    println!(
                        "{}",
                        if removed {
                            "Autostart entries removed. The running daemon and tray are unchanged."
                        } else {
                            "No personal autostart entries are installed."
                        }
                    );
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("media-router: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(error) => {
            eprintln!("media-router: {error}\nRun media-router --help for usage.");
            ExitCode::from(2)
        }
    }
}

async fn watch(
    selected: Option<ApplicationId>,
    interactive: bool,
    capture_override: Option<bool>,
    serve: bool,
    backend: Backend,
) -> Result<(), Box<dyn std::error::Error>> {
    // Register before announcing readiness, including when stdin is idle.
    let mut interrupts = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminations =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let connection = zbus::connection::Builder::session()?
        .method_timeout(std::time::Duration::from_secs(2))
        .build()
        .await?;

    let mut store = if serve {
        Some(Store::open(config::directory()?)?)
    } else {
        None
    };
    let mut preferences = store
        .as_ref()
        .map(|store| store.preferences().clone())
        .unwrap_or_default();
    let explicit_selection = selected.is_some();
    if let Some(selected) = selected {
        preferences.selected = Some(selected);
    }
    if let Some(enabled) = capture_override {
        preferences.capture_enabled = enabled;
    }
    let mut selection = Selection::default();
    selection.select(preferences.selected.clone());
    let mut auto_selection = AutoSelection::default();
    auto_selection.set_enabled(serve && preferences.auto_select_new);
    let mut capture_state = CaptureState::default();
    let mut capture = None;
    let (mut server, mut requests) = if serve {
        let (server, requests) = Server::start(
            &connection,
            State::snapshot(&selection, &capture_state),
            Settings::snapshot(&preferences, String::new()),
        )
        .await?;
        (Some(server), requests)
    } else {
        (None, mpsc::channel(1).1)
    };
    // Own the bus name before persisting CLI overrides or touching capture.
    if let Some(store) = store.as_mut()
        && (explicit_selection || capture_override.is_some())
    {
        store.save(preferences.clone())?;
    }
    if serve && let Err(error) = media_router::input::recover_pending_bindings() {
        capture_state = CaptureState {
            status: "faulted".into(),
            error: error.to_string(),
        };
        publish(&server, &selection, &capture_state);
        eprintln!("CAPTURE FAULT: startup binding recovery failed: {error}");
    }
    if preferences.capture_enabled && capture_state.status != "faulted" {
        capture_state.status = "starting".into();
        publish(&server, &selection, &capture_state);
        let result = change_capture(&mut capture, &mut capture_state, true, backend).await;
        publish(&server, &selection, &capture_state);
        if let Err(error) = result {
            if !serve {
                return Err(error.into());
            }
            eprintln!("CAPTURE FAULT: {error}");
        }
    }
    eprintln!(
        "CAPTURE STATUS: backend={} desired={} actual={}",
        match backend {
            Backend::CinnamonDbus => "cinnamon-dbus",
            Backend::X11 => "x11",
        },
        preferences.capture_enabled,
        capture_state.status,
    );
    if serve {
        eprintln!("API READY: {} {}", dbus_api::NAME, dbus_api::PATH);
    }
    eprintln!("Watching MPRIS players on the session bus. Press Ctrl+C to stop.");
    eprintln!("Status and capabilities are snapshots taken when a player is discovered.");
    print_selection(selection.state());
    let (state_sender, state_receiver) = watch::channel(selection.state());
    let (event_sender, mut events) = mpsc::unbounded_channel();
    let discovery = discovery::watch_with_origin(&connection, |event, origin| {
        let _ = event_sender.send((event, origin));
    });
    tokio::pin!(discovery);
    let mut input = terminal_input(interactive);
    let mut input_open = interactive;
    let mut pending = VecDeque::<(TransportAction, SelectionState)>::new();
    let mut commands = JoinSet::new();
    if interactive {
        eprintln!("Enter play-pause, stop, previous, or next; one command per line.");
    }
    let result = async {
        loop {
            if commands.is_empty()
                && let Some((action, target)) = pending.pop_front()
            {
                let connection = connection.clone();
                let current = state_receiver.clone();
                commands.spawn(async move {
                    let outcome = routing::dispatch(&connection, &current, &target, action).await;
                    (action, outcome)
                });
            }
            if interactive && !serve && capture.is_none() && !input_open && pending.is_empty() && commands.is_empty() {
                break;
            }
            tokio::select! {
                biased;
                _ = interrupts.recv() => {
                    if !commands.is_empty() {
                        eprintln!("Exiting with a command in flight; it may already have executed. No retry will be attempted.");
                    }
                    break;
                }
                _ = terminations.recv() => {
                    if !commands.is_empty() { eprintln!("Exiting with a command in flight; it may already have executed."); }
                    break;
                }
                result = &mut discovery => { result?; break; }
                result = async { server.as_mut().expect("serving").stopped().await }, if serve => {
                    result?;
                    return Err("D-Bus state publisher stopped".into());
                }
                Some(request) = requests.recv(), if serve => {
                    if request.reply.is_closed() { continue; }
                    if matches!(request.operation, Operation::Reclaim) {
                        let result = reclaim_capture(&mut capture, &mut capture_state).await;
                        let state = publish(&server, &selection, &capture_state);
                        let settings = server.as_ref().expect("serving").settings();
                        let _ = request.reply.send(result.map(|()| Response { state, settings }));
                        continue;
                    }
                    let selection_request = matches!(request.operation, Operation::Select(_));
                    let store = store.as_mut().expect("serving configuration");
                    let service = server.as_ref().expect("serving");
                    let mut next = store.preferences().clone();
                    match &request.operation {
                        Operation::Select(application) => next.selected = application.clone(),
                        Operation::Capture(enabled) => next.capture_enabled = *enabled,
                        Operation::Reclaim => unreachable!("handled without persistence"),
                        Operation::AutoSelectNew(enabled) => next.auto_select_new = *enabled,
                        Operation::Exclusions(exclusions) => next.exclusions = exclusions.clone(),
                    }
                    let saved = save_preferences(store, service, next);
                    // After an uncertain final sync, reflect the visible replacement
                    // but still return an error; do not start external capture work.
                    selection.select(store.preferences().selected.clone());
                    auto_selection.set_enabled(store.preferences().auto_select_new);
                    state_sender.send_replace(selection.state());
                    let result = match (saved, request.operation) {
                        (Err(error), _) => Err(error),
                        (Ok(()), Operation::Capture(enabled)) => {
                            if enabled && capture_state.status != "enabled" {
                                capture_state.status = "starting".into();
                                capture_state.error.clear();
                                publish(&server, &selection, &capture_state);
                            }
                            change_capture(&mut capture, &mut capture_state, enabled, backend).await
                                .map_err(dbus_api::Error::CaptureFailed)
                        }
                        (Ok(()), _) => Ok(()),
                    };
                    let result = if result.is_ok() && selection_request && capture.is_some() {
                        reclaim_capture(&mut capture, &mut capture_state).await
                    } else { result };
                    print_selection(selection.state());
                    let snapshot = publish(&server, &selection, &capture_state);
                    let settings = service.settings();
                    let _ = request.reply.send(result.map(|()| Response { state: snapshot, settings }));
                }
                Some((event, origin)) = events.recv() => {
                    print_event(&event);
                    let candidate = store.as_ref().and_then(|store|
                        auto_selection.candidate(&selection, store.preferences(), &event, origin));
                    let before = selection.state();
                    selection.apply_event(&event);
                    if let Some(application) = candidate {
                        let store = store.as_mut().expect("automatic selection only while serving");
                        let mut next = store.preferences().clone();
                        next.selected = Some(application);
                        if let Err(error) = save_preferences(store, server.as_ref().expect("serving"), next) {
                            eprintln!("AUTO-SELECTION SAVE FAILED: {error}");
                        }
                        selection.select(store.preferences().selected.clone());
                    }
                    let after = selection.state();
                    if before != after {
                        state_sender.send_replace(after.clone());
                        print_selection(after);
                    }
                    if matches!(event, DiscoveryEvent::Added(_)) && capture.is_some()
                        && let Err(error) = reclaim_capture(&mut capture, &mut capture_state).await {
                        eprintln!("AUTOMATIC RECLAIM FAILED (player arrival): {error}");
                        if !serve { return Err(error.to_string().into()); }
                    }
                    publish(&server, &selection, &capture_state);
                }
                result = commands.join_next(), if !commands.is_empty() => {
                    let (action, outcome) = result.expect("nonempty command set")?;
                    print_outcome(action, outcome);
                }
                action = async { capture.as_mut().expect("capture enabled").next().await }, if capture_state.status == "enabled" => {
                    match action {
                        Ok(InputEvent::Action(action)) => enqueue(action, selection.state(), &mut pending),
                        Ok(InputEvent::Reclaim) => {
                            if let Err(error) = reclaim_capture(&mut capture, &mut capture_state).await {
                                eprintln!("AUTOMATIC RECLAIM FAILED (screen activation): {error}");
                                if !serve { return Err(error.to_string().into()); }
                            }
                            publish(&server, &selection, &capture_state);
                        }
                        Err(error) => {
                            let mut message = error.to_string();
                            if let Err(cleanup) = change_capture(&mut capture, &mut capture_state, false, backend).await {
                                message.push_str(&format!("; restoration failed: {cleanup}"));
                            }
                            capture_state = CaptureState { status: "faulted".into(), error: message.clone() };
                            publish(&server, &selection, &capture_state);
                            if !serve { return Err(message.into()); }
                            eprintln!("CAPTURE FAULT: {message}");
                        }
                    }
                }
                line = input.recv(), if input_open => {
                    match line {
                        None => input_open = false,
                        Some(Err(error)) => {
                            eprintln!("Input error: {error}");
                            input_open = false;
                        }
                        Some(Ok(line)) => {
                            let line = line.trim();
                            if line.is_empty() { continue; }
                            match line.parse::<TransportAction>() {
                                Err(error) => eprintln!("COMMAND INVALID: {error}"),
                                Ok(action) => enqueue(action, selection.state(), &mut pending),
                            }
                        }
                    }
                }
            }
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }.await;
    commands.abort_all();
    if let Some(capture) = capture.as_mut() {
        capture.stop().await?;
    }
    result
}

fn save_preferences(
    store: &mut Store,
    server: &Server,
    next: Preferences,
) -> Result<(), dbus_api::Error> {
    let result = store.save(next);
    let error = result
        .as_ref()
        .err()
        .map(ToString::to_string)
        .unwrap_or_default();
    server.publish_settings(Settings::snapshot(store.preferences(), error));
    result.map_err(|error| dbus_api::Error::PersistenceFailed(error.to_string()))
}

fn publish(server: &Option<Server>, selection: &Selection, capture: &CaptureState) -> State {
    let snapshot = State::snapshot(selection, capture);
    match server {
        Some(server) => server.publish(snapshot),
        None => snapshot,
    }
}

async fn change_capture(
    capture: &mut Option<Capture>,
    state: &mut CaptureState,
    enabled: bool,
    backend: Backend,
) -> Result<(), String> {
    let result = async {
        if enabled && state.status == "enabled" {
            return Ok(());
        }
        if let Some(current) = capture.as_mut() {
            current.stop().await.map_err(|e| e.to_string())?;
        }
        capture.take();
        if enabled {
            *capture = Some(Capture::start(backend).await.map_err(|e| e.to_string())?);
            eprintln!(
                "CAPTURE READY: hardware transport input registered; Ctrl+C releases capture."
            );
        } else if state.status == "faulted" {
            media_router::input::restore_bindings().map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    }
    .await;
    match &result {
        Ok(()) => {
            *state = CaptureState {
                status: if enabled { "enabled" } else { "disabled" }.into(),
                error: String::new(),
            }
        }
        Err(error) => {
            *state = CaptureState {
                status: "faulted".into(),
                error: error.clone(),
            }
        }
    }
    result
}

async fn reclaim_capture(
    capture: &mut Option<Capture>,
    state: &mut CaptureState,
) -> Result<(), dbus_api::Error> {
    let Some(current) = capture.as_mut() else {
        return Err(dbus_api::Error::CaptureFailed(
            "capture is not active; enable capture first".into(),
        ));
    };
    match current.reclaim().await {
        Ok(()) => {
            eprintln!("CAPTURE RECLAIMED: best-effort listener priority refreshed");
            Ok(())
        }
        Err(error) => {
            let mut message = error.to_string();
            if let Err(cleanup) = current.stop().await {
                message.push_str(&format!("; cleanup failed: {cleanup}"));
            }
            capture.take();
            *state = CaptureState {
                status: "faulted".into(),
                error: message.clone(),
            };
            Err(dbus_api::Error::CaptureFailed(message))
        }
    }
}

fn enqueue(
    action: TransportAction,
    target: SelectionState,
    pending: &mut VecDeque<(TransportAction, SelectionState)>,
) {
    match target {
        SelectionState::Unselected => {
            print_outcome(action, RouteOutcome::Skipped(SkipReason::NoSelection))
        }
        SelectionState::Unavailable { .. } => {
            print_outcome(action, RouteOutcome::Skipped(SkipReason::Unavailable))
        }
        target if pending.len() < 32 => pending.push_back((action, target)),
        _ => eprintln!("COMMAND SKIPPED: {action}: command queue is full"),
    }
}

fn terminal_input(enabled: bool) -> mpsc::Receiver<std::io::Result<String>> {
    let (sender, receiver) = mpsc::channel(32);
    if enabled {
        // A detached standard thread avoids Tokio waiting for a blocking stdin
        // read during shutdown. Process exit ends this thread if stdin is idle.
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let failed = line.is_err();
                if sender.blocking_send(line).is_err() || failed {
                    break;
                }
            }
        });
    }
    receiver
}

fn print_outcome(action: TransportAction, outcome: RouteOutcome) {
    match outcome {
        RouteOutcome::Acknowledged => println!("COMMAND ACKNOWLEDGED: {action}"),
        RouteOutcome::Skipped(reason) => println!("COMMAND SKIPPED: {action}: {reason:?}"),
        RouteOutcome::NotSent(error) => println!("COMMAND NOT SENT: {action}: {error:?}"),
        RouteOutcome::PlayerError(error) => println!("COMMAND PLAYER ERROR: {action}: {error:?}"),
        RouteOutcome::Uncertain(error) => {
            println!("COMMAND UNCERTAIN: {action}: {error:?}; not retried")
        }
    }
}

fn print_event(event: &DiscoveryEvent) {
    match event {
        DiscoveryEvent::Added(player) => {
            println!("PLAYER ADDED");
            print_instance(player);
            println!("  identity: {:?}", player.identity);
            println!("  status: {:?}", player.playback_status);
            let capabilities = player.capabilities;
            println!(
                "  capabilities: control={} play={} pause={} next={} previous={} seek={}",
                capabilities.can_control,
                capabilities.can_play,
                capabilities.can_pause,
                capabilities.can_go_next,
                capabilities.can_go_previous,
                capabilities.can_seek
            );
        }
        DiscoveryEvent::Removed(player) => {
            println!("PLAYER REMOVED");
            print_instance(player);
        }
        DiscoveryEvent::Skipped { service, reason } => {
            eprintln!("PLAYER SKIPPED\n  service: {service}\n  reason: {reason:?}");
        }
    }
}

fn print_instance(player: &Player) {
    println!("  application: {}", ApplicationId::from(player));
    if player.desktop_entry.is_none() {
        println!("  DesktopEntry: not supplied (using exact Identity as fallback)");
    }
    println!("  service: {}", player.service);
    println!("  owner: {}", player.owner);
}

fn print_selection(state: SelectionState) {
    match state {
        SelectionState::Unselected => println!("SELECTION NONE"),
        SelectionState::Unavailable { application } => {
            println!("SELECTION UNAVAILABLE\n  application: {application}");
        }
        SelectionState::Available {
            application,
            instance,
        } => {
            println!("SELECTION AVAILABLE\n  application: {application}");
            println!(
                "  service: {}\n  owner: {}",
                instance.service, instance.owner
            );
        }
    }
}
