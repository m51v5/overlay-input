//! About page: what this is, where the project lives, how to get help, and who made it.

use egui::{
    Align2, Color32, ColorImage, CursorIcon, FontId, Image, OpenUrl, RichText, Sense,
    TextureHandle, TextureOptions, Ui, vec2,
};

use super::{AppUi, BLUE, MUTED, PANEL_2, TEXT, card, muted};
use crate::ui::host::{Command, HostView};

const REPOSITORY: &str = "https://github.com/m51v5/overlay-input";
const WEBSITE: &str = "https://m51v5.com";
const CONTACT: &str = "https://m51v5.com/contact";
const TELEGRAM: &str = "https://t.me/m51v5";
const HANDLE: &str = "@m51v5";
const AUTHOR: &str = "Mustafa Jassem";

const ICON_FILE: &[u8] = include_bytes!("../../../assets/windows/app.ico");
const ICON_SIZE: f32 = 88.0;
const ROW_HEIGHT: f32 = 36.0;

/// The icon as a texture, made when the page first opens.
#[derive(Default)]
pub struct AboutState {
    icon: Option<TextureHandle>,
}

impl AppUi {
    pub(super) fn about(&mut self, ui: &mut Ui, host: &HostView) {
        let icon = self
            .about
            .icon
            .get_or_insert_with(|| load_icon(ui.ctx()))
            .clone();
        ui.vertical_centered(|ui| {
            ui.add_space(8.0);
            ui.add(Image::new(&icon).fit_to_exact_size(vec2(ICON_SIZE, ICON_SIZE)));
            ui.add_space(8.0);
            ui.label(RichText::new("Overlay Input").size(20.0).strong());
            ui.label(muted(format!("Version {}", host.version)));
            ui.add_space(8.0);
            ui.label("Shows your keyboard, mouse and gamepad input on top of any app or game.");
            ui.add_space(16.0);
        });

        card(ui, "Project", |ui| {
            link_row(ui, "What's new", None, &format!("{REPOSITORY}/releases"));
            link_row(ui, "Source code", None, REPOSITORY);
            link_row(
                ui,
                "Privacy",
                None,
                &format!("{REPOSITORY}/blob/main/SECURITY.md"),
            );
            link_row(
                ui,
                "License",
                Some("MIT"),
                &format!("{REPOSITORY}/blob/main/LICENSE"),
            );
        });
        ui.add_space(8.0);
        card(ui, "Help", |ui| {
            link_row(ui, "Report a problem", None, &report_link(host.version));
            if row(
                ui,
                "Open logs folder",
                None,
                "Log files are here if you report a problem",
            ) {
                self.commands.push(Command::OpenLogsFolder);
            }
        });
        ui.add_space(8.0);
        card(ui, "Made by", |ui| {
            link_row(ui, AUTHOR, Some(HANDLE), WEBSITE);
            link_row(ui, "Telegram", Some(HANDLE), TELEGRAM);
            link_row(ui, "Other platforms", None, CONTACT);
        });
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            ui.label(muted(format!("© 2026 {HANDLE}. MIT License.")).small());
        });
        ui.add_space(8.0);
    }
}

/// The bug form with the version already filled in.
fn report_link(version: &str) -> String {
    format!("{REPOSITORY}/issues/new?template=bug_report.yml&version={version}")
}

fn link_row(ui: &mut Ui, label: &str, value: Option<&str>, url: &str) {
    if row(ui, label, value, url) {
        ui.ctx().open_url(OpenUrl::new_tab(url));
    }
}

/// A full-width clickable row; true when clicked. `hint` shows on hover.
fn row(ui: &mut Ui, label: &str, value: Option<&str>, hint: &str) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 6, PANEL_2);
    }
    let font = FontId::proportional(14.0);
    let left = rect.left_center() + vec2(8.0, 0.0);
    let right = rect.right_center() - vec2(10.0, 0.0);
    let painter = ui.painter();
    painter.text(left, Align2::LEFT_CENTER, label, font.clone(), TEXT);
    painter.text(right, Align2::RIGHT_CENTER, "›", font.clone(), MUTED);
    if let Some(value) = value {
        painter.text(
            right - vec2(18.0, 0.0),
            Align2::RIGHT_CENTER,
            value,
            font,
            BLUE,
        );
    }
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(hint)
        .clicked()
}

fn load_icon(ctx: &egui::Context) -> TextureHandle {
    let image = png_in_ico(ICON_FILE)
        .and_then(|png| eframe::icon_data::from_png_bytes(png).ok())
        .map(|icon| {
            let size = [icon.width as usize, icon.height as usize];
            ColorImage::from_rgba_unmultiplied(size, &icon.rgba)
        })
        // Without it the page still works.
        .unwrap_or_else(|| ColorImage::filled([1, 1], Color32::TRANSPARENT));
    ctx.load_texture("app-icon", image, TextureOptions::LINEAR)
}

/// The PNG stored inside an `.ico` file, if it has one.
fn png_in_ico(file: &[u8]) -> Option<&[u8]> {
    const SIGNATURE: &[u8] = b"\x89PNG";
    let count = u16::from_le_bytes([*file.get(4)?, *file.get(5)?]) as usize;
    (0..count).find_map(|i| {
        let entry = file.get(6 + i * 16..6 + (i + 1) * 16)?;
        let size = u32::from_le_bytes(entry[8..12].try_into().ok()?) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().ok()?) as usize;
        file.get(offset..offset + size)
            .filter(|data| data.starts_with(SIGNATURE))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_icon_decodes() {
        let icon = eframe::icon_data::from_png_bytes(png_in_ico(ICON_FILE).unwrap()).unwrap();
        assert_eq!((icon.width, icon.height), (256, 256));
    }

    #[test]
    fn a_broken_ico_has_no_png() {
        assert!(png_in_ico(b"").is_none());
        assert!(png_in_ico(&[0, 0, 1, 0, 9, 0]).is_none(), "entries cut off");
    }

    #[test]
    fn the_report_link_carries_the_version() {
        let link = report_link("1.2.3");
        assert!(
            link.ends_with("/issues/new?template=bug_report.yml&version=1.2.3"),
            "{link}"
        );
    }
}
