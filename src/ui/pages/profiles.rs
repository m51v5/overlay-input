//! Profiles page: the list, the form, previews, the save bar and the name prompt.

use std::time::{Duration, Instant};

use egui::{Align, Frame, Id, Layout, Margin, RichText, Stroke, Ui};
use serde_json::Value;

use super::{
    AppUi, BG, BLUE, BORDER, Confirm, DANGER, GREEN, GREEN_LIGHT, PANEL, PANEL_2, TEXT, Then, card,
    layout, muted, placement,
};
use crate::ui::host::{Command, HostView, Outcome, name_problem};

/// The form needs this much width; the screen preview only shows if it leaves that much.
const FORM_MIN_WIDTH: f32 = 540.0;
const SCREEN_DEFAULT_WIDTH: f32 = 340.0;
const SCREEN_MIN_WIDTH: f32 = 300.0;

/// Edits settle this long before they go to the real overlay.
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum NamingMode {
    New,
    Duplicate,
    Rename,
}

pub(super) struct Naming {
    pub(super) mode: NamingMode,
    pub(super) text: String,
    pub(super) error: Option<String>,
    pub(super) focus: bool,
}

impl AppUi {
    pub(super) fn profiles_page(&mut self, ui: &mut Ui, host: &HostView) {
        if self.profile.is_some() {
            egui::Panel::bottom("savebar")
                .frame(
                    Frame::new()
                        .fill(PANEL)
                        .stroke(Stroke::new(1.0, if self.dirty() { BLUE } else { BORDER }))
                        .inner_margin(Margin::symmetric(16, 10)),
                )
                .show(ui, |ui| self.save_bar(ui, host));
        }

        egui::Panel::left("side")
            .resizable(false)
            .exact_size(200.0)
            .frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(12, 8)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.profile_list(ui, host));
            });

        let room = ui.available_width();
        if let Some(profile) = &self.profile
            && room >= FORM_MIN_WIDTH + SCREEN_DEFAULT_WIDTH
        {
            egui::Panel::right("screen")
                .resizable(true)
                .default_size(SCREEN_DEFAULT_WIDTH)
                .min_size(SCREEN_MIN_WIDTH)
                .max_size(room - FORM_MIN_WIDTH)
                .frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(16, 8)))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if let Some(cmd) = self.screen.show(ui, profile, &self.forms, host) {
                            self.commands.push(cmd);
                        }
                    });
                });
        }

        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(8, 8)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.main(ui, host));
            });
    }

    pub(super) fn profile_list(&mut self, ui: &mut Ui, host: &HostView) {
        card(ui, "Profiles", |ui| {
            if host.profiles.is_empty() {
                ui.label(muted("No profiles yet."));
            }
            for name in &host.profiles {
                let selected = self.selected.as_deref() == Some(name.as_str());
                let active = host.active.as_deref() == Some(name.as_str());
                let mut text = RichText::new(name);
                if selected {
                    text = text.strong();
                }
                let row = ui
                    .horizontal(|ui| {
                        let r = ui
                            .add_sized(
                                [ui.available_width() - 20.0, 24.0],
                                egui::Button::selectable(selected, text).truncate(),
                            )
                            .on_hover_text(name);
                        if active {
                            ui.label(RichText::new("●").color(GREEN_LIGHT))
                                .on_hover_text("On the overlay");
                        }
                        r
                    })
                    .inner;
                if row.clicked() && !selected {
                    self.open(name.clone());
                }
            }
            ui.add_space(6.0);
            if ui.button("+ New profile").clicked() {
                self.start_naming(NamingMode::New);
            }
            if ui.button("Open folder").clicked() {
                self.commands.push(Command::OpenProfilesFolder);
            }
        });
    }

    pub(super) fn main(&mut self, ui: &mut Ui, host: &HostView) {
        ui.set_max_width(ui.available_width().min(900.0));
        if let Some(name) = self.selected.clone() {
            self.title_bar(ui, host, &name);
            ui.add_space(8.0);
        } else {
            card(ui, "", |ui| {
                ui.label(muted("Pick a profile on the left, or create one."));
                if ui.button("+ New profile").clicked() {
                    self.start_naming(NamingMode::New);
                }
            });
        }

        match &mut self.profile {
            Some(profile) => {
                placement::show(ui, profile, &self.forms, host);
                ui.add_space(8.0);
                self.layout.show(ui, profile, &self.forms);
                ui.add_space(24.0);
            }
            None => match (&self.selected, &self.error) {
                (Some(_), Some(error)) => {
                    card(ui, "", |ui| {
                        ui.label(
                            RichText::new(format!("● {error}"))
                                .color(DANGER)
                                .monospace(),
                        );
                        ui.label(muted(
                            "Fix the file in the profiles folder, then pick the profile again.",
                        ));
                    });
                }
                (Some(_), None) => {
                    ui.label(muted("Loading…"));
                }
                _ => {}
            },
        }
    }

    pub(super) fn title_bar(&mut self, ui: &mut Ui, host: &HostView, name: &str) {
        let dirty = self.dirty();
        ui.horizontal(|ui| {
            // Long names give way to the buttons on this row.
            let name_width = (ui.available_width() - 420.0).max(80.0);
            ui.scope(|ui| {
                ui.set_max_width(name_width);
                ui.add(egui::Label::new(RichText::new(name).heading()).truncate());
            });
            let active = host.active.as_deref() == Some(name);
            if active {
                Frame::new()
                    .stroke(Stroke::new(1.0, GREEN))
                    .corner_radius(10)
                    .inner_margin(Margin::symmetric(8, 1))
                    .show(ui, |ui| {
                        ui.label(RichText::new("on the overlay").small().color(GREEN_LIGHT))
                    });
            } else if ui
                .add_enabled(!dirty, egui::Button::new("Show on overlay"))
                .on_disabled_hover_text("Save first")
                .clicked()
            {
                self.commands.push(Command::Activate(name.to_owned()));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let delete = ui
                    .add_enabled(!dirty && !active, egui::Button::new("Delete"))
                    .on_disabled_hover_text(if active {
                        "Show another profile on the overlay first"
                    } else {
                        "Save or discard first"
                    });
                if delete.clicked() {
                    self.confirm = Some(Confirm::Delete(name.to_owned()));
                }
                if ui
                    .add_enabled(!dirty, egui::Button::new("Rename"))
                    .on_disabled_hover_text("Save or discard first")
                    .clicked()
                {
                    self.start_naming(NamingMode::Rename);
                }
                if ui
                    .add_enabled(!dirty, egui::Button::new("Duplicate"))
                    .on_disabled_hover_text("Save or discard first")
                    .clicked()
                {
                    self.start_naming(NamingMode::Duplicate);
                }
            });
        });
    }

    /// Name prompt for New / Duplicate / Rename, as a dialog so it's never scrolled out of view.
    pub(super) fn name_dialog(&mut self, ctx: &egui::Context, host: &HostView) {
        let selected = self.selected.clone().unwrap_or_default();
        let Some(naming) = &mut self.naming else {
            return;
        };
        let (title, action) = match naming.mode {
            NamingMode::New => ("New profile".to_owned(), "Create"),
            NamingMode::Duplicate => (format!("Duplicate \"{selected}\" as"), "Duplicate"),
            NamingMode::Rename => (format!("Rename \"{selected}\" to"), "Rename"),
        };
        let current = (naming.mode == NamingMode::Rename).then_some(selected.as_str());
        let name = naming.text.as_str();
        let problem = name_problem(name).map(str::to_owned).or_else(|| {
            if Some(name) == current {
                return Some("That is the current name.".into());
            }
            let lower = name.to_lowercase();
            host.profiles
                .iter()
                .any(|t| {
                    t.to_lowercase() == lower
                        && Some(t.to_lowercase()) != current.map(str::to_lowercase)
                })
                .then(|| "A profile with that name already exists.".into())
        });

        let mut submit = false;
        let mut cancel = false;
        let modal = egui::Modal::new(Id::unique("name")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.label(RichText::new(&title).strong().size(16.0));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut naming.text)
                        .char_limit(64)
                        .desired_width(280.0),
                );
                if naming.focus {
                    naming.focus = false;
                    edit.request_focus();
                }
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                cancel |= ui.input(|i| i.key_pressed(egui::Key::Escape));
                submit |= enter && problem.is_none();
                submit |= ui
                    .add_enabled(problem.is_none(), egui::Button::new(action))
                    .clicked();
                cancel |= ui.button("Cancel").clicked();
            });
            if let Some(error) = &naming.error {
                ui.label(RichText::new(format!("● {error}")).color(DANGER));
            } else if let Some(problem) = &problem
                && !naming.text.is_empty()
            {
                ui.label(muted(problem.as_str()));
            }
        });
        cancel |= modal.should_close() && !submit;

        if cancel {
            self.naming = None;
        } else if submit {
            let to = naming.text.clone();
            naming.error = None;
            self.commands.push(match naming.mode {
                NamingMode::New => Command::Create(to),
                NamingMode::Duplicate => Command::Duplicate { from: selected, to },
                NamingMode::Rename => Command::Rename { from: selected, to },
            });
        }
    }

    pub(super) fn save_bar(&mut self, ui: &mut Ui, host: &HostView) {
        let dirty = self.dirty();
        ui.horizontal(|ui| {
            let status = if let Some(error) = &self.error {
                RichText::new(format!("● {error}")).color(DANGER)
            } else if dirty {
                let mut text = "● Unsaved changes".to_owned();
                if self.on_screen && host.previewing {
                    text.push_str(" — previewing on the overlay");
                }
                RichText::new(text).color(TEXT)
            } else if let Some(notice) = &self.notice {
                RichText::new(format!("● {notice}")).color(GREEN_LIGHT)
            } else {
                muted("All changes saved")
            };
            ui.add(egui::Label::new(status).truncate());
            if dirty && let Some(e) = &self.preview_error {
                ui.label(RichText::new(format!("· preview: {e}")).color(DANGER))
                    .on_hover_text(e);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let save = egui::Button::new(RichText::new("Save").strong()).fill(if dirty {
                    BLUE.gamma_multiply(0.6)
                } else {
                    PANEL_2
                });
                if ui
                    .add_enabled(dirty, save)
                    .on_hover_text("Ctrl+S")
                    .clicked()
                {
                    self.save();
                }
                if ui
                    .add_enabled(dirty, egui::Button::new("Discard"))
                    .clicked()
                {
                    self.discard();
                }
                ui.separator();
                ui.checkbox(&mut self.on_screen, "Preview on screen")
                    .on_hover_text("Show unsaved changes on the real overlay while you edit");
            });
        });
    }

    pub(super) fn shortcuts(&mut self, ctx: &egui::Context) {
        let save = ctx.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::S,
            ))
        });
        if save && self.dirty() && self.confirm.is_none() {
            self.save();
        }
    }

    /// Notes when the working copy last changed, for the debounced previews.
    pub(super) fn track_edits(&mut self) {
        if self.profile != self.last_seen {
            self.last_seen = self.profile.clone();
            self.edited_at = Instant::now();
            if self.profile.is_some() && self.dirty() {
                self.notice = None;
            }
        }
    }

    /// Sends unsaved changes to the real overlay once edits settle; ends the preview when they're gone.
    pub(super) fn sync_preview(&mut self, ctx: &egui::Context) {
        // A save queued this frame ends any preview itself.
        if self
            .commands
            .iter()
            .any(|c| matches!(c, Command::Save { .. }))
        {
            return;
        }
        let want = (self.on_screen && self.dirty())
            .then(|| self.profile.clone())
            .flatten();
        if want == self.previewed {
            return;
        }
        match want {
            None => {
                self.commands.push(Command::EndPreview);
                self.previewed = None;
                self.preview_error = None;
            }
            Some(profile) => {
                let waited = self.edited_at.elapsed();
                if waited >= PREVIEW_DEBOUNCE {
                    self.commands.push(Command::Preview(profile.clone()));
                    self.previewed = Some(profile);
                } else {
                    ctx.request_repaint_after(PREVIEW_DEBOUNCE - waited);
                }
            }
        }
    }

    pub(super) fn open(&mut self, name: String) {
        if self.dirty() {
            self.confirm = Some(Confirm::Discard(Then::Open(name)));
        } else {
            self.load(name);
        }
    }

    pub(super) fn start_naming(&mut self, mode: NamingMode) {
        if mode == NamingMode::New && self.dirty() {
            self.confirm = Some(Confirm::Discard(Then::Name(mode)));
            return;
        }
        let selected = self.selected.clone().unwrap_or_default();
        self.naming = Some(Naming {
            mode,
            text: match mode {
                NamingMode::New => String::new(),
                NamingMode::Duplicate => format!("{selected} copy"),
                NamingMode::Rename => selected,
            },
            error: None,
            focus: true,
        });
    }

    pub(super) fn load(&mut self, name: String) {
        self.selected = Some(name.clone());
        self.profile = None;
        self.saved = Value::Null;
        self.error = None;
        self.notice = None;
        self.preview_error = None;
        self.layout = layout::LayoutState::default();
        self.commands.push(Command::Open(name));
    }

    pub(super) fn save(&mut self) {
        if let (Some(name), Some(profile)) = (&self.selected, &self.profile) {
            self.error = None;
            self.commands.push(Command::Save {
                name: name.clone(),
                profile: profile.clone(),
            });
        }
    }

    pub(super) fn discard(&mut self) {
        if self.profile.is_some() {
            self.profile = Some(self.saved.clone());
            self.error = None;
            self.notice = Some("Changes discarded.".into());
        }
    }

    pub(super) fn named(&mut self, to: String, result: Outcome, verb: &str) {
        match result {
            Ok(_) => {
                self.naming = None;
                self.load(to.clone());
                self.notice = Some(format!("{verb} \"{to}\"."));
            }
            Err(e) => match &mut self.naming {
                Some(naming) => naming.error = Some(e),
                None => self.error = Some(e),
            },
        }
    }
}
