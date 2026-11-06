//! Mouse flick widget: shows "Flick!" when horizontal movement within a short window passes a threshold.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Deserialize;
use tiny_skia::{Color, FillRule, Paint, Path, Pixmap, Transform};

use super::{Canvas, Widget};
use crate::input::InputEvent;
use crate::render::easing::ease_out;
use crate::render::glow::{GlowCache, Sprite, blur_into_color};
use crate::render::text::Font;

const MARGIN: f32 = 10.0;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct FlickOptions {
    /// Window for summing horizontal movement.
    pub interval_ms: u64,
    /// Raw counts; adjust for DPI and in-game sensitivity.
    #[schemars(range(min = 1.0))]
    pub threshold: f64,
    /// Fully visible time before the fade.
    pub display_ms: u64,
    pub fade_ms: u64,
    pub text: String,
    pub font_size: f32,
    #[schemars(extend("format" = "color"))]
    pub color: [u8; 3],
    /// Glow around the text.
    #[schemars(extend("format" = "color"))]
    pub glow_color: [u8; 3],
    pub glow_blur: f32,
}

impl Default for FlickOptions {
    fn default() -> Self {
        Self {
            interval_ms: 400,
            threshold: 1000.0,
            display_ms: 250,
            fade_ms: 250,
            text: "Flick!".into(),
            font_size: 60.0,
            color: [255, 0, 0],
            glow_color: [253, 253, 18],
            glow_blur: 4.0,
        }
    }
}

pub struct MouseFlickWidget {
    options: FlickOptions,
    text: Option<Path>,
    size: (f32, f32),
    window: VecDeque<(Instant, f64)>,
    delta_x: f64,
    shown_at: Option<Instant>,
}

impl MouseFlickWidget {
    pub fn new(font: &Font, options: FlickOptions) -> Result<Self> {
        anyhow::ensure!(options.threshold > 0.0, "threshold must be > 0");
        let px = options.font_size;
        let (ascent, descent) = font.metrics(px);
        let width = font.width(&options.text, px);
        Ok(Self {
            text: font.line_path(&options.text, px, MARGIN, MARGIN + ascent),
            size: (width + 2.0 * MARGIN, ascent - descent + 2.0 * MARGIN),
            options,
            window: VecDeque::new(),
            delta_x: 0.0,
            shown_at: None,
        })
    }

    fn display(&self) -> Duration {
        Duration::from_millis(self.options.display_ms)
    }

    fn visible_until(&self) -> Duration {
        Duration::from_millis(self.options.display_ms + self.options.fade_ms)
    }

    fn opacity(&self, now: Instant) -> f32 {
        let Some(t) = self.shown_at else { return 0.0 };
        let elapsed = now.saturating_duration_since(t);
        if elapsed < self.display() {
            1.0
        } else if elapsed < self.visible_until() {
            let p =
                (elapsed - self.display()).as_secs_f32() / (self.options.fade_ms as f32 / 1000.0);
            1.0 - ease_out(p)
        } else {
            0.0
        }
    }

    fn glow_sprite(&self, scale: f32) -> Sprite {
        let sigma = self.options.glow_blur / 2.0 * scale;
        let pad = (3.0 * sigma).ceil();
        let (w, h) = (
            (self.size.0 * scale).ceil() + 2.0 * pad,
            (self.size.1 * scale).ceil() + 2.0 * pad,
        );
        let mut mask = Pixmap::new(w as u32, h as u32).expect("flick sprite size");
        if let Some(text) = &self.text {
            let mut paint = Paint::default();
            paint.set_color(Color::WHITE);
            let ts = Transform::from_row(scale, 0.0, 0.0, scale, pad, pad);
            mask.fill_path(text, &paint, FillRule::Winding, ts, None);
        }
        blur_into_color(&mut mask, sigma, self.options.glow_color);
        Sprite {
            pixmap: mask,
            offset: (-(pad as i32), -(pad as i32)),
        }
    }
}

impl Widget for MouseFlickWidget {
    fn size(&self) -> (f32, f32) {
        self.size
    }

    fn on_input(&mut self, event: &InputEvent, now: Instant) -> bool {
        let InputEvent::MouseMove { dx, .. } = *event else {
            return false;
        };
        // The first event of a fresh window is only recorded.
        if self.window.is_empty() {
            self.window.push_back((now, dx));
            self.delta_x = dx;
            return false;
        }
        self.window.push_back((now, dx));
        self.delta_x += dx;
        let interval = Duration::from_millis(self.options.interval_ms);
        while let Some(&(t, old)) = self.window.front() {
            if now.saturating_duration_since(t) <= interval {
                break;
            }
            self.delta_x -= old;
            self.window.pop_front();
        }

        let showing = self
            .shown_at
            .is_some_and(|t| now.saturating_duration_since(t) < self.display());
        if self.delta_x.abs() > self.options.threshold && !showing {
            self.shown_at = Some(now);
            self.window.clear();
            self.delta_x = 0.0;
            return true;
        }
        false
    }

    fn reset(&mut self) {
        self.window.clear();
        self.delta_x = 0.0;
        self.shown_at = None;
    }

    fn is_animating(&self, now: Instant) -> bool {
        self.shown_at
            .is_some_and(|t| now.saturating_duration_since(t) < self.visible_until())
    }

    fn draw(&self, canvas: &mut Canvas, origin: (f32, f32), now: Instant) {
        let opacity = self.opacity(now);
        let Some(text) = &self.text else { return };
        if opacity <= 0.0 {
            return;
        }
        let s = canvas.scale;
        let key = format!(
            "flick-glow-{}-{:?}",
            self.options.text, self.options.glow_color
        );
        let (layer, glow) = canvas.highlight_and_glow();
        let sprite = glow.sprite(&key, s, || self.glow_sprite(s));
        GlowCache::blit(layer, sprite, origin.0 * s, origin.1 * s, opacity);

        let [r, g, b] = self.options.color;
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(r, g, b, (opacity * 255.0).round() as u8));
        let ts = Transform::from_row(s, 0.0, 0.0, s, origin.0 * s, origin.1 * s);
        canvas
            .highlight()
            .fill_path(text, &paint, FillRule::Winding, ts, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::snapshot::render_snapshot;

    fn widget() -> MouseFlickWidget {
        let font = Font::load_system(&["verdana.ttf", "segoeui.ttf", "arial.ttf"]).unwrap();
        MouseFlickWidget::new(&font, FlickOptions::default()).unwrap()
    }

    fn mv(dx: f64) -> InputEvent {
        InputEvent::MouseMove { dx, dy: 0.0 }
    }

    #[test]
    fn fast_horizontal_move_triggers_then_fades() {
        let mut w = widget();
        let t0 = Instant::now();
        assert!(!w.on_input(&mv(600.0), t0));
        assert!(w.on_input(&mv(600.0), t0 + Duration::from_millis(50)));
        assert_eq!(w.opacity(t0 + Duration::from_millis(100)), 1.0);
        assert!(w.opacity(t0 + Duration::from_millis(400)) < 1.0);
        assert!(!w.is_animating(t0 + Duration::from_millis(600)));
    }

    #[test]
    fn slow_move_outside_window_does_not_trigger() {
        let mut w = widget();
        let t0 = Instant::now();
        w.on_input(&mv(600.0), t0);
        w.on_input(&mv(10.0), t0 + Duration::from_millis(10));
        assert!(
            !w.on_input(&mv(600.0), t0 + Duration::from_millis(500)),
            "first 600 left the window"
        );
    }

    #[test]
    #[ignore = "writes target/snapshots/mouse-flick.png"]
    fn snapshot() {
        let mut w = widget();
        let t0 = Instant::now();
        w.on_input(&mv(600.0), t0);
        w.on_input(&mv(600.0), t0);
        render_snapshot(&mut w, "mouse-flick", t0 + Duration::from_millis(10));
    }
}
