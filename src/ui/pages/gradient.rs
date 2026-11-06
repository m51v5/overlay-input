//! Gradient stops on a bar: click to add, drag inner stops, edit the selected one below.

use egui::epaint::{Mesh, StrokeKind};
use egui::{Align2, Color32, FontId, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use serde_json::{Value, json};

use super::fields::{number, rgb_of, rgb_value};
use super::{BLUE, BORDER, MUTED, TEXT, muted};
use crate::ui::schema::tidy;

#[derive(Clone, Copy, PartialEq)]
struct Stop {
    at: f64,
    rgb: [u8; 3],
    opacity: Option<f64>,
}

impl Stop {
    fn read(v: &Value) -> Self {
        Self {
            at: v["at"].as_f64().unwrap_or(0.0),
            rgb: rgb_of(&v["rgb"]),
            opacity: v["opacity"].as_f64(),
        }
    }

    fn write(&self) -> Value {
        let mut v = json!({ "at": number(tidy(self.at)), "rgb": rgb_value(self.rgb) });
        if let Some(o) = self.opacity {
            v["opacity"] = number(tidy(o));
        }
        v
    }

    fn color(&self) -> Color32 {
        let a = (self.opacity.unwrap_or(1.0).clamp(0.0, 1.0) * 255.0).round() as u8;
        Color32::from_rgba_unmultiplied(self.rgb[0], self.rgb[1], self.rgb[2], a)
    }
}

/// Same interpolation as the overlay (`gradient::sample`).
fn sample(stops: &[Stop], at: f64) -> Stop {
    let next = stops.iter().position(|s| s.at >= at);
    let (a, b) = match next {
        None => {
            return Stop {
                at,
                ..*stops.last().expect("not empty")
            };
        }
        Some(0) => return Stop { at, ..stops[0] },
        Some(i) => (stops[i - 1], stops[i]),
    };
    let t = if b.at > a.at {
        (at - a.at) / (b.at - a.at)
    } else {
        0.0
    };
    let mix = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    let (oa, ob) = (a.opacity.unwrap_or(1.0), b.opacity.unwrap_or(1.0));
    Stop {
        at,
        rgb: [0, 1, 2].map(|i| mix(a.rgb[i], b.rgb[i])),
        opacity: Some(tidy(oa + (ob - oa) * t)),
    }
}

/// Edits `values` in place; true when changed.
pub fn editor(ui: &mut Ui, id: Id, values: &mut Vec<Value>) -> bool {
    let mut stops: Vec<Stop> = values.iter().map(Stop::read).collect();
    stops.sort_by(|a, b| a.at.total_cmp(&b.at));
    let before = stops.clone();
    if stops.is_empty() {
        ui.label(muted("No stops."));
        if ui.button("Add a stop").clicked() {
            values.push(
                Stop {
                    at: 0.0,
                    rgb: [255, 255, 255],
                    opacity: None,
                }
                .write(),
            );
            return true;
        }
        return false;
    }

    let lo = stops[0].at;
    let last = stops[stops.len() - 1].at;
    let hi = if stops.len() > 1 && last > lo {
        last
    } else {
        lo + 1.0
    };
    // About 1000 positions across the bar.
    let step = 10f64.powi((hi - lo).log10().floor() as i32 - 2);
    let mut selected = ui
        .data(|d| d.get_temp::<usize>(id))
        .unwrap_or(0)
        .min(stops.len() - 1);

    let width = ui.available_width().clamp(240.0, 520.0);
    let (rect, bar) = ui.allocate_exact_size(vec2(width, 22.0), Sense::click());
    let handles = Rect::from_min_size(rect.left_bottom(), vec2(width, 16.0));
    ui.allocate_rect(handles, Sense::hover());
    let x_of = |at: f64| rect.left() + ((at - lo) / (hi - lo)) as f32 * rect.width();
    let value_at = |x: f32| {
        let frac = ((x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64;
        ((lo + frac * (hi - lo)) / step).round() * step
    };

    let painter = ui.painter_at(rect.union(handles).expand(6.0));
    checkerboard(&painter, rect);
    let mut mesh = Mesh::default();
    for pair in stops.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (xa, xb) = (x_of(a.at), x_of(b.at));
        let i = mesh.vertices.len() as u32;
        mesh.colored_vertex(pos2(xa, rect.top()), a.color());
        mesh.colored_vertex(pos2(xb, rect.top()), b.color());
        mesh.colored_vertex(pos2(xb, rect.bottom()), b.color());
        mesh.colored_vertex(pos2(xa, rect.bottom()), a.color());
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    }
    if stops.len() == 1 {
        painter.rect_filled(rect, 0, stops[0].color());
    }
    painter.add(mesh);
    painter.rect_stroke(rect, 4, Stroke::new(1.0, BORDER), StrokeKind::Outside);

    let mut add_at = None;
    if bar.clicked()
        && let Some(p) = bar.interact_pointer_pos()
    {
        add_at = Some(value_at(p.x));
    }
    bar.on_hover_text("Click to add a stop");

    let n = stops.len();
    for i in 0..n {
        let x = x_of(stops[i].at);
        let handle = Rect::from_center_size(pos2(x, handles.top() + 8.0), vec2(12.0, 14.0));
        let fixed = i == 0 || i == n - 1;
        let response = ui.interact(handle, id.with(("stop", i)), Sense::click_and_drag());
        if response.clicked() || response.drag_started() {
            selected = i;
        }
        if !fixed
            && response.dragged()
            && let Some(p) = response.interact_pointer_pos()
        {
            let at = value_at(p.x).clamp(stops[i - 1].at, stops[i + 1].at);
            stops[i].at = at;
        }
        let on = i == selected;
        painter.rect_filled(handle, 3, stops[i].color().to_opaque());
        painter.rect_stroke(
            handle,
            3,
            Stroke::new(if on { 2.0 } else { 1.0 }, if on { BLUE } else { TEXT }),
            StrokeKind::Outside,
        );
        let tip = if fixed {
            format!("{} (end stop: set its value below)", tidy(stops[i].at))
        } else {
            format!("{} — drag to move", tidy(stops[i].at))
        };
        response.on_hover_text(tip);
    }
    let scale_font = FontId::proportional(11.0);
    painter.text(
        handles.left_bottom() + vec2(0.0, 2.0),
        Align2::LEFT_TOP,
        tidy(lo),
        scale_font.clone(),
        MUTED,
    );
    painter.text(
        handles.right_bottom() + vec2(0.0, 2.0),
        Align2::RIGHT_TOP,
        tidy(hi),
        scale_font,
        MUTED,
    );
    ui.add_space(14.0);

    if let Some(at) = add_at {
        let stop = sample(&stops, at);
        stops.push(stop);
        stops.sort_by(|a, b| a.at.total_cmp(&b.at));
        selected = stops.iter().position(|s| *s == stop).unwrap_or(0);
    }

    let mut remove = false;
    if let Some(stop) = stops.get_mut(selected) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().slider_width = 120.0;
            ui.label("at");
            ui.add(
                egui::DragValue::new(&mut stop.at)
                    .speed(step)
                    .max_decimals(4),
            );
            ui.color_edit_button_srgb(&mut stop.rgb);
            ui.label("opacity");
            let mut o = stop.opacity.unwrap_or(1.0);
            if ui
                .add(egui::Slider::new(&mut o, 0.0..=1.0).step_by(0.01))
                .changed()
            {
                stop.opacity = Some(o);
            }
            remove = ui
                .add_enabled(n > 1, egui::Button::new("Remove"))
                .on_hover_text("Remove this stop")
                .clicked();
        });
    }
    if remove {
        stops.remove(selected);
        selected = selected.saturating_sub(1);
    }
    ui.label(
        muted(format!(
            "Click the bar to add a stop. Stop {} of {} selected.",
            selected + 1,
            stops.len()
        ))
        .small(),
    );
    ui.data_mut(|d| d.insert_temp(id, selected));

    if stops == before {
        return false;
    }
    stops.sort_by(|a, b| a.at.total_cmp(&b.at));
    *values = stops.iter().map(Stop::write).collect();
    true
}

fn checkerboard(painter: &egui::Painter, rect: Rect) {
    let size = 6.0;
    painter.rect_filled(rect, 0, Color32::from_gray(60));
    let (cols, rows) = (
        (rect.width() / size).ceil() as i32,
        (rect.height() / size).ceil() as i32,
    );
    for r in 0..rows {
        for c in (r % 2..cols).step_by(2) {
            let min = Pos2::new(rect.left() + c as f32 * size, rect.top() + r as f32 * size);
            let cell = Rect::from_min_size(min, Vec2::splat(size)).intersect(rect);
            painter.rect_filled(cell, 0, Color32::from_gray(90));
        }
    }
}
