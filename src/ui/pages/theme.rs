//! Colors, egui style and the small helpers every page uses.

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Frame, Margin, RichText, Stroke, TextStyle, Theme,
    Ui, Vec2,
};

pub const BG: Color32 = Color32::from_rgb(0x12, 0x14, 0x17);
pub const PANEL: Color32 = Color32::from_rgb(0x1b, 0x1e, 0x23);
pub const PANEL_2: Color32 = Color32::from_rgb(0x23, 0x27, 0x2e);
pub const BORDER: Color32 = Color32::from_rgb(0x30, 0x35, 0x3d);
pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xea, 0xed);
pub const MUTED: Color32 = Color32::from_rgb(0x9a, 0xa0, 0xa6);
pub const GREEN: Color32 = Color32::from_rgb(0x2d, 0xa0, 0x26);
pub const GREEN_LIGHT: Color32 = Color32::from_rgb(0x96, 0xcf, 0x13);
pub const BLUE: Color32 = Color32::from_rgb(0x23, 0xad, 0xff);
pub const DANGER: Color32 = Color32::from_rgb(0xff, 0x5d, 0x5d);

/// Dark theme matching the overlay: white outlines, green active, blue accent.
pub fn style(ctx: &egui::Context) {
    ctx.set_theme(Theme::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
        let v = &mut style.visuals;
        v.panel_fill = BG;
        v.window_fill = PANEL;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.extreme_bg_color = Color32::from_rgb(0x0e, 0x10, 0x13);
        v.faint_bg_color = PANEL_2;
        v.hyperlink_color = BLUE;
        v.error_fg_color = DANGER;
        v.override_text_color = Some(TEXT);
        v.selection.bg_fill = BLUE.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, BLUE);
        v.window_corner_radius = CornerRadius::same(10);
        for w in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = CornerRadius::same(6);
        }
        v.widgets.inactive.weak_bg_fill = PANEL_2;
        v.widgets.inactive.bg_fill = PANEL_2;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x2b, 0x30, 0x38);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, BLUE.gamma_multiply(0.7));
        v.widgets.active.bg_stroke = Stroke::new(1.0, BLUE);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);

        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(8.0, 6.0);
        s.button_padding = Vec2::new(10.0, 4.0);
        s.interact_size.y = 24.0;
        s.slider_width = 220.0;
        s.combo_width = 160.0;

        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(18.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(14.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
        ]
        .into();
    });
}

/// A titled panel box.
pub fn card<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(10)
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).strong().size(15.0));
                ui.add_space(4.0);
            }
            add(ui)
        })
        .inner
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(MUTED)
}
