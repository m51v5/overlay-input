//! Settings page: hotkey (record new keys, remove, or reset), overlay window, start options.

use std::str::FromStr;
use std::time::{Duration, Instant};

use egui::{Event, Frame, Key, Margin, RichText, Stroke, Ui};
use global_hotkey::hotkey::HotKey;

use super::{AppUi, BLUE, BORDER, DANGER, GREEN_LIGHT, PANEL_2, card, muted};
use crate::settings::{Display, State};
use crate::ui::host::{Command, HostView, Outcome};

/// While dragging in the color picker, the overlay hears about it this often.
const BACKGROUND_INTERVAL: Duration = Duration::from_millis(50);

impl AppUi {
    pub(super) fn settings(&mut self, ui: &mut Ui, host: &HostView) {
        card(ui, "Show/hide hotkey", |ui| {
            if let Some(command) = self.hotkey.show(ui, host) {
                self.commands.push(command);
            }
        });
        ui.add_space(8.0);
        card(ui, "Overlay", |ui| {
            let mut hide = host.hide_from_capture;
            let on_screen = host.display == Display::Screen;
            let toggled = ui
                .add_enabled(
                    on_screen,
                    egui::Checkbox::new(&mut hide, "Hide from recordings"),
                )
                .on_hover_text("You see the overlay; screenshots and screen recordings don't")
                .on_disabled_hover_text("Only for the overlay on screen")
                .changed();
            if toggled {
                self.commands.push(Command::SetHideFromCapture(hide));
            }
            ui.add_space(4.0);
            self.background_row(ui, host);
            ui.label(muted(
                "Behind the overlay in its window. Magenta suits a Color Key, as the overlay itself \
                 is green and blue. Black needs no key: use Screen blending in OBS.",
            ));
        });
        ui.add_space(8.0);
        card(ui, "Start", |ui| {
            let mut at_start = host.show_window_at_start;
            if ui
                .checkbox(&mut at_start, "Open this window when Overlay Input starts")
                .changed()
            {
                self.commands.push(Command::SetShowWindowAtStart(at_start));
            }
            let mut with_windows = host.start_with_windows;
            if ui
                .checkbox(&mut with_windows, "Start with Windows")
                .changed()
            {
                self.commands
                    .push(Command::SetStartWithWindows(with_windows));
            }
        });
    }

    /// Color of the overlay's window; applies while the picker is open.
    pub(super) fn background_row(&mut self, ui: &mut Ui, host: &HostView) {
        let mut color = self.background.editing.unwrap_or(host.window_background);
        let default = State::default().window_background;
        ui.horizontal(|ui| {
            if egui::color_picker::color_edit_button_srgb(ui, &mut color).changed() {
                self.background.editing = Some(color);
            }
            ui.label("Window background");
            if color != default && ui.button("Reset to magenta").clicked() {
                self.background.editing = Some(default);
            }
        });
        let Some(color) = self.background.editing else {
            return;
        };
        if color == host.window_background {
            self.background.editing = None;
            return;
        }
        if self.background.sent_at.elapsed() >= BACKGROUND_INTERVAL {
            self.background.sent_at = Instant::now();
            self.commands.push(Command::SetWindowBackground(color));
        }
        ui.ctx().request_repaint_after(BACKGROUND_INTERVAL);
    }
}

/// The color being picked, until the overlay has it.
pub struct BackgroundState {
    editing: Option<[u8; 3]>,
    sent_at: Instant,
}

impl Default for BackgroundState {
    fn default() -> Self {
        Self {
            editing: None,
            sent_at: Instant::now(),
        }
    }
}

#[derive(Default)]
pub struct HotkeyState {
    capturing: bool,
    /// Recorded keys waiting for "Use it".
    pending: Option<String>,
    message: Option<(bool, String)>,
}

impl HotkeyState {
    pub fn on_result(&mut self, spec: &str, outcome: Outcome) {
        self.message = Some(match outcome {
            Ok(_) if spec.is_empty() => (true, "Hotkey removed.".into()),
            Ok(_) => (true, format!("Hotkey is now {spec}.")),
            Err(e) => (false, e),
        });
        if self.message.as_ref().is_some_and(|(ok, _)| *ok) {
            self.pending = None;
        }
    }

    pub fn show(&mut self, ui: &mut Ui, host: &HostView) -> Option<Command> {
        let mut command = None;
        if self.capturing {
            Frame::new()
                .fill(PANEL_2)
                .stroke(Stroke::new(1.0, BLUE))
                .corner_radius(6)
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.label("Press the new keys…");
                    ui.label(muted("Esc cancels").small());
                });
            self.capture(ui);
        } else if let Some(pending) = self.pending.clone() {
            ui.horizontal_wrapped(|ui| {
                kbd(ui, &pending);
                if ui.button(RichText::new("Use it").strong()).clicked() {
                    command = Some(Command::SetHotkey(pending));
                }
                if ui.button("Cancel").clicked() {
                    self.pending = None;
                }
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                if host.hotkey.is_empty() {
                    ui.label(muted("none"));
                } else {
                    kbd(ui, &host.hotkey);
                }
                if ui.button("Change").clicked() {
                    self.capturing = true;
                    self.message = None;
                }
                if !host.hotkey.is_empty()
                    && ui
                        .button("Remove")
                        .on_hover_text("No hotkey; show and hide from here or the tray")
                        .clicked()
                {
                    command = Some(Command::SetHotkey(String::new()));
                }
                if host.hotkey != host.default_hotkey
                    && ui
                        .button("Reset to default")
                        .on_hover_text(format!("Back to {}", host.default_hotkey))
                        .clicked()
                {
                    command = Some(Command::SetHotkey(host.default_hotkey.to_owned()));
                }
            });
            if !host.hotkey.is_empty() && !host.hotkey_registered {
                ui.label(
                    RichText::new("Not active: another app may use it. Pick another.")
                        .color(DANGER),
                );
            }
        }
        if let Some((ok, text)) = &self.message {
            ui.label(RichText::new(text).color(if *ok { GREEN_LIGHT } else { DANGER }));
        }
        command
    }

    fn capture(&mut self, ui: &mut Ui) {
        let events = ui.input(|i| i.events.clone());
        for event in events {
            let Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            if key == Key::Escape && !modifiers.any() {
                self.capturing = false;
                return;
            }
            let mut parts: Vec<&str> = Vec::new();
            if modifiers.ctrl {
                parts.push("Ctrl");
            }
            if modifiers.alt {
                parts.push("Alt");
            }
            if modifiers.shift {
                parts.push("Shift");
            }
            let name = key.name();
            let f_key = name.starts_with('F') && name[1..].parse::<u8>().is_ok();
            // A bare letter or digit would stop that key from typing anywhere.
            if parts.is_empty() && !f_key {
                self.message = Some((
                    false,
                    "Add Ctrl, Alt or Shift, or use an F-key alone.".into(),
                ));
                continue;
            }
            parts.push(name);
            let spec = parts.join("+");
            if HotKey::from_str(&spec).is_err() {
                self.message = Some((false, format!("{spec} can't be used as a hotkey.")));
                continue;
            }
            self.pending = Some(spec);
            self.capturing = false;
            self.message = None;
            return;
        }
        // Clicking elsewhere ends recording.
        if ui.input(|i| i.pointer.any_pressed()) {
            self.capturing = false;
        }
    }
}

fn kbd(ui: &mut Ui, text: &str) {
    Frame::new()
        .fill(PANEL_2)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(4)
        .inner_margin(Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(RichText::new(text).monospace()));
}
