//! Test helper: renders a widget on black to `target/snapshots/<name>.png` (`SNAPSHOT_SCALE` env, default 1).

use std::time::Instant;

use tiny_skia::{Color, Pixmap};

use super::{Canvas, Widget};
use crate::render::glow::GlowCache;

pub fn render_snapshot(widget: &mut dyn Widget, name: &str, now: Instant) {
    let scale: f32 = std::env::var("SNAPSHOT_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let (w, h) = widget.size();
    let mut pixmap = Pixmap::new((w * scale).ceil() as u32, (h * scale).ceil() as u32).unwrap();
    pixmap.fill(Color::BLACK);
    let mut glow = GlowCache::default();
    let mut canvas = Canvas {
        pixmap: &mut pixmap,
        highlight: None,
        scale,
        glow: &mut glow,
    };
    widget.tick(now);
    widget.draw(&mut canvas, (0.0, 0.0), now);

    std::fs::create_dir_all("target/snapshots").unwrap();
    pixmap
        .save_png(format!("target/snapshots/{name}.png"))
        .unwrap();
}
