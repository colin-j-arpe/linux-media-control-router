use ksni::TrayMethods;
use media_router_tray::{client, model::View, tray::Tray};
use std::{sync::Arc, time::Duration};
use tokio::sync::{Notify, mpsc, watch};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!(
            "Usage: media-router-tray [--wait-for-host]\n\nConnect to media-router --serve on the session bus.\n--wait-for-host waits for the desktop tray host during login startup.\nRequires a StatusNotifier tray host. Right-click to open the menu.\nQuitting closes only the tray, not the daemon. No preferences are stored by this client."
        );
        return Ok(());
    }
    let wait_for_host = args.len() == 1 && args[0] == "--wait-for-host";
    if !args.is_empty() && !wait_for_host {
        return Err("unknown arguments; use --help".into());
    }
    let connection = zbus::connection::Builder::session()?
        .method_timeout(Duration::from_secs(5))
        .build()
        .await?;
    // One tray per session; never replace another GUI or queue for its name.
    connection
        .request_name_with_flags(
            "org.mediarouter.Tray1",
            zbus::fdo::RequestNameFlags::DoNotQueue.into(),
        )
        .await?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let quit = Arc::new(Notify::new());
    let (commands, inbox) = mpsc::channel(1);
    let (views, mut updates) = watch::channel(View::default());
    let handle = Tray {
        view: View::default(),
        commands,
        quit: quit.clone(),
        wait_for_host,
    }
    .assume_sni_available(wait_for_host)
    .spawn()
    .await?;
    let worker = tokio::spawn(client::run(connection.clone(), inbox, views));
    loop {
        tokio::select! {
            _ = interrupt.recv() => break,
            _ = terminate.recv() => break,
            _ = quit.notified() => break,
            changed = updates.changed() => {
                if changed.is_err() { break; }
                let view = updates.borrow_and_update().clone();
                handle.update(move |tray| tray.view = view).await;
            }
        }
    }
    worker.abort();
    let _ = worker.await;
    handle.shutdown().await;
    Ok(())
}
