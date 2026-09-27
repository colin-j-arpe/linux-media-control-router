mod cli;

use std::{collections::VecDeque, io::BufRead, process::ExitCode};

use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
};

use media_router::player::{
    Player,
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
        }) => match watch(selected, interactive).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("media-router: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("media-router: {error}\nRun media-router --help for usage.");
            ExitCode::from(2)
        }
    }
}

async fn watch(
    selected: Option<ApplicationId>,
    interactive: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Register before announcing readiness, including when stdin is idle.
    let mut interrupts = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let connection = zbus::connection::Builder::session()?
        .method_timeout(std::time::Duration::from_secs(2))
        .build()
        .await?;

    eprintln!("Watching MPRIS players on the session bus. Press Ctrl+C to stop.");
    eprintln!("Status and capabilities are snapshots taken when a player is discovered.");
    let mut selection = Selection::default();
    selection.select(selected);
    print_selection(selection.state());
    let (state_sender, state_receiver) = watch::channel(selection.state());
    let (event_sender, mut events) = mpsc::unbounded_channel();
    let discovery = discovery::watch(&connection, |event| {
        let _ = event_sender.send(event);
    });
    tokio::pin!(discovery);
    let mut input = terminal_input(interactive);
    let mut input_open = interactive;
    let mut pending = VecDeque::<(TransportAction, SelectionState)>::new();
    let mut commands = JoinSet::new();
    if interactive {
        eprintln!("Enter play-pause, stop, previous, or next; one command per line.");
    }
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
        if interactive && !input_open && pending.is_empty() && commands.is_empty() {
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
            result = &mut discovery => { result?; break; }
            Some(event) = events.recv() => {
                print_event(&event);
                if let Some(state) = selection.apply_event(&event) {
                    state_sender.send_replace(state.clone());
                    print_selection(state);
                }
            }
            result = commands.join_next(), if !commands.is_empty() => {
                let (action, outcome) = result.expect("nonempty command set")?;
                print_outcome(action, outcome);
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
                            Ok(action) => match selection.state() {
                                SelectionState::Unselected => print_outcome(action, RouteOutcome::Skipped(SkipReason::NoSelection)),
                                SelectionState::Unavailable { .. } => print_outcome(action, RouteOutcome::Skipped(SkipReason::Unavailable)),
                                target => {
                                    if pending.len() < 32 {
                                        pending.push_back((action, target));
                                    } else {
                                        eprintln!("COMMAND SKIPPED: {action}: command queue is full");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
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
