//! Mouse speed widget: speed text (inch/sec by default), refreshed every 100 ms, gradient-colored.

use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Deserialize;
use tiny_skia::{Color, FillRule, Paint, Path, Transform};

use super::{Canvas, Widget};
use crate::input::InputEvent;
use crate::render::gradient::{self, Stop};
use crate::render::text::Font;

const MARGIN: f32 = 10.0;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct SpeedOptions {
    /// Mouse DPI (counts per inch).
    #[schemars(range(min = 1.0))]
    pub cpi: f32,
    #[schemars(range(min = 10))]
    pub refresh_ms: u64,
    pub decimals: usize,
    pub units: String,
    /// inch/sec × this (1/39 = m/s, 1/10.936 = km/h, 1/17.6 = mph).
    pub multiplier: f32,
    pub font_size: f32,
    /// Speed → color/opacity.
    pub gradient: Vec<Stop>,
}

impl Default for SpeedOptions {
    fn default() -> Self {
        Self {
            cpi: 1200.0,
            refresh_ms: 100,
            decimals: 2,
            units: "inch/sec".into(),
            multiplier: 1.0,
            font_size: 60.0,
            gradient: vec![
                Stop::new(0.0, [255, 255, 255], 1.0),
                Stop::new(10.0, [255, 255, 71], 1.0),
                Stop::new(20.0, [255, 71, 71], 1.0),
            ],
        }
    }
}

pub struct MouseSpeedWidget {
    font: Rc<Font>,
    options: SpeedOptions,
    distance: f64,
    window_start: Option<Instant>,
    speed: f32,
    text: Option<Path>,
    color: Color,
    size: (f32, f32),
    baseline: f32,
}

impl MouseSpeedWidget {
    pub fn new(font: &Rc<Font>, options: SpeedOptions) -> Result<Self> {
        anyhow::ensure!(options.cpi > 0.0, "cpi must be > 0");
        anyhow::ensure!(options.refresh_ms > 0, "refresh_ms must be > 0");
        anyhow::ensure!(
            !options.gradient.is_empty(),
            "gradient needs at least one stop"
        );

        let font = Rc::clone(font);
        let (ascent, descent) = font.metrics(options.font_size);
        // Fixed box sized for "00.00 units" so the layout doesn't jump as digits change.
        let widest = format!("{:.*} {}", options.decimals, 88.0, options.units);
        let width = font.width(&widest, options.font_size);
        let mut widget = Self {
            size: (width + 2.0 * MARGIN, ascent - descent + 2.0 * MARGIN),
            baseline: MARGIN + ascent,
            font,
            options,
            distance: 0.0,
            window_start: None,
            speed: 0.0,
            text: None,
            color: Color::WHITE,
        };
        widget.set_speed(0.0);
        Ok(widget)
    }

    fn set_speed(&mut self, speed: f32) {
        self.speed = speed;
        let text = format!("{:.*} {}", self.options.decimals, speed, self.options.units);
        let px = self.options.font_size;
        let left = (self.size.0 - self.font.width(&text, px)) / 2.0;
        self.text = self.font.line_path(&text, px, left, self.baseline);
        self.color = gradient::sample(&self.options.gradient, speed);
    }

    fn refresh(&self) -> Duration {
        Duration::from_millis(self.options.refresh_ms)
    }
}

impl Widget for MouseSpeedWidget {
    fn size(&self) -> (f32, f32) {
        self.size
    }

    fn on_input(&mut self, event: &InputEvent, now: Instant) -> bool {
        let InputEvent::MouseMove { dx, dy } = *event else {
            return false;
        };
        self.distance += (dx * dx + dy * dy).sqrt();
        self.window_start.get_or_insert(now);
        true
    }

    fn tick(&mut self, now: Instant) {
        let Some(start) = self.window_start else {
            return;
        };
        if now.saturating_duration_since(start) < self.refresh() {
            return;
        }
        let inch_per_sec =
            1000.0 * self.distance as f32 / self.options.refresh_ms as f32 / self.options.cpi;
        self.set_speed(self.options.multiplier * inch_per_sec);
        self.distance = 0.0;
        // Keep sampling until a window with no movement brings the readout back to 0.
        self.window_start = (self.speed != 0.0).then_some(now);
    }

    fn reset(&mut self) {
        self.distance = 0.0;
        self.window_start = None;
        self.set_speed(0.0);
    }

    fn is_animating(&self, _now: Instant) -> bool {
        self.window_start.is_some()
    }

    fn draw(&self, canvas: &mut Canvas, origin: (f32, f32), _now: Instant) {
        let Some(text) = &self.text else { return };
        let s = canvas.scale;
        let ts = Transform::from_row(s, 0.0, 0.0, s, origin.0 * s, origin.1 * s);
        let mut paint = Paint::default();
        paint.set_color(self.color);
        canvas
            .pixmap
            .fill_path(text, &paint, FillRule::Winding, ts, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::snapshot::render_snapshot;

    fn verdana() -> Rc<Font> {
        Rc::new(Font::load_system(&["verdana.ttf", "segoeui.ttf", "arial.ttf"]).unwrap())
    }

    #[test]
    fn speed_from_counts() {
        let mut w = MouseSpeedWidget::new(&verdana(), SpeedOptions::default()).unwrap();
        let t0 = Instant::now();
        // 1200 counts in 100 ms at 1200 cpi = 10 inch/sec.
        w.on_input(
            &InputEvent::MouseMove {
                dx: 720.0,
                dy: 960.0,
            },
            t0,
        );
        w.tick(t0 + Duration::from_millis(100));
        assert!((w.speed - 10.0).abs() < 1e-3, "{}", w.speed);
        assert!(w.is_animating(t0), "keeps sampling");
        w.tick(t0 + Duration::from_millis(200));
        assert_eq!(w.speed, 0.0);
        assert!(!w.is_animating(t0), "idle once back to zero");
    }

    #[test]
    #[ignore = "writes target/snapshots/mouse-speed.png"]
    fn snapshot() {
        let mut w = MouseSpeedWidget::new(&verdana(), SpeedOptions::default()).unwrap();
        w.set_speed(12.34);
        render_snapshot(&mut w, "mouse-speed", Instant::now());
    }
}
