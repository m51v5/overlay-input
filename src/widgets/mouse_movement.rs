//! Mouse movement widget: a ring of ellipses that light up in the direction the mouse moves.

use std::f32::consts::TAU;
use std::time::Instant;

use anyhow::Result;
use serde::Deserialize;
use tiny_skia::{Color, FillRule, Paint, Path, PathBuilder, Rect, Stroke, Transform};

use super::{Canvas, Widget};
use crate::input::InputEvent;
use crate::render::gradient::{self, Stop};

/// Intensity is tuned per 60 Hz frame; per-tick increments scale to match.
const REFERENCE_FRAME_MS: f32 = 1000.0 / 60.0;
const BORDER: f32 = 2.0;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct MovementOptions {
    pub radius: f32,
    pub padding: f32,
    #[schemars(range(min = 3, max = 360))]
    pub elements: usize,
    pub element_size: [f32; 2],
    /// Above 1; every millisecond intensity is divided by this (closer to 1 = slower fade).
    #[schemars(range(min = 1.0001, max = 1.1))]
    pub fade_factor: f32,
    /// Raw counts per second × this = intensity added (change with DPI).
    pub speed_factor: f32,
    pub affected_neighbours: usize,
    pub neighbour_fade_factor: f32,
    /// Intensity (0 to 1) → color and opacity. The first stop should have opacity 0.
    pub gradient: Vec<Stop>,
}

impl Default for MovementOptions {
    fn default() -> Self {
        Self {
            radius: 320.0,
            padding: 20.0,
            elements: 30,
            element_size: [48.0, 24.0],
            fade_factor: 1.005,
            speed_factor: 0.00001,
            affected_neighbours: 1,
            neighbour_fade_factor: 2.0,
            gradient: vec![
                Stop::new(0.0, [45, 100, 38], 0.0),
                Stop::new(0.3, [45, 100, 38], 1.0),
                Stop::new(0.5, [255, 225, 61], 1.0),
                Stop::new(0.8, [255, 61, 61], 1.0),
                Stop::new(1.0, [255, 61, 61], 1.0),
            ],
        }
    }
}

struct Element {
    fill: Path,
    border: Path,
}

pub struct MouseMovementWidget {
    options: MovementOptions,
    ring: Vec<Element>,
    intensity: Vec<f32>,
    pending: (f64, f64),
    last_tick: Option<Instant>,
    size: (f32, f32),
}

impl MouseMovementWidget {
    pub fn new(options: MovementOptions) -> Result<Self> {
        anyhow::ensure!(options.elements >= 3, "elements must be >= 3");
        anyhow::ensure!(options.fade_factor > 1.0, "fade_factor must be > 1");
        anyhow::ensure!(
            !options.gradient.is_empty(),
            "gradient needs at least one stop"
        );

        let [ew, eh] = options.element_size;
        let (ow, oh) = (ew + 2.0 * BORDER, eh + 2.0 * BORDER);
        let n = options.elements;
        let r = options.radius;

        // div left/top sits on the circle; it rotates around its own center.
        let placed: Vec<(Transform, f32)> = (0..n)
            .map(|i| {
                let angle = TAU * i as f32 / n as f32;
                let (cx, cy) = (
                    r + angle.cos() * r + ow / 2.0,
                    r + angle.sin() * r + oh / 2.0,
                );
                let deg = 90.0 + 360.0 * i as f32 / n as f32;
                (Transform::from_rotate(deg).post_translate(cx, cy), angle)
            })
            .collect();

        let oval = |rx: f32, ry: f32, ts: Transform| {
            Rect::from_ltrb(-rx, -ry, rx, ry)
                .and_then(PathBuilder::from_oval)
                .and_then(|p| p.transform(ts))
        };
        let mut ring: Vec<Element> = Vec::with_capacity(n);
        for (ts, _) in &placed {
            let (Some(fill), Some(border)) = (
                oval(ew / 2.0, eh / 2.0, *ts),
                oval(ow / 2.0 - BORDER / 2.0, oh / 2.0 - BORDER / 2.0, *ts),
            ) else {
                anyhow::bail!("element_size too small");
            };
            ring.push(Element { fill, border });
        }

        // Normalize so the ring has `padding` on every side.
        let bounds = ring
            .iter()
            .map(|e| e.border.bounds())
            .reduce(|a, b| a.join(&b).unwrap_or(a))
            .expect("at least 3 elements");
        let pad = options.padding + BORDER;
        let shift = Transform::from_translate(pad - bounds.left(), pad - bounds.top());
        for e in &mut ring {
            e.fill = e.fill.clone().transform(shift).expect("finite");
            e.border = e.border.clone().transform(shift).expect("finite");
        }

        Ok(Self {
            intensity: vec![0.0; n],
            size: (bounds.width() + 2.0 * pad, bounds.height() + 2.0 * pad),
            options,
            ring,
            pending: (0.0, 0.0),
            last_tick: None,
        })
    }

    fn add(&mut self, index: i64, value: f32) {
        let n = self.intensity.len() as i64;
        let slot = &mut self.intensity[index.rem_euclid(n) as usize];
        *slot = (*slot + value).min(1.0);
    }
}

impl Widget for MouseMovementWidget {
    fn size(&self) -> (f32, f32) {
        self.size
    }

    fn on_input(&mut self, event: &InputEvent, _now: Instant) -> bool {
        let InputEvent::MouseMove { dx, dy } = *event else {
            return false;
        };
        self.pending.0 += dx;
        self.pending.1 += dy;
        true
    }

    fn tick(&mut self, now: Instant) {
        let dt_ms = self.last_tick.map_or(REFERENCE_FRAME_MS, |t| {
            (now.saturating_duration_since(t).as_secs_f32() * 1000.0).clamp(0.1, 100.0)
        });

        let fade = self.options.fade_factor.powf(dt_ms);
        for v in &mut self.intensity {
            *v /= fade;
            if *v < 0.001 {
                *v = 0.0;
            }
        }

        let (dx, dy) = std::mem::take(&mut self.pending);
        if dx != 0.0 || dy != 0.0 {
            let n = self.intensity.len() as f32;
            let index = ((dy as f32).atan2(dx as f32) / (TAU / n)).round() as i64;
            let speed = 1000.0 * (dx * dx + dy * dy).sqrt() as f32 / dt_ms;
            let delta = (speed * self.options.speed_factor).min(1.0) * (dt_ms / REFERENCE_FRAME_MS);
            self.add(index, delta);
            let mut falloff = self.options.neighbour_fade_factor;
            for i in 1..=self.options.affected_neighbours as i64 {
                self.add(index - i, delta / falloff);
                self.add(index + i, delta / falloff);
                falloff *= self.options.neighbour_fade_factor;
            }
        }

        self.last_tick = self.is_animating(now).then_some(now);
    }

    fn reset(&mut self) {
        self.intensity.iter_mut().for_each(|v| *v = 0.0);
        self.pending = (0.0, 0.0);
        self.last_tick = None;
    }

    fn is_animating(&self, _now: Instant) -> bool {
        self.pending != (0.0, 0.0) || self.intensity.iter().any(|v| *v > 0.0)
    }

    fn draw(&self, canvas: &mut Canvas, origin: (f32, f32), _now: Instant) {
        let s = canvas.scale;
        let ts = Transform::from_row(s, 0.0, 0.0, s, origin.0 * s, origin.1 * s);
        let stroke = Stroke {
            width: (BORDER * s).floor().max(1.0) / s,
            ..Default::default()
        };
        for (element, &v) in self.ring.iter().zip(&self.intensity) {
            if v <= 0.0 {
                continue;
            }
            // The color table has 1/1000 steps.
            let color = gradient::sample(&self.options.gradient, (v * 1000.0).round() / 1000.0);
            let opacity = color.alpha();
            if opacity <= 0.0 {
                continue;
            }
            let mut fill = Paint::default();
            fill.set_color(color);
            canvas
                .highlight()
                .fill_path(&element.fill, &fill, FillRule::Winding, ts, None);
            let mut border = Paint::default();
            border.set_color(Color::from_rgba(1.0, 1.0, 1.0, opacity).unwrap_or(Color::WHITE));
            canvas
                .highlight()
                .stroke_path(&element.border, &border, &stroke, ts, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::snapshot::render_snapshot;
    use std::time::Duration;

    #[test]
    fn moving_right_lights_the_right_element_then_decays() {
        let mut w = MouseMovementWidget::new(MovementOptions::default()).unwrap();
        let t0 = Instant::now();
        w.on_input(&InputEvent::MouseMove { dx: 500.0, dy: 0.0 }, t0);
        w.tick(t0);
        let max = w.intensity.iter().cloned().fold(0.0, f32::max);
        assert_eq!(w.intensity[0], max, "angle 0 = element 0 (right)");
        assert!(
            w.intensity[1] > 0.0 && w.intensity[29] > 0.0,
            "neighbours lit"
        );
        assert!(w.is_animating(t0));

        for i in 1..=300 {
            w.tick(t0 + Duration::from_millis(16 * i));
        }
        assert!(
            !w.is_animating(t0 + Duration::from_secs(5)),
            "fully decayed"
        );
    }

    #[test]
    fn size_is_ring_plus_padding() {
        let w = MouseMovementWidget::new(MovementOptions {
            radius: 100.0,
            ..Default::default()
        })
        .unwrap();
        assert!(w.size.0 > 200.0 && w.size.0 < 320.0, "{:?}", w.size);
    }

    #[test]
    #[ignore = "writes target/snapshots/mouse-movement.png"]
    fn snapshot() {
        let mut w = MouseMovementWidget::new(MovementOptions {
            radius: 120.0,
            ..Default::default()
        })
        .unwrap();
        let t0 = Instant::now();
        for i in 0..8 {
            w.on_input(
                &InputEvent::MouseMove {
                    dx: 900.0,
                    dy: -400.0,
                },
                t0,
            );
            w.tick(t0 + Duration::from_millis(16 * i));
        }
        render_snapshot(&mut w, "mouse-movement", t0 + Duration::from_millis(16 * 8));
    }
}
