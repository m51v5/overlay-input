//! Home page: where the overlay shows and which profile it uses.

use egui::{Align, Color32, CursorIcon, Frame, Layout, Margin, RichText, Sense, Stroke, Ui};

use super::{AppUi, BLUE, BORDER, PANEL_2, Page, card, muted};
use crate::settings::Display;
use crate::ui::host::{Command, HostView};

const CHOICES: [(Display, &str, &str); 3] = [
    (
        Display::Screen,
        "On screen",
        "Over everything, and click-through.",
    ),
    (
        Display::Window,
        "In a window",
        "On a solid color, to capture in OBS.",
    ),
    (
        Display::Hidden,
        "Hidden",
        "Nothing shows; input isn't read.",
    ),
];

impl AppUi {
    pub(super) fn home(&mut self, ui: &mut Ui, host: &HostView) {
        card(ui, "Overlay", |ui| {
            ui.columns(CHOICES.len(), |columns| {
                for (column, (display, title, text)) in columns.iter_mut().zip(CHOICES) {
                    if choice(column, host.display == display, title, text) {
                        self.commands.push(Command::SetDisplay(display));
                    }
                }
            });
            if !host.hotkey.is_empty() && host.hotkey_registered {
                ui.add_space(4.0);
                ui.label(muted(format!(
                    "{} hides it and brings it back.",
                    host.hotkey
                )));
            }
            if host.display == Display::Window {
                ui.add_space(8.0);
                ui.separator();
                self.background_row(ui, host);
                ui.label(muted(
                    "In OBS: add Window Capture of \"Overlay Input — Window\", \
                     then a Color Key filter with this color.",
                ));
            }
        });
        ui.add_space(8.0);
        card(ui, "Profile", |ui| {
            let dirty = self.dirty();
            let active = host.active.as_deref().unwrap_or("Built-in default");
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!dirty, |ui| {
                    egui::ComboBox::from_id_salt("active_profile")
                        .selected_text(active)
                        .width(260.0)
                        .show_ui(ui, |ui| {
                            for name in &host.profiles {
                                let on = host.active.as_deref() == Some(name.as_str());
                                if ui.selectable_label(on, name).clicked() && !on {
                                    self.commands.push(Command::Activate(name.clone()));
                                }
                            }
                        });
                });
                if ui.button("Edit profiles").clicked() {
                    self.page = Page::Profiles;
                }
            });
            if dirty {
                ui.label(muted(format!(
                    "\"{}\" has unsaved changes; save or discard them to switch.",
                    self.selected.as_deref().unwrap_or_default()
                )));
            }
        });
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.button("Quit Overlay Input").clicked() {
                self.request_quit();
            }
            ui.label(muted("Closing this window leaves the overlay running."));
        });
    }
}

/// A selectable tile; true when clicked.
fn choice(ui: &mut Ui, selected: bool, title: &str, text: &str) -> bool {
    let (fill, stroke) = if selected {
        (BLUE.gamma_multiply(0.18), Stroke::new(1.5, BLUE))
    } else {
        (PANEL_2, Stroke::new(1.0, BORDER))
    };
    let response = Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(8)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(64.0);
            // Columns are justified, which would stretch the wrapped text.
            ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                let heading = RichText::new(title).strong().size(15.0);
                ui.add(
                    egui::Label::new(if selected {
                        heading.color(Color32::WHITE)
                    } else {
                        heading
                    })
                    .selectable(false),
                );
                ui.add(egui::Label::new(muted(text)).selectable(false));
            });
        })
        .response
        .interact(Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    response.clicked() && !selected
}
