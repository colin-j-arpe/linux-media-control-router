mod cli;

use std::process::ExitCode;

use media_router::player::{
    Player,
    discovery::{self, DiscoveryEvent},
    selection::{ApplicationId, Selection, SelectionState},
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match cli::parse(std::env::args_os().skip(1)) {
        Ok(cli::Command::Help) => {
            println!("{}", cli::HELP);
            ExitCode::SUCCESS
        }
        Ok(cli::Command::Watch(selected)) => match watch(selected).await {
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

async fn watch(selected: Option<ApplicationId>) -> Result<(), Box<dyn std::error::Error>> {
    let connection = zbus::connection::Builder::session()?
        .method_timeout(std::time::Duration::from_secs(2))
        .build()
        .await?;

    eprintln!("Watching MPRIS players on the session bus. Press Ctrl+C to stop.");
    eprintln!("Status and capabilities are snapshots taken when a player is discovered.");
    let mut selection = Selection::default();
    selection.select(selected);
    print_selection(selection.state());
    tokio::select! {
        result = discovery::watch(&connection, |event| {
            print_event(&event);
            if let Some(state) = selection.apply_event(&event) {
                print_selection(state);
            }
        }) => result?,
        result = tokio::signal::ctrl_c() => result?,
    }
    Ok(())
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
