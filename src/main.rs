use media_router::player::{
    Player,
    discovery::{self, DiscoveryEvent},
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let connection = zbus::connection::Builder::session()?
        .method_timeout(std::time::Duration::from_secs(2))
        .build()
        .await?;

    eprintln!("Watching MPRIS players on the session bus. Press Ctrl+C to stop.");
    eprintln!("Status and capabilities are snapshots taken when a player is discovered.");
    tokio::select! {
        result = discovery::watch(&connection, print_event) => result?,
        result = tokio::signal::ctrl_c() => result?,
    }
    Ok(())
}

fn print_event(event: DiscoveryEvent) {
    match event {
        DiscoveryEvent::Added(player) => {
            println!("PLAYER ADDED");
            print_instance(&player);
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
            print_instance(&player);
        }
        DiscoveryEvent::Skipped { service, reason } => {
            eprintln!("PLAYER SKIPPED\n  service: {service}\n  reason: {reason:?}");
        }
    }
}

fn print_instance(player: &Player) {
    println!(
        "  application: {:?}",
        player.desktop_entry.as_deref().unwrap_or(&player.service)
    );
    if player.desktop_entry.is_none() {
        println!("  DesktopEntry: not supplied (application label is the service name)");
    }
    println!("  service: {}", player.service);
    println!("  owner: {}", player.owner);
}
