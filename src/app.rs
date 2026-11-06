//! winit application: owns window, tray and the active profile's scene; redraws on input, animates, idles otherwise.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use notify::RecommendedWatcher;
use tiny_skia::{Color, Pixmap};
use tray_icon::menu::MenuEvent;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, DeviceEvents, EventLoopProxy};
use winit::window::WindowId;

use crate::dialog;
use crate::input::gamepad::GamepadThreads;
use crate::input::{self, InputEvent, InputState, PadEvent};
use crate::overlay::{
    ColorWindow, ForegroundWatcher, OverlayWindow, take_display_changed, take_open_window_requested,
};
use crate::profile::{self, Profile};
use crate::render::glow::GlowCache;
use crate::scene::Scene;
use crate::settings::{self, Display, Output, Settings, State};
use crate::tray::{Tray, TrayAction};
use crate::ui::{Status, ToOverlay, ToWindow, WindowProcess};
use crate::widgets::{Canvas, Resources};

const FALLBACK_FPS: f64 = 60.0;
/// Editors save in several writes; wait for them to settle.
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(200);
const DEFAULT_PROFILE: &str = "default.toml";
const SLOW_FRAME_LOG_EVERY: Duration = Duration::from_secs(10);

/// Command-line switches.
#[derive(Debug, Default, Clone, Copy)]
pub struct Launch {
    /// `--window`: open the app window right away.
    pub window: bool,
    /// `--ui`: be the app window process, started by the overlay.
    pub ui: bool,
}

impl Launch {
    pub fn from_args(args: impl Iterator<Item = String>) -> Self {
        let mut launch = Self::default();
        for arg in args {
            match arg.as_str() {
                "--window" => launch.window = true,
                "--ui" => launch.ui = true,
                other => tracing::warn!(arg = other, "unknown argument ignored"),
            }
        }
        launch
    }
}

pub enum UserEvent {
    Menu(MenuEvent),
    ProfilesChanged,
    /// Release the input attach made so the tray menu could take focus.
    DetachInput(u32),
    Gamepad(PadEvent),
    Hotkey(GlobalHotKeyEvent),
    /// Double-click on the tray icon.
    OpenWindow,
    /// The app window process asked for something.
    AppWindow(ToOverlay),
    /// The app window process ended.
    AppWindowClosed,
}

pub struct App {
    proxy: EventLoopProxy<UserEvent>,
    launch: Launch,
    tray: Option<Tray>,
    window: Option<OverlayWindow>,
    resources: Option<Resources>,
    scene: Option<Scene>,
    pixmap: Option<Pixmap>,
    /// Highlight layer, only for profiles with `highlight_opacity`.
    highlight: Option<Pixmap>,
    glow: GlowCache,
    input: InputState,
    profiles_dir: PathBuf,
    /// Holds settings.toml, state.toml and the log.
    app_dir: PathBuf,
    profiles: Vec<PathBuf>,
    active_profile: Option<PathBuf>,
    _watcher: Option<RecommendedWatcher>,
    /// Started on first profile with a gamepad widget; paused when hidden or unused.
    gamepads: Option<GamepadThreads>,
    /// Kept for the app's lifetime so the hotkey can be swapped; dropping it unregisters everything.
    hotkey_manager: Option<GlobalHotKeyManager>,
    /// Show/hide hotkey currently registered.
    hotkey: Option<HotKey>,
    /// `settings.toml`; `toggle_hotkey` stays as set even when it couldn't be registered.
    settings: Settings,
    _foreground: Option<ForegroundWatcher>,
    /// Remembered between runs (`state.toml`).
    state: State,
    /// Nothing shows and input isn't read; `state.output` is where it comes back.
    hidden: bool,
    /// Shows the overlay on a solid color, for OBS; only in `Display::Window`.
    color_window: Option<ColorWindow>,
    /// App window process; only exists while it runs.
    app_window: Option<WindowProcess>,
    /// The scene on screen is an unsaved preview, not `active_profile`.
    previewing: bool,
    reload_at: Option<Instant>,
    /// Frames over budget, the worst one (total, draw, present), and when counting started.
    slow_frames: (u32, (Duration, Duration, Duration), Instant),
    /// Draw and present time of the last frame.
    frame_parts: (Duration, Duration),
    dirty: bool,
    animating: bool,
    next_frame: Instant,
    frame_interval: Duration,
    error: Option<anyhow::Error>,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>, profiles_dir: PathBuf, launch: Launch) -> Self {
        Self {
            proxy,
            launch,
            tray: None,
            window: None,
            resources: None,
            scene: None,
            pixmap: None,
            highlight: None,
            glow: GlowCache::default(),
            input: InputState::default(),
            app_dir: settings::app_dir(&profiles_dir),
            profiles_dir,
            profiles: Vec::new(),
            active_profile: None,
            _watcher: None,
            gamepads: None,
            hotkey_manager: None,
            hotkey: None,
            settings: Settings::default(),
            _foreground: None,
            app_window: None,
            state: State::default(),
            hidden: false,
            color_window: None,
            previewing: false,
            reload_at: None,
            slow_frames: (0, Default::default(), Instant::now()),
            frame_parts: Default::default(),
            dirty: false,
            animating: false,
            next_frame: Instant::now(),
            frame_interval: Duration::from_secs_f64(1.0 / FALLBACK_FPS),
            error: None,
        }
    }

    pub fn into_result(self) -> Result<()> {
        self.error.map_or(Ok(()), Err)
    }

    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        self.tray = Some(Tray::create()?);
        if let Some(mhz) = event_loop
            .primary_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
        {
            self.frame_interval = Duration::from_secs_f64(1000.0 / mhz as f64);
        }
        self.resources = Some(Resources::load()?);
        let first_run = !State::exists(&self.app_dir);
        self.state = State::load_or_create(&self.app_dir);
        let window = OverlayWindow::create()?;
        self._foreground = ForegroundWatcher::install(window.hwnd());
        if self.state.hide_from_capture {
            window.set_hidden_from_capture(true);
        }
        self.window = Some(window);

        self.settings = Settings::load_or_create(&self.app_dir);
        let hotkey = self.settings.toggle_hotkey.clone();
        if let Err(e) = self.set_hotkey(&hotkey) {
            // Usually: taken by another app, or a typo in settings.toml.
            tracing::error!(%hotkey, "show/hide hotkey not registered: {e:#}");
        }

        let proxy = self.proxy.clone();
        self._watcher = profile::watch::watch(&self.profiles_dir, move || {
            let _ = proxy.send_event(UserEvent::ProfilesChanged);
        })
        .inspect_err(|e| tracing::warn!(error = %e, "hot reload disabled"))
        .ok();

        self.refresh_profile_list();
        if !self.activate_initial_profile()? {
            event_loop.exit();
            return Ok(());
        }
        tracing::info!(dir = ?self.profiles_dir, frame_ms = self.frame_interval.as_secs_f32() * 1000.0, "ready");
        self.set_display(event_loop, self.state.output.into());
        if self.launch.window || first_run || self.settings.show_window_at_start {
            self.open_app_window();
        }
        Ok(())
    }

    fn save_state(&self) {
        if let Err(e) = self.state.save(&self.app_dir) {
            tracing::warn!("{e:#}");
        }
    }

    /// Shows the overlay on the screen or in its window, or hides it. Only hiding stops reading input.
    fn set_display(&mut self, event_loop: &ActiveEventLoop, display: Display) {
        let mut display = display;
        if display == Display::Window && !self.open_color_window(event_loop) {
            display = Display::Screen;
        }
        if display != Display::Window {
            self.close_color_window();
        }
        match display {
            Display::Screen => self.state.output = Output::Screen,
            Display::Window => self.state.output = Output::Window,
            Display::Hidden => {}
        }
        if let Some(window) = &mut self.window {
            window.set_visible(display == Display::Screen);
        }
        self.hidden = display == Display::Hidden;
        // Hidden = no input capture at all (raw input unregistered).
        event_loop.listen_device_events(if self.hidden {
            DeviceEvents::Never
        } else {
            DeviceEvents::Always
        });
        if self.hidden {
            self.input = InputState::default();
            if let Some(scene) = &mut self.scene {
                scene.reset();
            }
        }
        self.dirty = !self.hidden;
        self.sync_gamepads();
        self.save_state();
        if let Some(tray) = &mut self.tray {
            tray.set_hidden(self.hidden);
        }
        self.send_status();
        tracing::info!(mode = ?self.display(), "overlay display");
    }

    fn display(&self) -> Display {
        if self.hidden {
            Display::Hidden
        } else {
            self.state.output.into()
        }
    }

    /// Hotkey and tray: hide, or come back where it was.
    fn toggle_hidden(&mut self, event_loop: &ActiveEventLoop) {
        let display = if self.hidden {
            self.state.output.into()
        } else {
            Display::Hidden
        };
        self.set_display(event_loop, display);
    }

    fn set_window_background(&mut self, color: [u8; 3]) {
        self.state.window_background = color;
        if let Some(window) = &mut self.color_window {
            window.set_background(color);
        }
        self.dirty = true;
        self.save_state();
        self.send_status();
    }

    fn set_hide_from_capture(&mut self, hide: bool) {
        self.state.hide_from_capture = hide;
        if let Some(window) = &self.window {
            window.set_hidden_from_capture(hide);
        }
        self.save_state();
        self.send_status();
    }

    /// Asks the app window first if it's open: it may have unsaved changes.
    fn quit(&self, event_loop: &ActiveEventLoop) {
        match &self.app_window {
            Some(app_window) => app_window.send(&ToWindow::Quit),
            None => event_loop.exit(),
        }
    }

    /// Opens the overlay window at its remembered size (else the overlay's), or brings it to the front.
    fn open_color_window(&mut self, event_loop: &ActiveEventLoop) -> bool {
        if let Some(window) = &self.color_window {
            window.focus();
            return true;
        }
        let size = self.state.window_size.unwrap_or_else(|| {
            self.pixmap
                .as_ref()
                .map_or([960, 540], |p| [p.width().max(320), p.height().max(180)])
        });
        match ColorWindow::open(event_loop, size, self.state.window_background) {
            Ok(window) => {
                self.color_window = Some(window);
                true
            }
            Err(e) => {
                tracing::error!("overlay window: {e:#}");
                false
            }
        }
    }

    fn close_color_window(&mut self) {
        if let Some(window) = self.color_window.take() {
            self.state.window_size = Some(window.size());
        }
    }

    /// Last used profile, else `default.toml`. A broken `default.toml` asks the user: replace it, or close.
    /// Returns false when the user chose to close.
    fn activate_initial_profile(&mut self) -> Result<bool> {
        let default = profile::find_file(&self.profiles_dir, DEFAULT_PROFILE)
            .unwrap_or_else(|| self.profiles_dir.join(DEFAULT_PROFILE));
        let remembered = self
            .state
            .last_profile
            .clone()
            .and_then(|name| profile::find_file(&self.profiles_dir, &name))
            .filter(|path| *path != default);
        if let Some(path) = remembered {
            self.activate_profile(&path)?;
            if self.active_profile.as_deref() == Some(path.as_path()) {
                return Ok(true);
            }
        }

        if let Err(e) = self.check_profile(&default) {
            tracing::error!("{DEFAULT_PROFILE} can't be loaded: {e:#}");
            let text = format!(
                "The default profile can't be loaded:\n{}\n\n{}\n\n\
                 Replace it with the original default profile?\n\
                 Yes: your file is kept as a .bak backup next to it.\n\
                 No: close Overlay Input so you can fix the file.",
                default.display(),
                short_error(&e),
            );
            if !dialog::ask_yes_no("Overlay Input: default profile is broken", &text) {
                tracing::info!("closing: the user kept the broken {DEFAULT_PROFILE}");
                return Ok(false);
            }
            let backup = profile::replace_with_builtin(&default)?;
            tracing::warn!(
                ?backup,
                "{DEFAULT_PROFILE} replaced; the broken file was backed up"
            );
            self.refresh_profile_list();
        }
        self.activate_profile(&default)?;
        Ok(true)
    }

    /// The checks a profile must pass to be shown: parse, then build every widget.
    fn check_profile(&self, path: &Path) -> Result<()> {
        let profile = Profile::load(path)?;
        Scene::build(&profile, self.resources()?).map(drop)
    }

    /// Loads `path`; on failure keeps the current scene (or the built-in one at startup).
    fn activate_profile(&mut self, path: &Path) -> Result<()> {
        self.set_previewing(false);
        let built = Profile::load(path).and_then(|p| {
            let scene = Scene::build(&p, self.resources()?)?;
            Ok((profile::label(&p, path), scene))
        });
        match built {
            Ok((name, scene)) => {
                self.install_scene(scene)?;
                self.active_profile = Some(path.to_path_buf());
                if let Some(tray) = &mut self.tray {
                    tray.set_profile_name(&name);
                }
                tracing::info!(profile = %name, "profile loaded");
                self.state.last_profile =
                    path.file_name().map(|n| n.to_string_lossy().into_owned());
                self.save_state();
            }
            Err(e) => {
                tracing::error!("{e:#}");
                if self.scene.is_none() {
                    self.install_builtin()?;
                }
            }
        }
        self.sync_tray_selection();
        Ok(())
    }

    fn install_builtin(&mut self) -> Result<()> {
        let profile = Profile::parse(profile::BUILTIN_DEFAULT).context("built-in profile")?;
        let scene = Scene::build(&profile, self.resources()?)?;
        self.install_scene(scene)
    }

    fn install_scene(&mut self, scene: Scene) -> Result<()> {
        self.scene = Some(scene);
        self.fit_to_display()?;
        self.sync_gamepads();
        Ok(())
    }

    /// Re-reads monitor work area and DPI, and resizes the canvas to match.
    fn fit_to_display(&mut self) -> Result<()> {
        let (Some(window), Some(scene)) = (self.window.as_mut(), self.scene.as_ref()) else {
            return Ok(());
        };
        window.set_monitor(scene.monitor)?;
        let scale = window.scale() * scene.scale;
        let (w, h) = scene.size;
        let (pw, ph) = ((w * scale).ceil() as u32, (h * scale).ceil() as u32);
        let alloc = || Pixmap::new(pw, ph).context("pixmap alloc");
        self.pixmap = Some(alloc()?);
        self.highlight = match scene.highlight_opacity {
            Some(_) => Some(alloc()?),
            None => None,
        };
        self.dirty = true;
        Ok(())
    }

    /// Opens the app window (its own process), or brings it to the front.
    fn open_app_window(&mut self) {
        if let Some(app_window) = &self.app_window {
            app_window.send(&ToWindow::Focus);
            return;
        }
        let proxy = self.proxy.clone();
        let spawned = WindowProcess::spawn(move |message| {
            let _ = proxy.send_event(match message {
                Some(message) => UserEvent::AppWindow(message),
                None => UserEvent::AppWindowClosed,
            });
        });
        match spawned {
            Ok(app_window) => {
                self.app_window = Some(app_window);
                self.send_status();
                tracing::info!("app window opened");
            }
            Err(e) => {
                tracing::error!("app window: {e:#}");
                dialog::error(
                    "Overlay Input",
                    &format!("The app window can't open:\n{}", short_error(&e)),
                );
            }
        }
    }

    /// The window process ended, closed or crashed; an unsaved preview leaves the overlay.
    fn app_window_closed(&mut self) {
        self.app_window = None;
        self.end_preview();
        tracing::info!("app window closed");
    }

    /// Tells the app window what changed here; it re-reads the profile list on each one.
    fn send_status(&self) {
        let Some(app_window) = &self.app_window else {
            return;
        };
        app_window.send(&ToWindow::Status(Status {
            active: self.active_profile.as_deref().map(profile::display_name),
            previewing: self.previewing,
            hotkey: self.settings.toggle_hotkey.clone(),
            hotkey_registered: self.hotkey.is_some(),
            display: self.display(),
            hide_from_capture: self.state.hide_from_capture,
            window_background: self.state.window_background,
            show_window_at_start: self.settings.show_window_at_start,
        }));
    }

    /// Runs what the app window asked for and answers it.
    fn app_window_message(&mut self, event_loop: &ActiveEventLoop, message: ToOverlay) {
        let outcome = match message {
            ToOverlay::Preview(text) => self.show_preview(&text),
            ToOverlay::EndPreview => {
                self.end_preview();
                Ok(())
            }
            ToOverlay::Activate(name) => self.activate_named(&name),
            ToOverlay::SetHotkey(spec) => self.save_hotkey(&spec),
            ToOverlay::SetDisplay(display) => {
                self.set_display(event_loop, display);
                Ok(())
            }
            ToOverlay::SetWindowBackground(color) => {
                self.set_window_background(color);
                Ok(())
            }
            ToOverlay::SetHideFromCapture(hide) => {
                self.set_hide_from_capture(hide);
                Ok(())
            }
            ToOverlay::SetShowWindowAtStart(show) => {
                self.settings.show_window_at_start = show;
                let saved = self.settings.save(&self.app_dir);
                self.send_status();
                saved
            }
            ToOverlay::Quit => {
                tracing::info!("quit from the app window");
                event_loop.exit();
                Ok(())
            }
        };
        if let Some(app_window) = &self.app_window {
            app_window.send(&ToWindow::Reply(outcome.map_err(|e| format!("{e:#}"))));
        }
    }

    fn show_preview(&mut self, text: &str) -> Result<()> {
        let parsed = Profile::parse(text)?;
        let scene = Scene::build(&parsed, self.resources()?)?;
        self.install_scene(scene)?;
        self.set_previewing(true);
        Ok(())
    }

    fn activate_named(&mut self, name: &str) -> Result<()> {
        let path = self.profiles_dir.join(format!("{name}.toml"));
        anyhow::ensure!(path.is_file(), "profile {name:?} not found");
        self.activate_profile(&path)?;
        anyhow::ensure!(
            self.active_profile.as_deref() == Some(path.as_path()),
            "{name} could not be loaded; see the log"
        );
        Ok(())
    }

    fn save_hotkey(&mut self, spec: &str) -> Result<()> {
        self.set_hotkey(spec)?;
        self.settings.save(&self.app_dir)
    }

    fn set_previewing(&mut self, previewing: bool) {
        if self.previewing == previewing {
            return;
        }
        self.previewing = previewing;
        if let Some(tray) = &mut self.tray {
            tray.set_previewing(previewing);
        }
        self.send_status();
        tracing::info!(previewing, "profile preview");
    }

    /// Puts the active profile back on screen after a preview.
    fn end_preview(&mut self) {
        if !self.previewing {
            return;
        }
        self.set_previewing(false);
        // A reload failure then falls back to the built-in profile, never back to the preview.
        self.scene = None;
        let restored = match self.active_profile.clone() {
            Some(path) => self.activate_profile(&path),
            None => self.install_builtin(),
        };
        if let Err(e) = restored {
            tracing::error!("{e:#}");
        }
    }

    /// Makes `spec` the show/hide hotkey (empty = none); on failure the previous one stays registered.
    fn set_hotkey(&mut self, spec: &str) -> Result<()> {
        let spec = spec.trim();
        let new = if spec.is_empty() {
            None
        } else {
            // The parser's own message asks to file a bug; ours says what to type.
            Some(HotKey::from_str(spec).map_err(|_| {
                anyhow::anyhow!("{spec:?} is not a hotkey; use modifiers + key, e.g. Ctrl+Alt+F12")
            })?)
        };
        if self.hotkey_manager.is_none() {
            self.hotkey_manager = Some(GlobalHotKeyManager::new().context("hotkey manager")?);
        }
        let manager = self.hotkey_manager.as_ref().expect("created above");
        let old = self.hotkey.take();
        if let Some(old) = old {
            let _ = manager.unregister(old);
        }
        if let Some(hotkey) = new
            && let Err(e) = manager.register(hotkey)
        {
            if let Some(old) = old
                && manager.register(old).is_ok()
            {
                self.hotkey = Some(old);
            }
            return Err(anyhow::anyhow!("{e}"))
                .with_context(|| format!("{spec} can't be used (another app may have it)"));
        }
        self.hotkey = new;
        self.settings.toggle_hotkey = spec.to_owned();
        if let Some(tray) = &mut self.tray {
            tray.set_hotkey_hint(new.map(|_| spec));
        }
        tracing::info!(hotkey = spec, "show/hide hotkey set");
        self.send_status();
        Ok(())
    }

    /// Gamepad threads run only while not hidden and some widget uses them.
    fn sync_gamepads(&mut self) {
        let needed = !self.hidden && self.scene.as_ref().is_some_and(|s| s.uses_gamepads());
        match &self.gamepads {
            Some(threads) => threads.set_paused(!needed),
            None if needed => {
                let proxy = self.proxy.clone();
                self.gamepads = Some(GamepadThreads::spawn(move |event| {
                    let _ = proxy.send_event(UserEvent::Gamepad(event));
                }));
                tracing::info!("gamepad readers started");
            }
            None => {}
        }
    }

    fn dispatch(&mut self, event: InputEvent) {
        if !self.input.apply(&event) {
            return;
        }
        match event {
            InputEvent::MouseMove { .. } | InputEvent::Gamepad(_) => tracing::trace!(?event),
            _ => tracing::debug!(?event),
        }
        if let Some(scene) = &mut self.scene
            && scene.on_input(&event, Instant::now())
        {
            self.dirty = true;
        }
    }

    fn resources(&self) -> Result<&Resources> {
        self.resources.as_ref().context("resources not loaded")
    }

    fn refresh_profile_list(&mut self) {
        self.profiles = profile::list_profiles(&self.profiles_dir);
        let names: Vec<String> = self
            .profiles
            .iter()
            .map(|p| profile::display_name(p))
            .collect();
        let active = self.active_index();
        if let Some(tray) = &mut self.tray {
            tray.set_profiles(&names, active);
        }
        self.send_status();
    }

    fn sync_tray_selection(&self) {
        if let Some(tray) = &self.tray {
            tray.set_active_profile(self.active_index());
        }
        self.send_status();
    }

    fn active_index(&self) -> Option<usize> {
        let active = self.active_profile.as_ref()?;
        self.profiles.iter().position(|p| p == active)
    }

    fn reload_profiles(&mut self) -> Result<()> {
        self.refresh_profile_list();
        if self.previewing {
            return Ok(()); // end_preview loads the active profile fresh
        }
        match self.active_profile.clone() {
            Some(path) if path.exists() => self.activate_profile(&path),
            Some(path) => {
                tracing::warn!(?path, "active profile was removed; keeping current layout");
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// Draws the frame for wherever the overlay shows: the screen or its window.
    fn render(&mut self, now: Instant) -> Result<()> {
        let (Some(window), Some(scene), Some(pixmap)) =
            (&mut self.window, &mut self.scene, &mut self.pixmap)
        else {
            return Ok(());
        };
        if !window.is_visible() && self.color_window.is_none() {
            self.animating = false;
            self.dirty = false;
            return Ok(());
        }
        let started = Instant::now();
        scene.tick(now);
        if let Some(color_window) = &mut self.color_window {
            color_window.render(scene, &mut self.glow, now)?;
            self.frame_parts = (started.elapsed(), Duration::ZERO);
            self.animating = scene.is_animating(now);
            self.dirty = false;
            return Ok(());
        }
        pixmap.fill(Color::TRANSPARENT);
        let dpi = window.scale();
        if let Some(layer) = &mut self.highlight {
            layer.fill(Color::TRANSPARENT);
        }
        let mut canvas = Canvas {
            pixmap,
            highlight: scene.highlight_opacity.and(self.highlight.as_mut()),
            scale: dpi * scene.scale,
            glow: &mut self.glow,
        };
        scene.draw(&mut canvas, now);

        let offset = (
            (scene.offset.0 * dpi).round() as i32,
            (scene.offset.1 * dpi).round() as i32,
        );
        let highlight = self.highlight.as_ref().zip(scene.highlight_opacity);
        let drawn = Instant::now();
        window.present(pixmap, highlight, scene.anchor, offset, scene.opacity)?;
        self.frame_parts = (drawn - started, drawn.elapsed());

        self.animating = scene.is_animating(now);
        self.dirty = false;
        Ok(())
    }

    /// Counts frames that miss the monitor's refresh; logs a summary at most every 10 s.
    fn report_slow_frame(&mut self, took: Duration) {
        let (count, worst, since) = &mut self.slow_frames;
        if took > self.frame_interval {
            *count += 1;
            if took > worst.0 {
                *worst = (took, self.frame_parts.0, self.frame_parts.1);
            }
        }
        if *count == 0 || since.elapsed() < SLOW_FRAME_LOG_EVERY {
            return;
        }
        tracing::warn!(
            slow_frames = *count,
            worst_ms = worst.0.as_secs_f32() * 1000.0,
            worst_draw_ms = worst.1.as_secs_f32() * 1000.0,
            worst_present_ms = worst.2.as_secs_f32() * 1000.0,
            budget_ms = self.frame_interval.as_secs_f32() * 1000.0,
            "frames missed the refresh{}",
            if cfg!(debug_assertions) {
                " (debug build; use --release for real speed)"
            } else {
                ""
            }
        );
        self.slow_frames = (0, Default::default(), Instant::now());
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error);
        event_loop.exit();
    }
}

/// First lines of an error, short enough for a message box.
fn short_error(error: &anyhow::Error) -> String {
    let text = format!("{error:#}");
    let lines: Vec<&str> = text.lines().take(8).collect();
    lines.join("\n")
}

impl ApplicationHandler<UserEvent> for App {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if cause == StartCause::Init
            && let Err(e) = self.init(event_loop)
        {
            self.fail(event_loop, e);
        }
    }

    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.color_window {
            self.state.window_size = Some(window.size());
            self.save_state();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.color_window.as_ref().is_some_and(|w| w.id() == id) {
            match event {
                WindowEvent::CloseRequested => self.set_display(event_loop, Display::Hidden),
                WindowEvent::Resized(_) | WindowEvent::RedrawRequested => self.dirty = true,
                _ => {}
            }
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let Some(event) = input::from_device_event(&event) {
            self.dispatch(event);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        let menu_event = match event {
            UserEvent::ProfilesChanged => {
                self.reload_at = Some(Instant::now() + RELOAD_DEBOUNCE);
                return;
            }
            UserEvent::DetachInput(thread) => {
                crate::tray::detach_input(thread);
                return;
            }
            UserEvent::Hotkey(event) => {
                let ours = self.hotkey.is_some_and(|h| h.id() == event.id());
                if ours && event.state() == HotKeyState::Pressed {
                    self.toggle_hidden(event_loop);
                }
                return;
            }
            UserEvent::OpenWindow => {
                self.open_app_window();
                return;
            }
            UserEvent::Gamepad(event) => {
                // Late events from a just-paused reader must not light a hidden overlay.
                if !self.hidden {
                    self.dispatch(InputEvent::Gamepad(event));
                }
                return;
            }
            UserEvent::AppWindow(message) => {
                self.app_window_message(event_loop, message);
                return;
            }
            UserEvent::AppWindowClosed => {
                self.app_window_closed();
                return;
            }
            UserEvent::Menu(e) => e,
        };
        let Some(action) = self.tray.as_ref().and_then(|t| t.action(&menu_event)) else {
            return;
        };
        match action {
            TrayAction::SelectProfile(i) => {
                if let Some(path) = self.profiles.get(i).cloned()
                    && let Err(e) = self.activate_profile(&path)
                {
                    self.fail(event_loop, e);
                }
            }
            TrayAction::OpenWindow => self.open_app_window(),
            TrayAction::ToggleHidden => self.toggle_hidden(event_loop),
            TrayAction::Quit => {
                tracing::info!("quit from tray");
                self.quit(event_loop);
            }
        }
    }

    /// Frame pacing: at most one redraw per monitor frame; sleep when idle.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();

        if take_open_window_requested() {
            self.open_app_window();
        }

        if take_display_changed() {
            tracing::info!("display changed; re-placing overlay");
            if let Err(e) = self.fit_to_display() {
                return self.fail(event_loop, e);
            }
        }

        if self.reload_at.is_some_and(|t| now >= t) {
            self.reload_at = None;
            if let Err(e) = self.reload_profiles() {
                return self.fail(event_loop, e);
            }
        }

        if (self.dirty || self.animating) && now >= self.next_frame {
            if let Err(e) = self.render(now) {
                return self.fail(event_loop, e);
            }
            self.next_frame = now + self.frame_interval;
            self.report_slow_frame(now.elapsed());
        }

        let frame_wake = (self.dirty || self.animating).then_some(self.next_frame);
        let wake = [frame_wake, self.reload_at].into_iter().flatten().min();
        event_loop.set_control_flow(wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}
