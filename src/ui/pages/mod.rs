//! App window contents: `AppUi`, the sidebar and what ties the pages together.

mod about;
mod fields;
mod gradient;
mod home;
mod keys;
mod layout;
mod placement;
mod preview;
mod profiles;
mod settings;
#[cfg(test)]
mod tests;
mod theme;

use std::time::Instant;

use egui::{Align, Frame, Id, Layout, Margin, RichText, Ui, Vec2};
use serde_json::Value;

use super::host::{Command, HostView, Outcome, Reply};
use super::schema::Forms;
use profiles::{Naming, NamingMode};
pub use theme::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Profiles,
    Settings,
    About,
}

enum Confirm {
    /// Unsaved changes; then open this profile (or start a name prompt).
    Discard(Then),
    Delete(String),
    /// The window's close button with unsaved changes.
    Close,
}

enum Then {
    Open(String),
    Name(NamingMode),
}

pub struct AppUi {
    page: Page,
    /// A setting the overlay or Windows refused; shown on Home and Settings.
    page_error: Option<String>,
    forms: Forms,
    started: bool,
    selected: Option<String>,
    /// Working copy being edited.
    profile: Option<Value>,
    /// Last saved (or opened) version, to tell unsaved changes.
    saved: Value,
    error: Option<String>,
    notice: Option<String>,
    naming: Option<Naming>,
    confirm: Option<Confirm>,
    /// Pick the active (or first) profile on the next frame.
    reopen: bool,
    /// Show unsaved changes on the real overlay.
    on_screen: bool,
    /// Last sent on-screen preview (`None` = the saved profile is shown).
    previewed: Option<Value>,
    preview_error: Option<String>,
    /// Profile as of the last frame, and when it last changed.
    last_seen: Option<Value>,
    edited_at: Instant,
    layout: layout::LayoutState,
    hotkey: settings::HotkeyState,
    background: settings::BackgroundState,
    about: about::AboutState,
    screen: preview::PreviewState,
    commands: Vec<Command>,
    /// Close after the pending save succeeds.
    close_after_save: bool,
    close: bool,
    /// Closing quits the whole app, not just this window.
    quit: bool,
}

impl AppUi {
    pub fn new() -> Self {
        Self {
            page: Page::Home,
            page_error: None,
            forms: Forms::build(),
            started: false,
            selected: None,
            profile: None,
            saved: Value::Null,
            error: None,
            notice: None,
            naming: None,
            confirm: None,
            reopen: false,
            on_screen: true,
            previewed: None,
            preview_error: None,
            last_seen: None,
            edited_at: Instant::now(),
            layout: layout::LayoutState::default(),
            hotkey: settings::HotkeyState::default(),
            background: settings::BackgroundState::default(),
            about: about::AboutState::default(),
            screen: preview::PreviewState::default(),
            commands: Vec::new(),
            close_after_save: false,
            close: false,
            quit: false,
        }
    }

    fn dirty(&self) -> bool {
        self.profile.as_ref().is_some_and(|p| *p != self.saved)
    }

    /// The window's close button: true to close now; with unsaved changes, asks first.
    pub fn request_close(&mut self) -> bool {
        if self.dirty() {
            self.confirm = Some(Confirm::Close);
            false
        } else {
            true
        }
    }

    /// Quit Overlay Input; with unsaved changes, asks first.
    pub fn request_quit(&mut self) {
        self.quit = true;
        if self.request_close() {
            self.close = true;
        }
    }

    pub fn quitting(&self) -> bool {
        self.quit
    }

    /// True once the user chose to close (after a save or discard).
    pub fn take_close(&mut self) -> bool {
        std::mem::take(&mut self.close)
    }

    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    pub fn show(&mut self, ui: &mut Ui, host: &HostView) {
        if !self.started || self.reopen {
            self.started = true;
            self.reopen = false;
            // Keeps e.g. "Deleted ..." visible over the profile that opens instead.
            let notice = self.notice.take();
            let first = host
                .active
                .clone()
                .or_else(|| host.profiles.first().cloned());
            match first {
                Some(name) => self.load(name),
                None => self.selected = None,
            }
            self.notice = notice;
        }
        self.track_edits();

        self.nav(ui);
        match self.page {
            Page::Profiles => self.profiles_page(ui, host),
            page => {
                egui::CentralPanel::default()
                    .frame(Frame::new().fill(BG).inner_margin(Margin::same(16)))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_max_width(ui.available_width().min(760.0));
                                if let Some(error) = &self.page_error {
                                    ui.label(RichText::new(format!("● {error}")).color(DANGER));
                                    ui.add_space(8.0);
                                }
                                match page {
                                    Page::Home => self.home(ui, host),
                                    Page::Settings => self.settings(ui, host),
                                    _ => self.about(ui, host),
                                }
                            });
                    });
            }
        }

        self.name_dialog(ui.ctx(), host);
        self.dialogs(ui.ctx());
        self.shortcuts(ui.ctx());
        self.sync_preview(ui.ctx());
    }

    fn nav(&mut self, ui: &mut Ui) {
        egui::Panel::left("nav")
            .resizable(false)
            .exact_size(176.0)
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .inner_margin(Margin::symmetric(12, 16)),
            )
            .show(ui, |ui| {
                ui.label(RichText::new("Overlay Input").size(17.0).strong());
                ui.add_space(16.0);
                ui.with_layout(Layout::top_down_justified(Align::LEFT), |ui| {
                    ui.spacing_mut().button_padding = Vec2::new(10.0, 7.0);
                    let dirty = self.dirty();
                    for (page, label) in [
                        (Page::Home, "Home"),
                        (
                            Page::Profiles,
                            if dirty { "Profiles ●" } else { "Profiles" },
                        ),
                        (Page::Settings, "Settings"),
                        (Page::About, "About"),
                    ] {
                        let selected = self.page == page;
                        if ui.add(egui::Button::selectable(selected, label)).clicked() {
                            self.page = page;
                            self.page_error = None;
                        }
                    }
                });
            });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(confirm) = &self.confirm else {
            return;
        };
        let (title, text, yes, alt) = match confirm {
            Confirm::Discard(_) => (
                "Unsaved changes",
                format!(
                    "Discard unsaved changes to \"{}\"?",
                    self.selected.as_deref().unwrap_or_default()
                ),
                "Discard",
                None,
            ),
            Confirm::Delete(name) => (
                "Delete profile",
                format!("Delete \"{name}\"? This removes {name}.toml from the profiles folder."),
                "Delete",
                None,
            ),
            Confirm::Close => (
                "Unsaved changes",
                format!(
                    "\"{}\" has unsaved changes.",
                    self.selected.as_deref().unwrap_or_default()
                ),
                "Discard and close",
                Some("Save and close"),
            ),
        };
        let mut choice = None;
        let modal = egui::Modal::new(Id::unique("confirm")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(title).strong().size(16.0));
            ui.add_space(6.0);
            ui.label(text);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if let Some(alt) = alt
                    && ui.button(RichText::new(alt).strong()).clicked()
                {
                    choice = Some(2);
                }
                let danger = egui::Button::new(RichText::new(yes).color(DANGER));
                if ui.add(danger).clicked() {
                    choice = Some(1);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(0);
                }
            });
        });
        if modal.should_close() && choice.is_none() {
            choice = Some(0);
        }
        let Some(choice) = choice else {
            return;
        };
        let confirm = self.confirm.take().expect("checked above");
        match (confirm, choice) {
            (Confirm::Close, 0) => self.quit = false,
            (_, 0) => {}
            (Confirm::Discard(then), _) => {
                self.discard();
                match then {
                    Then::Open(name) => self.load(name),
                    Then::Name(mode) => self.start_naming(mode),
                }
            }
            (Confirm::Delete(name), _) => self.commands.push(Command::Delete(name)),
            (Confirm::Close, 1) => {
                self.discard();
                self.close = true;
            }
            (Confirm::Close, _) => {
                self.close_after_save = true;
                self.save();
            }
        }
    }

    /// Applies what the overlay answered to `command`.
    pub fn on_result(&mut self, command: Command, outcome: Outcome, host_active: Option<&str>) {
        match (command, outcome) {
            (Command::Open(name), Ok(Reply::Profile(profile))) => {
                if self.selected.as_deref() == Some(name.as_str()) {
                    self.saved = profile.clone();
                    self.profile = Some(profile);
                    self.last_seen = self.profile.clone();
                }
            }
            (Command::Open(name), Err(e)) => {
                if self.selected.as_deref() == Some(name.as_str()) {
                    self.error = Some(e);
                }
            }
            (Command::Save { name, profile }, Ok(_)) => {
                self.saved = profile;
                self.previewed = None;
                self.notice = Some(if host_active == Some(name.as_str()) {
                    "Saved — the overlay updated.".into()
                } else {
                    "Saved.".into()
                });
                if std::mem::take(&mut self.close_after_save) {
                    self.close = true;
                }
            }
            (Command::Save { .. }, Err(e)) => {
                self.error = Some(e);
                if std::mem::take(&mut self.close_after_save) {
                    self.quit = false;
                }
            }
            (Command::Preview(_), result) => {
                self.preview_error = result.err();
            }
            (Command::Render { .. }, result) => self.screen.on_result(result),
            (Command::Activate(name), Ok(_)) => {
                self.page_error = None;
                self.notice = Some(format!("\"{name}\" is now on the overlay."));
            }
            (Command::Activate(_), Err(e)) => {
                self.page_error = Some(e.clone());
                self.error = Some(e);
            }
            (Command::Create(to), result) => self.named(to, result, "Created"),
            (Command::Duplicate { to, .. }, result) => self.named(to, result, "Copied to"),
            (Command::Rename { to, .. }, result) => self.named(to, result, "Renamed to"),
            (Command::Delete(name), Ok(_)) => {
                self.profile = None;
                self.selected = None;
                self.reopen = true;
                self.notice = Some(format!("Deleted \"{name}\"."));
            }
            (Command::SetHotkey(spec), result) => self.hotkey.on_result(&spec, result),
            (
                Command::SetDisplay(_)
                | Command::SetWindowBackground(_)
                | Command::SetHideFromCapture(_)
                | Command::SetShowWindowAtStart(_)
                | Command::SetStartWithWindows(_)
                | Command::OpenProfilesFolder
                | Command::OpenLogsFolder,
                result,
            ) => self.page_error = result.err(),
            (_, Err(e)) => self.error = Some(e),
            (_, Ok(_)) => {}
        }
    }
}
