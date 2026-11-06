// Release builds have no console window; debug builds keep it for logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod dialog;
mod input;
mod instance;
mod logfile;
mod overlay;
mod profile;
mod render;
mod scene;
mod settings;
mod tray;
mod ui;
mod widgets;

use std::path::Path;

use global_hotkey::GlobalHotKeyEvent;
use tracing::info;
use tracing_subscriber::EnvFilter;
use tray_icon::TrayIconEvent;
use tray_icon::menu::MenuEvent;
use winit::event_loop::{DeviceEvents, EventLoop};

use crate::app::{App, Launch, UserEvent};

/// Warnings and errors only; `RUST_LOG` overrides it (e.g. `overlay_input=debug`).
const LOG_FILTER: &str = "warn,gilrs=error,gilrs_core=off";

fn main() -> anyhow::Result<()> {
    let launch = Launch::from_args(std::env::args().skip(1));
    if launch.ui {
        return run_window_process();
    }
    if !instance::claim() {
        // Already running: open its window and leave.
        overlay::request_open_window();
        return Ok(());
    }
    let profiles_dir = profile::profiles_dir()?;
    init_logging(&settings::app_dir(&profiles_dir));
    std::panic::set_hook(Box::new(|info| tracing::error!("panic: {info}")));

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "overlay input starting"
    );
    // After logging starts, so what gets created shows in the log.
    profile::seed_profiles(&profiles_dir)?;
    let result = run(profiles_dir, launch);
    if let Err(e) = &result {
        tracing::error!("fatal: {e:#}");
    }
    result
}

/// `--ui`: the app window, started by the overlay. Stdout is its pipe to the overlay.
fn run_window_process() -> anyhow::Result<()> {
    let profiles_dir = profile::profiles_dir()?;
    init_logging(&settings::app_dir(&profiles_dir));
    std::panic::set_hook(Box::new(|info| tracing::error!("panic: {info}")));
    let result = ui::run(profiles_dir);
    if let Err(e) = &result {
        tracing::error!("app window: {e:#}");
        dialog::error(
            "Overlay Input",
            &format!("The app window can't open:\n{e:#}"),
        );
    }
    result
}

fn run(profiles_dir: std::path::PathBuf, launch: Launch) -> anyhow::Result<()> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    // Raw Input with RIDEV_INPUTSINK: keep receiving input while a game has focus.
    event_loop.listen_device_events(DeviceEvents::Always);
    let proxy = event_loop.create_proxy();
    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = menu_proxy.send_event(UserEvent::Menu(event));
    }));
    // Runs synchronously before the tray menu opens; detach is delivered from inside the menu loop.
    let icon_proxy = proxy.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        if let Some(thread) = tray::on_tray_icon_event(&event) {
            let _ = icon_proxy.send_event(UserEvent::DetachInput(thread));
        }
        if tray::is_open_request(&event) {
            let _ = icon_proxy.send_event(UserEvent::OpenWindow);
        }
    }));
    let hotkey_proxy = proxy.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |event| {
        let _ = hotkey_proxy.send_event(UserEvent::Hotkey(event));
    }));

    let mut app = App::new(proxy, profiles_dir, launch);
    event_loop.run_app(&mut app)?;

    info!("shutdown");
    app.into_result()
}

/// `info` by default (gilrs noise muted); override with `RUST_LOG`, e.g. `RUST_LOG=overlay_input=debug`.
/// Debug builds log to the console; release builds to `logs/overlay_input-YYYY-MM-DD.log` in `dir`, one file per day.
fn init_logging(dir: &Path) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(LOG_FILTER));
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_timer(logfile::LocalTime);
    if cfg!(debug_assertions) {
        return builder.with_writer(std::io::stderr).init();
    }
    match logfile::DailyLog::new(dir.join("logs")) {
        Ok(log) => builder.with_ansi(false).with_writer(log).init(),
        // No writable folder: nothing to log to (there is no console either).
        Err(_) => builder.with_writer(std::io::sink).init(),
    }
}
