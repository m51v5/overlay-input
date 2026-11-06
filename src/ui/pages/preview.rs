//! Screen preview: the target monitor in miniature with the overlay in place, plus a close-up,
//! drawn by the overlay's own renderer.

use std::time::{Duration, Instant};

use egui::epaint::StrokeKind;
use egui::{
    Color32, ColorImage, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions, Ui, pos2,
    vec2,
};
use serde_json::Value;

use super::{BORDER, DANGER, card, muted};
use crate::overlay::MonitorInfo;
use crate::ui::host::{Command, HostView, Outcome, Reply};
use crate::ui::schema::Forms;
use crate::ui::tree;

/// Edits settle this long before a new render.
const RENDER_DEBOUNCE: Duration = Duration::from_millis(100);

pub struct PreviewState {
    sample: bool,
    texture: Option<TextureHandle>,
    /// Rendered image waiting for upload (results arrive outside the frame).
    pending: Option<ColorImage>,
    /// What was last asked to render, and what the page last showed.
    tried: Option<(Value, bool)>,
    seen: Option<(Value, bool)>,
    changed_at: Instant,
    error: Option<String>,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            sample: true,
            texture: None,
            pending: None,
            tried: None,
            seen: None,
            changed_at: Instant::now(),
            error: None,
        }
    }
}

impl PreviewState {
    pub fn on_result(&mut self, outcome: Outcome) {
        match outcome {
            Ok(Reply::Image(pixmap)) => {
                let size = [pixmap.width() as usize, pixmap.height() as usize];
                self.pending = Some(ColorImage::from_rgba_premultiplied(size, pixmap.data()));
                self.error = None;
            }
            Ok(_) => {}
            Err(e) => self.error = Some(e),
        }
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        profile: &Value,
        forms: &Forms,
        host: &HostView,
    ) -> Option<Command> {
        if let Some(image) = self.pending.take() {
            match &mut self.texture {
                Some(texture) => texture.set(image, TextureOptions::LINEAR),
                None => {
                    self.texture = Some(ui.ctx().load_texture(
                        "overlay-preview",
                        image,
                        TextureOptions::LINEAR,
                    ));
                }
            }
        }

        let get = |key: &str| {
            profile
                .get(key)
                .filter(|v| !v.is_null())
                .or_else(|| forms.profile_field(key)?.default.as_ref())
                .cloned()
                .unwrap_or(Value::Null)
        };
        let opacity = get("opacity").as_f64().unwrap_or(1.0);
        // Two opacities can't be one image tint: the renderer bakes them in then.
        let split = get("highlight_opacity")
            .as_f64()
            .is_some_and(|h| h != opacity);
        let mut command = None;

        card(ui, "Screen preview", |ui| {
            ui.checkbox(&mut self.sample, "Show sample input")
                .on_hover_text("Some keys, buttons and a stick held, to judge colors and glow");
            command = self.request(ui, render_json(profile, split));

            let wanted = get("monitor").as_u64().unwrap_or(0) as usize;
            let monitor = host
                .monitors
                .iter()
                .find(|m| m.index == wanted)
                .or_else(|| host.monitors.iter().find(|m| m.primary))
                .or(host.monitors.first());
            let tint = Color32::from_white_alpha(if split {
                255
            } else {
                (opacity.clamp(0.0, 1.0) * 255.0) as u8
            });

            if let Some(m) = monitor {
                let (w, h) = (m.bounds[2] - m.bounds[0], m.bounds[3] - m.bounds[1]);
                ui.label(muted(format!(
                    "Monitor #{} · {w}×{h} · {}%{}",
                    m.index,
                    (m.scale * 100.0).round(),
                    if m.primary { " · primary" } else { "" }
                )));
                let anchor = get("anchor").as_str().unwrap_or("bottom-left").to_owned();
                let offset = [0, 1].map(|i| get("offset")[i].as_f64().unwrap_or(0.0));
                self.screen(ui, m, &anchor, offset, tint);
            }

            ui.add_space(8.0);
            ui.label(muted("Close-up").small());
            self.closeup(ui, tint);
            if let Some(e) = &self.error {
                ui.label(RichText::new(format!("● {e}")).color(DANGER));
            }
        });
        command
    }

    /// Asks for a render once edits settle.
    fn request(&mut self, ui: &Ui, json: Value) -> Option<Command> {
        let empty = json
            .get("layout")
            .is_none_or(|l| tree::widget_count(l) == 0);
        let want = (json, self.sample);
        if self.seen.as_ref() != Some(&want) {
            self.seen = Some(want.clone());
            self.changed_at = Instant::now();
        }
        if self.tried.as_ref() == Some(&want) || empty {
            return None;
        }
        let waited = self.changed_at.elapsed();
        if self.tried.is_some() && waited < RENDER_DEBOUNCE {
            ui.ctx().request_repaint_after(RENDER_DEBOUNCE - waited);
            return None;
        }
        self.tried = Some(want.clone());
        Some(Command::Render {
            profile: want.0,
            sample: want.1,
        })
    }

    fn screen(&self, ui: &mut Ui, m: &MonitorInfo, anchor: &str, offset: [f64; 2], tint: Color32) {
        let (mw, mh) = (
            (m.bounds[2] - m.bounds[0]) as f32,
            (m.bounds[3] - m.bounds[1]) as f32,
        );
        let width = ui.available_width();
        let k = width / mw.max(1.0);
        let (screen, _) = ui.allocate_exact_size(vec2(width, mh * k), Sense::hover());
        let painter = ui.painter_at(screen);
        painter.rect_filled(screen, 6, Color32::from_rgb(0x0b, 0x0d, 0x10));
        let rel = |x: i32, y: i32| {
            pos2(
                screen.left() + (x - m.bounds[0]) as f32 * k,
                screen.top() + (y - m.bounds[1]) as f32 * k,
            )
        };
        let work = Rect::from_min_max(
            rel(m.work_area[0], m.work_area[1]),
            rel(m.work_area[2], m.work_area[3]),
        );
        painter.rect_filled(work, 4, Color32::from_rgb(0x1e, 0x2a, 0x3a));
        painter.rect_stroke(screen, 6, Stroke::new(1.0, BORDER), StrokeKind::Inside);

        let Some(texture) = &self.texture else {
            return;
        };
        let [iw, ih] = texture.size().map(|v| v as i32);
        let ox = (offset[0] * m.scale as f64).round() as i32;
        let oy = (offset[1] * m.scale as f64).round() as i32;
        let [x, y] = place(anchor, m.work_area, iw, ih, ox, oy);
        let at = rel(x, y);
        let rect = Rect::from_min_size(at, vec2(iw as f32 * k, ih as f32 * k));
        painter.image(
            texture.id(),
            rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            tint,
        );
    }

    /// The overlay at its real size on a same-DPI screen, shrunk to fit.
    fn closeup(&self, ui: &mut Ui, tint: Color32) {
        let Some(texture) = &self.texture else {
            ui.label(muted("Rendering…"));
            return;
        };
        let [w, h] = texture.size().map(|v| v as f32 / ui.pixels_per_point());
        let fit = (ui.available_width() / w).min(1.0);
        let (rect, _) = ui.allocate_exact_size(vec2(w * fit, h * fit), Sense::hover());
        ui.painter().image(
            texture.id(),
            rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            tint,
        );
    }
}

/// The profile without what the page applies itself (placement, single opacity), so dragging those
/// never waits for a render.
fn render_json(profile: &Value, split: bool) -> Value {
    let mut json = profile.clone();
    if let Some(map) = json.as_object_mut() {
        map.remove("anchor");
        map.remove("offset");
        if !split {
            map.remove("opacity");
            map.remove("highlight_opacity");
        }
    }
    json
}

/// Same as `Anchor::position` in the overlay: top-left of the image, physical px.
fn place(anchor: &str, area: [i32; 4], w: i32, h: i32, ox: i32, oy: i32) -> [i32; 2] {
    let [l, t, r, b] = area;
    match anchor {
        "top-left" => [l + ox, t + oy],
        "top-right" => [r - ox - w, t + oy],
        "bottom-right" => [r - ox - w, b - oy - h],
        "center" => [l + (r - l - w) / 2 + ox, t + (b - t - h) / 2 + oy],
        _ => [l + ox, b - oy - h],
    }
}
