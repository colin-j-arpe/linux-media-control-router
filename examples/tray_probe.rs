//! Standalone Cinnamon compatibility probe. All controls affect sample data only.
//! Run with `cargo run --locked --example tray_probe -- --seconds 600`.
use ksni::{TrayMethods, menu::*};
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;

struct Probe {
    selected: usize,
    unavailable: bool,
    mro: bool,
    excluded: bool,
    extra: bool,
    quit: Arc<Notify>,
}

impl Probe {
    fn label(&self) -> &'static str {
        [
            "Sample music player",
            "Sample browser",
            "Sample player — 音楽",
        ][self.selected]
    }
    fn report(&self, action: &str) {
        println!(
            "PROBE EVENT: {action}; selected={}; unavailable={}; mro={}; excluded={}; extra={}",
            self.label(),
            self.unavailable,
            self.mro,
            self.excluded,
            self.extra
        );
    }
}

impl ksni::Tray for Probe {
    // Ask the tray host to open the menu on primary (usually left-click) activation.
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "media-router-tray-probe".into()
    }
    fn title(&self) -> String {
        "Media Router compatibility probe".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [24, 32, 48]
            .into_iter()
            .map(|size| icon(size, self.unavailable))
            .collect()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.title(),
            description: format!(
                "{}: {} (sample data only)",
                self.label(),
                if self.unavailable {
                    "unavailable"
                } else {
                    "available"
                }
            ),
            ..Default::default()
        }
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut options = vec![
            RadioItem {
                label: "Sample music player".into(),
                ..Default::default()
            },
            RadioItem {
                label: "Sample browser".into(),
                ..Default::default()
            },
        ];
        if self.extra {
            options.push(RadioItem {
                label: "Sample player — 音楽".into(),
                ..Default::default()
            });
        }
        vec![
            StandardItem {
                label: "Media Router — compatibility probe".into(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: format!(
                    "Selected: {} ({})",
                    self.label(),
                    if self.unavailable {
                        "unavailable"
                    } else {
                        "available"
                    }
                ),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            RadioGroup {
                selected: self.selected,
                options,
                select: Box::new(|this: &mut Self, selected| {
                    this.selected = selected;
                    this.report("select");
                }),
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "Simulate unavailable player".into(),
                checked: self.unavailable,
                activate: Box::new(|this: &mut Self| {
                    this.unavailable = !this.unavailable;
                    this.report("unavailable");
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Demo MRO toggle (memory only)".into(),
                checked: self.mro,
                activate: Box::new(|this: &mut Self| {
                    this.mro = !this.mro;
                    this.report("mro");
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Show extra sample player".into(),
                checked: self.extra,
                activate: Box::new(|this: &mut Self| {
                    this.extra = !this.extra;
                    if !this.extra && this.selected == 2 {
                        this.selected = 0;
                    }
                    this.report("inventory");
                }),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Demo exclusions".into(),
                submenu: vec![
                    CheckmarkItem {
                        label: "Exclude sample browser (memory only)".into(),
                        checked: self.excluded,
                        activate: Box::new(|this: &mut Self| {
                            this.excluded = !this.excluded;
                            this.report("exclusion");
                        }),
                        ..Default::default()
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit compatibility probe".into(),
                activate: Box::new(|this: &mut Self| {
                    this.report("quit");
                    this.quit.notify_one();
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
    fn menu_about_to_show(&mut self) {
        println!("PROBE MENU: host requested menu");
    }
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        eprintln!("PROBE HOST LOST: {reason:?}");
        self.quit.notify_one();
        false
    }
}

/// Direct ARGB artwork keeps the probe independent of installed icon themes.
/// The badge is composed into the main icon, without relying on host overlays.
fn icon(size: i32, unavailable: bool) -> ksni::Icon {
    let mut argb_data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (x, y) = (
                (x as f32 + 0.5) / size as f32,
                (y as f32 + 0.5) / size as f32,
            );
            let mut pixel = [0, 0, 0, 0];
            if (x - 0.5).powi(2) + (y - 0.5).powi(2) < 0.45_f32.powi(2) {
                pixel = [255, 40, 140, 230];
            }
            if (0.35..0.73).contains(&x) && (y - 0.5).abs() < (0.73 - x) * 0.65 {
                pixel = [255, 255, 255, 255];
            }
            if unavailable && (x - 0.78).powi(2) + (y - 0.22).powi(2) < 0.21_f32.powi(2) {
                pixel = [255, 220, 35, 45];
                if (0.75..0.81).contains(&x)
                    && ((0.08..0.24).contains(&y) || (0.28..0.34).contains(&y))
                {
                    pixel = [255, 255, 255, 255];
                }
            }
            argb_data.extend_from_slice(&pixel);
        }
    }
    ksni::Icon {
        width: size,
        height: size,
        data: argb_data,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let seconds = match arguments.as_slice() {
        [] => 300,
        [flag] if flag == "--help" => {
            println!(
                "Usage: cargo run --locked --example tray_probe -- [--seconds N]\nSample data only. Exits after N seconds (default 300), Quit, Ctrl+C, or SIGTERM."
            );
            return Ok(());
        }
        [flag, value] if flag == "--seconds" => value.parse::<u64>()?,
        _ => return Err("expected --seconds N or --help".into()),
    };
    if !(1..=3600).contains(&seconds) {
        return Err("seconds must be between 1 and 3600".into());
    }
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let quit = Arc::new(Notify::new());
    let handle = Probe {
        selected: 0,
        unavailable: false,
        mro: false,
        excluded: false,
        extra: false,
        quit: quit.clone(),
    }
    .spawn()
    .await?;
    println!(
        "PROBE READY: pid={}; sample controls only; automatic exit in {seconds}s",
        std::process::id()
    );
    tokio::select! {
        _ = interrupt.recv() => (),
        _ = terminate.recv() => (),
        _ = quit.notified() => (),
        _ = tokio::time::sleep(Duration::from_secs(seconds)) => println!("PROBE TIME LIMIT"),
    }
    handle.shutdown().await;
    println!("PROBE STOPPED");
    Ok(())
}
