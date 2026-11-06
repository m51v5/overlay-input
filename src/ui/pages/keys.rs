//! Clickable keyboard for `hidden_keys`.

use std::sync::OnceLock;

use egui::epaint::StrokeKind;
use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::{BLUE, GREEN, MUTED, PANEL_2, TEXT, muted};
use crate::widgets::keyboard::{KeyLayout, Preset, layout_info};

fn layout(preset: Preset) -> &'static KeyLayout {
    static LAYOUTS: OnceLock<[KeyLayout; 2]> = OnceLock::new();
    let [default, full] =
        LAYOUTS.get_or_init(|| [layout_info(Preset::Default), layout_info(Preset::Full)]);
    match preset {
        Preset::Default => default,
        Preset::Full => full,
    }
}

/// Toggles names in `hidden` (which may also hold keys of the other preset); true when changed.
pub fn picker(ui: &mut Ui, preset: &str, hidden: &mut Vec<String>) -> bool {
    let preset: Preset = serde_json::from_value(preset.into()).unwrap_or_default();
    let layout = layout(preset);
    let before = hidden.clone();
    let is_hidden = |hidden: &[String], name: &str| hidden.iter().any(|h| h == name);
    let shown = layout
        .keys
        .iter()
        .filter(|k| !is_hidden(hidden, &k.name))
        .count();

    ui.horizontal_wrapped(|ui| {
        ui.label(muted(format!(
            "Click keys to hide or show them. {shown} of {} shown.",
            layout.keys.len()
        )));
        if ui
            .add_enabled(!hidden.is_empty(), egui::Button::new("Show all"))
            .clicked()
        {
            hidden.clear();
        }
        if ui.button("Hide all").clicked() {
            for key in &layout.keys {
                if !is_hidden(hidden, &key.name) {
                    hidden.push(key.name.clone());
                }
            }
        }
    });

    let width = ui.available_width().min(760.0);
    let k = width / layout.size[0];
    let (board, _) = ui.allocate_exact_size(vec2(width, layout.size[1] * k), Sense::hover());
    let painter = ui.painter_at(board);
    let font = FontId::proportional((13.0 * k).clamp(9.0, 14.0));
    for key in &layout.keys {
        let [x, y, w, h] = key.rect;
        let rect =
            Rect::from_min_size(board.min + vec2(x * k, y * k), vec2(w * k, h * k)).shrink(1.5);
        let response = ui.interact(rect, ui.scope_id().with(("key", &key.name)), Sense::click());
        let off = is_hidden(hidden, &key.name);
        let (fill, stroke, text) = if off {
            (
                Color32::TRANSPARENT,
                Stroke::new(1.0, MUTED.gamma_multiply(0.5)),
                MUTED.gamma_multiply(0.6),
            )
        } else {
            (PANEL_2, Stroke::new(1.0, GREEN), TEXT)
        };
        let stroke = if response.hovered() {
            Stroke::new(1.5, BLUE)
        } else {
            stroke
        };
        painter.rect_filled(rect, 4, fill);
        painter.rect_stroke(rect, 4, stroke, StrokeKind::Inside);
        painter.text(
            pos2(rect.center().x, rect.center().y),
            Align2::CENTER_CENTER,
            &key.label,
            font.clone(),
            text,
        );
        let state = if off {
            "hidden — click to show"
        } else {
            "shown — click to hide"
        };
        if response
            .on_hover_text(format!("{}: {state}", key.name))
            .clicked()
        {
            if off {
                hidden.retain(|h| h != &key.name);
            } else {
                hidden.push(key.name.clone());
            }
        }
    }
    *hidden != before
}
