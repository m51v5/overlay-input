//! One schema-driven option control per field; unset fields show their default.

use egui::{Grid, RichText, Ui};
use serde_json::{Map, Value};

use super::{gradient, keys, muted};
use crate::ui::schema::{FieldKind, FieldSpec, tidy};

/// Option controls in a label | control grid; the key picker and gradient bar get full-width rows.
/// `fallback` gives the value an unset optional number shows (what "auto" means now).
pub fn form(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    specs: &[FieldSpec],
    map: &mut Map<String, Value>,
    fallback: impl Fn(&str) -> Option<f64>,
) {
    let wide = |s: &FieldSpec| matches!(s.kind, FieldKind::Keys | FieldKind::Gradient);
    ui.push_id(id, |ui| {
        let mut rest = specs;
        let mut chunk = 0;
        while !rest.is_empty() {
            let split = rest.iter().position(wide).unwrap_or(rest.len());
            let (narrow, after) = rest.split_at(split);
            if !narrow.is_empty() {
                Grid::new(chunk)
                    .num_columns(2)
                    .spacing([16.0, 10.0])
                    .min_col_width(150.0)
                    .show(ui, |ui| {
                        for spec in narrow {
                            label(ui, spec, map);
                            ui.vertical(|ui| {
                                control(ui, spec, map, fallback(&spec.key));
                                if let Some(help) = &spec.help {
                                    ui.label(muted(help.as_str()).small());
                                }
                            });
                            ui.end_row();
                        }
                    });
                chunk += 1;
            }
            let Some((spec, after)) = after.split_first() else {
                break;
            };
            ui.add_space(8.0);
            label(ui, spec, map);
            if let Some(help) = &spec.help {
                ui.label(muted(help.as_str()).small());
            }
            control(ui, spec, map, None);
            ui.add_space(8.0);
            rest = after;
        }
    });
}

/// Option name (help on hover) and a reset button while it's set.
fn label(ui: &mut Ui, spec: &FieldSpec, map: &mut Map<String, Value>) {
    let is_set = map.get(&spec.key).is_some_and(|v| !v.is_null());
    ui.horizontal(|ui| {
        let label = ui.label(if is_set {
            RichText::new(&spec.label)
        } else {
            RichText::new(&spec.label).color(super::MUTED)
        });
        if let Some(help) = &spec.help {
            label.on_hover_text(help);
        }
        if is_set
            && (spec.default.is_some() || spec.optional)
            && ui
                .small_button("↺")
                .on_hover_text("Back to default")
                .clicked()
        {
            map.remove(&spec.key);
        }
    });
}

fn control(ui: &mut Ui, spec: &FieldSpec, map: &mut Map<String, Value>, fallback: Option<f64>) {
    let key = spec.key.as_str();
    let current = map
        .get(key)
        .filter(|v| !v.is_null())
        .or(spec.default.as_ref())
        .cloned()
        .unwrap_or(Value::Null);
    match spec.kind {
        FieldKind::Boolean => {
            let mut on = current.as_bool().unwrap_or(false);
            if ui.checkbox(&mut on, "").changed() {
                map.insert(key.to_owned(), on.into());
            }
        }
        FieldKind::Enum => {
            let value = current.as_str().unwrap_or_default().to_owned();
            let label = spec
                .options
                .iter()
                .find(|o| o.value == value)
                .map_or(value.clone(), |o| o.label.clone());
            egui::ComboBox::from_id_salt(ui.scope_id().with(key))
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for option in &spec.options {
                        let r = ui.selectable_label(option.value == value, &option.label);
                        let r = match &option.help {
                            Some(help) => r.on_hover_text(help),
                            None => r,
                        };
                        if r.clicked() {
                            map.insert(key.to_owned(), option.value.clone().into());
                        }
                    }
                });
        }
        FieldKind::Color => {
            let mut rgb = rgb_of(&current);
            ui.horizontal(|ui| {
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    map.insert(key.to_owned(), rgb_value(rgb));
                }
                ui.label(muted(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])).monospace());
            });
        }
        FieldKind::Pair => {
            let mut pair = [0, 1].map(|i| current[i].as_f64().unwrap_or(0.0));
            let mut changed = false;
            ui.horizontal(|ui| {
                for v in &mut pair {
                    changed |= ui
                        .add(egui::DragValue::new(v).speed(0.5).max_decimals(2))
                        .changed();
                }
            });
            if changed {
                map.insert(key.to_owned(), Value::Array(pair.map(number).to_vec()));
            }
        }
        FieldKind::Number | FieldKind::Integer => number_control(ui, spec, map, &current, fallback),
        FieldKind::String => {
            let mut text = current.as_str().unwrap_or_default().to_owned();
            let edit = egui::TextEdit::singleline(&mut text).desired_width(240.0);
            if ui.add(edit).changed() {
                if text.is_empty() && spec.optional {
                    map.remove(key);
                } else {
                    map.insert(key.to_owned(), text.into());
                }
            }
        }
        FieldKind::Gradient => {
            let mut stops = current.as_array().cloned().unwrap_or_default();
            if gradient::editor(ui, ui.scope_id().with(key), &mut stops) {
                map.insert(key.to_owned(), Value::Array(stops));
            }
        }
        FieldKind::Keys => {
            let preset = map
                .get("preset")
                .and_then(Value::as_str)
                .unwrap_or("default")
                .to_owned();
            let mut hidden: Vec<String> = current
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            if keys::picker(ui, &preset, &mut hidden) {
                if hidden.is_empty() {
                    map.remove(key);
                } else {
                    map.insert(key.to_owned(), hidden.into());
                }
            }
        }
        FieldKind::Unknown => {
            ui.label(muted(current.to_string()).monospace());
        }
    }
}

fn number_control(
    ui: &mut Ui,
    spec: &FieldSpec,
    map: &mut Map<String, Value>,
    current: &Value,
    fallback: Option<f64>,
) {
    let key = spec.key.as_str();
    let integer = spec.kind == FieldKind::Integer;
    // Tidied, so the slider's step snapping doesn't count f32 noise (0.6000000238) as an edit.
    let shown = current.as_f64().or(fallback).map(tidy);
    let Some(mut v) = shown else {
        // Optional, no default: "auto" until the user sets it.
        ui.horizontal(|ui| {
            ui.label(muted("auto"));
            if ui.small_button("Set").clicked() {
                map.insert(key.to_owned(), number(spec.min.unwrap_or(0.0)));
            }
        });
        return;
    };
    let changed = match (spec.min, spec.max) {
        (Some(lo), Some(hi)) => {
            let mut slider = egui::Slider::new(&mut v, lo..=hi);
            if let Some(step) = spec.step {
                slider = slider.step_by(step);
            }
            if integer {
                slider = slider.integer();
            }
            ui.add(slider).changed()
        }
        (lo, hi) => {
            let speed = spec.step.unwrap_or(if integer {
                1.0
            } else {
                // Small values (e.g. 0.00002) need a small step to show their digits.
                if v == 0.0 { 0.01 } else { v.abs() * 0.01 }
            });
            let mut drag = egui::DragValue::new(&mut v).speed(speed);
            drag = drag.range(lo.unwrap_or(f64::NEG_INFINITY)..=hi.unwrap_or(f64::INFINITY));
            if integer {
                drag = drag.max_decimals(0);
            }
            ui.add(drag).changed()
        }
    };
    let v = if integer { v.round() } else { tidy(v) };
    if changed && shown != Some(v) {
        map.insert(key.to_owned(), number(v));
    }
    if spec.optional && current.is_null() {
        ui.label(muted("not set: the default applies").small());
    }
}

/// Whole numbers as integers (like a hand-written file), others trimmed of f32 noise.
pub fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        Value::from(v as i64)
    } else {
        Value::from(tidy(v))
    }
}

pub fn rgb_of(value: &Value) -> [u8; 3] {
    [0, 1, 2].map(|i| value[i].as_f64().unwrap_or(255.0).clamp(0.0, 255.0) as u8)
}

pub fn rgb_value(rgb: [u8; 3]) -> Value {
    Value::Array(rgb.iter().map(|&c| Value::from(c)).collect())
}
