//! Placement card: anchor on a mini screen, monitor, and the profile's own settings.

use egui::epaint::StrokeKind;
use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::Value;

use super::{BLUE, BORDER, MUTED, PANEL_2, card, fields, muted};
use crate::ui::host::HostView;
use crate::ui::schema::Forms;

const ANCHORS: [(&str, f32, f32); 5] = [
    ("top-left", 0.0, 0.0),
    ("top-right", 1.0, 0.0),
    ("center", 0.5, 0.5),
    ("bottom-left", 0.0, 1.0),
    ("bottom-right", 1.0, 1.0),
];

pub fn show(ui: &mut Ui, profile: &mut Value, forms: &Forms, host: &HostView) {
    let Some(map) = profile.as_object_mut() else {
        card(ui, "Placement", |ui| {
            ui.label(muted("This profile is not a table."))
        });
        return;
    };
    card(ui, "Placement", |ui| {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.label(muted("Anchor"));
                let current = map
                    .get("anchor")
                    .and_then(Value::as_str)
                    .unwrap_or("bottom-left");
                if let Some(anchor) = anchor_picker(ui, current) {
                    map.insert("anchor".into(), anchor.into());
                }
            });
            ui.add_space(24.0);
            ui.vertical(|ui| {
                ui.label(muted("Monitor"));
                let current = map.get("monitor").and_then(Value::as_u64).unwrap_or(0) as usize;
                let label = |i: usize| match host.monitors.get(i) {
                    Some(m) => format!(
                        "#{i} · {}×{}{}",
                        m.bounds[2] - m.bounds[0],
                        m.bounds[3] - m.bounds[1],
                        if m.primary { " · primary" } else { "" }
                    ),
                    None => format!("#{i} (not connected: falls back to primary)"),
                };
                egui::ComboBox::from_id_salt("monitor")
                    .width(240.0)
                    .selected_text(label(current))
                    .show_ui(ui, |ui| {
                        for m in &host.monitors {
                            if ui
                                .selectable_label(m.index == current, label(m.index))
                                .clicked()
                            {
                                map.insert("monitor".into(), m.index.into());
                            }
                        }
                    });
            });
        });
        ui.add_space(8.0);
        // Unset highlight opacity follows the main opacity; its slider shows that.
        let opacity = map
            .get("opacity")
            .and_then(Value::as_f64)
            .or_else(|| forms.profile_field("opacity")?.default.as_ref()?.as_f64());
        fields::form(ui, "placement", &forms.profile, map, |key| {
            (key == "highlight_opacity").then_some(opacity).flatten()
        });
    });
}

/// Corners and center on a mini screen with a taskbar; returns the clicked anchor.
fn anchor_picker(ui: &mut Ui, current: &str) -> Option<&'static str> {
    let (screen, _) = ui.allocate_exact_size(vec2(132.0, 84.0), Sense::hover());
    let painter = ui.painter_at(screen.expand(2.0));
    painter.rect_filled(screen, 6, PANEL_2);
    painter.rect_stroke(screen, 6, Stroke::new(1.0, BORDER), StrokeKind::Inside);
    let taskbar = Rect::from_min_max(pos2(screen.left(), screen.bottom() - 8.0), screen.max);
    painter.rect_filled(taskbar.shrink2(vec2(1.0, 1.0)), 0, BORDER);
    let area = Rect::from_min_max(screen.min, pos2(screen.right(), taskbar.top())).shrink(12.0);

    let mut picked = None;
    for (name, fx, fy) in ANCHORS {
        let center = pos2(
            area.left() + fx * area.width(),
            area.top() + fy * area.height(),
        );
        let hit = Rect::from_center_size(center, vec2(22.0, 18.0));
        let response = ui.interact(hit, ui.scope_id().with(name), Sense::click());
        let on = name == current;
        let color = if on {
            BLUE
        } else if response.hovered() {
            BLUE.gamma_multiply(0.6)
        } else {
            MUTED.gamma_multiply(0.5)
        };
        painter.rect_filled(Rect::from_center_size(center, vec2(16.0, 11.0)), 2, color);
        if response.on_hover_text(name).clicked() {
            picked = Some(name);
        }
    }
    picked
}
