//! The window process (`--ui`): shows the app window, works on profile files itself, asks the overlay for the rest.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use anyhow::{Result, anyhow};
use eframe::{CreationContext, NativeOptions};
use egui::{FontData, FontDefinitions, FontFamily, IconData, ViewportBuilder, ViewportCommand};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXICON,
    SM_CXSMICON, SendMessageW, WM_SETICON,
};
use windows::core::PCWSTR;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::host::{Command, HostView, Outcome, Reply, json_to_toml, with_display_name};
use super::link::{ToOverlay, ToWindow, receive, send};
use super::pages::{self, AppUi};
use super::render::render_pixmap;
use crate::autostart;
use crate::overlay::monitor_infos;
use crate::profile::{self, Profile};
use crate::scene::Scene;
use crate::settings;
use crate::widgets::Resources;

/// Runs until the window closes or the overlay goes away.
pub fn run(profiles_dir: PathBuf) -> Result<()> {
    let mut lines = BufReader::new(std::io::stdin()).lines();
    // The overlay speaks first: without one there is nothing to edit for.
    let Some(ToWindow::Status(status)) = receive(&mut lines) else {
        tracing::warn!("--ui is started by the overlay; use --window");
        return Ok(());
    };
    let resources = Resources::load()?;
    let host = HostView {
        version: env!("CARGO_PKG_VERSION"),
        profiles: profile_names(&profiles_dir),
        active: status.active,
        previewing: status.previewing,
        monitors: monitor_infos(),
        hotkey: status.hotkey,
        hotkey_registered: status.hotkey_registered,
        default_hotkey: settings::DEFAULT_HOTKEY,
        display: status.display,
        hide_from_capture: status.hide_from_capture,
        window_background: status.window_background,
        show_window_at_start: status.show_window_at_start,
        start_with_windows: autostart::is_enabled(),
    };
    let options = NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("Overlay Input")
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([960.0, 600.0])
            // Empty, or eframe puts its own icon over `set_window_icon`'s.
            .with_icon(IconData::default()),
        ..Default::default()
    };
    eframe::run_native(
        "overlay_input",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_fonts(system_fonts()?);
            pages::style(&cc.egui_ctx);
            set_window_icon(cc);
            let inbox = listen(lines, cc.egui_ctx.clone());
            Ok(Box::new(WindowApp {
                ui: AppUi::new(),
                host,
                profiles_dir,
                resources,
                inbox,
                waiting: VecDeque::new(),
                shown: false,
            }))
        }),
    )
    .map_err(|e| anyhow!("{e}"))
}

/// Passes overlay messages to the UI, waking it for each; a closed pipe ends the process.
fn listen(
    mut lines: impl Iterator<Item = std::io::Result<String>> + Send + 'static,
    ctx: egui::Context,
) -> Receiver<ToWindow> {
    let (sender, inbox) = mpsc::channel();
    let read = move || {
        while let Some(message) = receive(&mut lines) {
            if sender.send(message).is_err() {
                return;
            }
            ctx.request_repaint();
        }
        tracing::info!("overlay is gone; closing the window");
        std::process::exit(0);
    };
    if let Err(e) = std::thread::Builder::new()
        .name("overlay-link".into())
        .spawn(read)
    {
        tracing::error!(error = %e, "can't read from the overlay");
    }
    inbox
}

struct WindowApp {
    ui: AppUi,
    host: HostView,
    profiles_dir: PathBuf,
    resources: Resources,
    inbox: Receiver<ToWindow>,
    /// Commands the overlay hasn't answered yet, oldest first; `None` = nobody waits for that answer.
    waiting: VecDeque<Option<Command>>,
    shown: bool,
}

impl eframe::App for WindowApp {
    /// Also runs while minimized, unlike `ui`.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(message) = self.inbox.try_recv() {
            self.on_message(ctx, message);
        }
        self.close_if_asked(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !std::mem::replace(&mut self.shown, true) {
            // eframe shows the window after this frame but doesn't raise it.
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.ui.request_close() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        }
        self.host.monitors = monitor_infos();
        self.ui.show(ui, &self.host);
        let commands = self.ui.take_commands();
        let any = !commands.is_empty();
        for command in commands {
            self.run(command);
        }
        self.close_if_asked(&ctx);
        if any {
            ctx.request_repaint();
        }
    }
}

impl WindowApp {
    /// Quitting ends this process too: the overlay's exit closes the pipe.
    fn close_if_asked(&mut self, ctx: &egui::Context) {
        if !self.ui.take_close() {
            return;
        }
        if self.ui.quitting() {
            self.ask(None, ToOverlay::Quit);
        } else {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn on_message(&mut self, ctx: &egui::Context, message: ToWindow) {
        match message {
            ToWindow::Status(status) => {
                self.host.active = status.active;
                self.host.previewing = status.previewing;
                self.host.hotkey = status.hotkey;
                self.host.hotkey_registered = status.hotkey_registered;
                self.host.display = status.display;
                self.host.hide_from_capture = status.hide_from_capture;
                self.host.window_background = status.window_background;
                self.host.show_window_at_start = status.show_window_at_start;
                self.refresh_profiles();
            }
            ToWindow::Reply(result) => match self.waiting.pop_front() {
                Some(Some(command)) => self.finish(command, result.map(|()| Reply::Done)),
                Some(None) => {
                    if let Err(e) = result {
                        tracing::warn!("overlay: {e}");
                    }
                }
                None => tracing::warn!("overlay answered nothing"),
            },
            ToWindow::Focus => focus(ctx),
            ToWindow::Quit => {
                focus(ctx);
                self.ui.request_quit();
            }
        }
    }

    /// Runs a command here, or passes it to the overlay; the answer reaches the UI either way.
    fn run(&mut self, command: Command) {
        let message = match &command {
            Command::Activate(name) => ToOverlay::Activate(name.clone()),
            Command::EndPreview => ToOverlay::EndPreview,
            Command::SetHotkey(spec) => ToOverlay::SetHotkey(spec.clone()),
            Command::SetDisplay(display) => ToOverlay::SetDisplay(*display),
            Command::SetWindowBackground(color) => ToOverlay::SetWindowBackground(*color),
            Command::SetHideFromCapture(on) => ToOverlay::SetHideFromCapture(*on),
            Command::SetShowWindowAtStart(on) => ToOverlay::SetShowWindowAtStart(*on),
            Command::Preview(profile) => match json_to_toml("preview", profile) {
                Ok(text) => ToOverlay::Preview(text),
                Err(e) => return self.finish(command, Err(e)),
            },
            _ => {
                let outcome = self.run_here(&command);
                return self.finish(command, outcome);
            }
        };
        self.ask(Some(command), message);
    }

    /// Sends to the overlay. A failed write needs no handling: the reader sees the same closed pipe.
    fn ask(&mut self, command: Option<Command>, message: ToOverlay) {
        self.waiting.push_back(command);
        if let Err(e) = send(std::io::stdout(), &message) {
            tracing::debug!(error = %e, "overlay not reachable");
        }
    }

    fn finish(&mut self, command: Command, outcome: Outcome) {
        self.ui
            .on_result(command, outcome, self.host.active.as_deref());
    }

    fn run_here(&mut self, command: &Command) -> Outcome {
        match command {
            Command::Open(name) => {
                let text = self.read_profile(name)?;
                let value: toml::Value = toml::from_str(&text)
                    .map_err(|e| format!("{name}.toml is not valid TOML: {e}"))?;
                serde_json::to_value(value)
                    .map(Reply::Profile)
                    .map_err(|e| e.to_string())
            }
            Command::Save { name, profile } => {
                let text = json_to_toml(name, profile)?;
                self.build(&text)?;
                write_profile(&self.profile_path(name), &text)?;
                tracing::info!(profile = %name, "saved from the window");
                // The overlay's file watcher applies it if it's on screen.
                self.refresh_profiles();
                self.ask(None, ToOverlay::EndPreview);
                Ok(Reply::Done)
            }
            Command::Render { profile, sample } => {
                let (parsed, mut scene) = self.build(&json_to_toml("preview", profile)?)?;
                let dpi = self
                    .host
                    .monitors
                    .get(parsed.monitor)
                    .or(self.host.monitors.first())
                    .map_or(1.0, |m| m.scale);
                render_pixmap(&mut scene, dpi, *sample)
                    .map(Reply::Image)
                    .map_err(|e| format!("{e:#}"))
            }
            Command::Create(name) => {
                let path = self.free_profile_path(name)?;
                write_profile(&path, profile::STARTER_PROFILE)?;
                tracing::info!(profile = %name, "created from the window");
                self.refresh_profiles();
                Ok(Reply::Done)
            }
            Command::Duplicate { from, to } => {
                let text = self.read_profile(from)?;
                let path = self.free_profile_path(to)?;
                write_profile(&path, &with_display_name(&text, to))?;
                tracing::info!(%from, %to, "duplicated from the window");
                self.refresh_profiles();
                Ok(Reply::Done)
            }
            Command::Rename { from, to } => {
                let old = self.existing_profile_path(from)?;
                // Case-only renames ("fps" → "FPS") hit the same file on Windows.
                let new = if from.eq_ignore_ascii_case(to) {
                    self.profile_path(to)
                } else {
                    self.free_profile_path(to)?
                };
                let text = self.read_profile(from)?;
                std::fs::rename(&old, &new).map_err(|e| format!("rename {from} → {to}: {e}"))?;
                write_profile(&new, &with_display_name(&text, to))?;
                tracing::info!(%from, %to, "renamed from the window");
                self.refresh_profiles();
                if self.host.active.as_deref() == Some(from.as_str()) {
                    self.ask(None, ToOverlay::Activate(to.clone()));
                }
                Ok(Reply::Done)
            }
            Command::Delete(name) => {
                let path = self.existing_profile_path(name)?;
                if self.host.active.as_deref() == Some(name.as_str()) {
                    return Err(format!(
                        "{name} is on the overlay; show another profile first"
                    ));
                }
                std::fs::remove_file(&path).map_err(|e| format!("delete {name}.toml: {e}"))?;
                tracing::info!(profile = %name, "deleted from the window");
                self.refresh_profiles();
                Ok(Reply::Done)
            }
            Command::SetStartWithWindows(on) => {
                autostart::set(*on).map_err(|e| format!("{e:#}"))?;
                self.host.start_with_windows = autostart::is_enabled();
                Ok(Reply::Done)
            }
            Command::OpenProfilesFolder => {
                std::process::Command::new("explorer")
                    .arg(&self.profiles_dir)
                    .spawn()
                    .map_err(|e| format!("open the profiles folder: {e}"))?;
                Ok(Reply::Done)
            }
            Command::OpenLogsFolder => {
                // Dev builds log to the console, so the folder may not exist yet.
                let logs = settings::app_dir(&self.profiles_dir).join("logs");
                std::fs::create_dir_all(&logs).map_err(|e| format!("logs folder: {e}"))?;
                std::process::Command::new("explorer")
                    .arg(&logs)
                    .spawn()
                    .map_err(|e| format!("open the logs folder: {e}"))?;
                Ok(Reply::Done)
            }
            _ => unreachable!("the overlay runs the others"),
        }
    }

    /// The same checks the overlay runs on load.
    fn build(&self, text: &str) -> Result<(Profile, Scene), String> {
        let parsed = Profile::parse(text).map_err(|e| format!("{e:#}"))?;
        let scene = Scene::build(&parsed, &self.resources).map_err(|e| format!("{e:#}"))?;
        Ok((parsed, scene))
    }

    fn refresh_profiles(&mut self) {
        self.host.profiles = profile_names(&self.profiles_dir);
    }

    fn profile_path(&self, name: &str) -> PathBuf {
        self.profiles_dir.join(format!("{name}.toml"))
    }

    fn existing_profile_path(&self, name: &str) -> Result<PathBuf, String> {
        let path = self.profile_path(name);
        path.is_file()
            .then_some(path)
            .ok_or_else(|| format!("profile {name:?} not found"))
    }

    /// Path for a new profile; refuses names already taken (case-insensitive, like Windows).
    fn free_profile_path(&self, name: &str) -> Result<PathBuf, String> {
        let path = self.profile_path(name);
        if path.exists() {
            return Err(format!("a profile named {name:?} already exists"));
        }
        Ok(path)
    }

    fn read_profile(&self, name: &str) -> Result<String, String> {
        std::fs::read_to_string(self.existing_profile_path(name)?)
            .map_err(|e| format!("read {name}.toml: {e}"))
    }
}

fn focus(ctx: &egui::Context) {
    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(ViewportCommand::Focus);
}

fn profile_names(dir: &Path) -> Vec<String> {
    profile::list_profiles(dir)
        .iter()
        .map(|p| profile::display_name(p))
        .collect()
}

fn write_profile(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        format!("write {name}: {e}")
    })
}

/// The exe's own icon (resource 1); eframe only takes pixels.
fn set_window_icon(cc: &CreationContext) {
    let Ok(RawWindowHandle::Win32(handle)) = cc.window_handle().map(|h| h.as_raw()) else {
        return;
    };
    let hwnd = HWND(handle.hwnd.get() as _);
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        for (kind, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
            let size = GetSystemMetrics(metric);
            let resource = PCWSTR(1 as _);
            if let Ok(icon) = LoadImageW(
                Some(module.into()),
                resource,
                IMAGE_ICON,
                size,
                size,
                LR_DEFAULTCOLOR,
            ) {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(kind as usize)),
                    Some(LPARAM(icon.0 as isize)),
                );
            }
        }
    }
}

/// Segoe UI for text, Segoe UI Symbol for arrows and marks, Consolas for code.
pub(super) fn system_fonts() -> Result<FontDefinitions> {
    let dir = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| "C:\\Windows".into())
        .join("Fonts");
    let mut fonts = FontDefinitions::empty();
    let mut load = |name: &str, file: &str| match std::fs::read(dir.join(file)) {
        Ok(bytes) => {
            fonts
                .font_data
                .insert(name.to_owned(), Arc::new(FontData::from_owned(bytes)));
            true
        }
        Err(e) => {
            tracing::warn!(font = file, error = %e, "font missing");
            false
        }
    };
    let text = load("segoe", "segoeui.ttf");
    let symbols = load("symbols", "seguisym.ttf");
    let mono = load("consolas", "consola.ttf");
    if !text && !symbols && !mono {
        return Err(anyhow!("no usable font in {}", dir.display()));
    }
    let pick = |list: &[(bool, &str)]| -> Vec<String> {
        list.iter()
            .filter(|(ok, _)| *ok)
            .map(|(_, n)| (*n).to_owned())
            .collect()
    };
    fonts.families.insert(
        FontFamily::Proportional,
        pick(&[(text, "segoe"), (symbols, "symbols")]),
    );
    fonts.families.insert(
        FontFamily::Monospace,
        pick(&[(mono, "consolas"), (text, "segoe"), (symbols, "symbols")]),
    );
    if fonts.families[&FontFamily::Proportional].is_empty() {
        let fallback = fonts.families[&FontFamily::Monospace].clone();
        fonts.families.insert(FontFamily::Proportional, fallback);
    }
    Ok(fonts)
}
