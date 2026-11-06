//! Overlay widgets.

pub mod gamepad;
pub mod keyboard;
pub mod mouse;
pub mod mouse_flick;
pub mod mouse_movement;
pub mod mouse_speed;
#[cfg(test)]
mod snapshot;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use tiny_skia::{Color, Mask, Pixmap, Transform};

use crate::input::InputEvent;
use crate::render::glow::GlowCache;
use crate::render::mul255;
use crate::render::svg::SvgShape;
use crate::render::text::Font;

pub const KNOWN_WIDGETS: &str =
    "keyboard, mouse, mouse-movement, mouse-speed, mouse-flick, gamepad";

/// Shared assets widgets are built from.
pub struct Resources {
    /// Key labels: Calibri, with symbol fallbacks for ⌫ ⏎ ⏸ and arrows.
    pub calibri: Rc<Font>,
    /// mouse-speed / mouse-flick text.
    pub verdana: Rc<Font>,
}

impl Resources {
    pub fn load() -> Result<Self> {
        Ok(Self {
            calibri: Rc::new(Font::load_system(&[
                "calibri.ttf",
                "seguisym.ttf",
                "segoeui.ttf",
                "arial.ttf",
            ])?),
            verdana: Rc::new(Font::load_system(&[
                "verdana.ttf",
                "segoeui.ttf",
                "arial.ttf",
            ])?),
        })
    }
}

/// Creates a widget from its profile name and options table.
pub fn build(name: &str, options: &toml::Table, res: &Resources) -> Result<Box<dyn Widget>> {
    Ok(match name {
        "keyboard" => Box::new(keyboard::KeyboardWidget::new(
            &res.calibri,
            parse_options(name, options)?,
        )?),
        "mouse" => Box::new(mouse::MouseWidget::new(parse_options(name, options)?)?),
        "mouse-movement" => Box::new(mouse_movement::MouseMovementWidget::new(parse_options(
            name, options,
        )?)?),
        "mouse-speed" => Box::new(mouse_speed::MouseSpeedWidget::new(
            &res.verdana,
            parse_options(name, options)?,
        )?),
        "mouse-flick" => Box::new(mouse_flick::MouseFlickWidget::new(
            &res.verdana,
            parse_options(name, options)?,
        )?),
        "gamepad" => Box::new(gamepad::GamepadWidget::new(parse_options(name, options)?)?),
        other => anyhow::bail!("unknown widget {other:?} (known: {KNOWN_WIDGETS})"),
    })
}

fn parse_options<T: DeserializeOwned>(name: &str, options: &toml::Table) -> Result<T> {
    toml::Value::Table(options.clone())
        .try_into()
        .with_context(|| format!("options for widget {name:?}"))
}

/// Physical-pixel target plus shared caches; widgets draw in logical px times `scale`.
pub struct Canvas<'a> {
    /// Base layer: outlines, labels, everything that is always shown.
    pub pixmap: &'a mut Pixmap,
    /// Pressed fills, glows and other input feedback, when the profile gives them their own opacity.
    pub highlight: Option<&'a mut Pixmap>,
    pub scale: f32,
    pub glow: &'a mut GlowCache,
}

impl Canvas<'_> {
    /// Layer for input feedback (the base layer unless highlights are split off).
    pub fn highlight(&mut self) -> &mut Pixmap {
        match &mut self.highlight {
            Some(layer) => layer,
            None => self.pixmap,
        }
    }

    /// Highlight layer and glow cache, borrowed together.
    pub fn highlight_and_glow(&mut self) -> (&mut Pixmap, &mut GlowCache) {
        let layer = match &mut self.highlight {
            Some(layer) => &mut **layer,
            None => &mut *self.pixmap,
        };
        (layer, &mut *self.glow)
    }

    /// SVG part: an overridden (pressed) fill goes to the highlight layer, its own fill and stroke to the base.
    pub fn draw_svg(
        &mut self,
        shape: &SvgShape,
        ts: Transform,
        fill: Option<Color>,
        mask: Option<&Mask>,
    ) {
        match fill {
            Some(color) => shape.draw_fill(self.highlight(), ts, 1.0, Some(color), mask),
            None => shape.draw_fill(self.pixmap, ts, 1.0, None, mask),
        }
        shape.draw_stroke(self.pixmap, ts, 1.0, mask);
    }
}

/// Pixels that only change with the scale (outlines, labels): drawn once, then blitted every frame.
#[derive(Default)]
pub struct StaticLayer {
    /// Most recent first; several, since the screen, the overlay window and the app window draw at different scales.
    cache: RefCell<Vec<(LayerKey, Layer)>>,
}

/// Scales kept per layer.
const LAYER_CACHE: usize = 3;

/// Scale, sub-pixel x and sub-pixel y (as bits) a layer was rendered for.
type LayerKey = (u32, u32, u32);

/// A rendered layer and its non-empty pixel runs: outlines cover a few percent of the box,
/// and blending only those is much cheaper than a full `draw_pixmap`.
struct Layer {
    pixmap: Pixmap,
    /// (row, first column, end column) of each run of visible pixels.
    runs: Vec<(u32, u32, u32)>,
}

impl Layer {
    fn new(pixmap: Pixmap) -> Self {
        let width = pixmap.width() as usize;
        let mut runs = Vec::new();
        for (row, pixels) in pixmap.pixels().chunks_exact(width).enumerate() {
            let mut x = 0;
            while x < width {
                if pixels[x].alpha() == 0 {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < width && pixels[x].alpha() != 0 {
                    x += 1;
                }
                runs.push((row as u32, start as u32, x as u32));
            }
        }
        Self { pixmap, runs }
    }

    /// Premultiplied source-over of the runs onto `target` with the layer's top-left at `(x, y)`.
    fn blend_onto(&self, target: &mut Pixmap, x: i32, y: i32) {
        let (tw, th) = (target.width() as i32, target.height() as i32);
        let width = self.pixmap.width() as usize;
        let src = self.pixmap.data();
        let dst = target.data_mut();
        for &(row, start, end) in &self.runs {
            let ty = y + row as i32;
            let from = (x + start as i32).max(0);
            let to = (x + end as i32).min(tw);
            if ty < 0 || ty >= th || from >= to {
                continue;
            }
            for tx in from..to {
                let s = (row as usize * width + (tx - x) as usize) * 4;
                let d = (ty as usize * tw as usize + tx as usize) * 4;
                let alpha = src[s + 3] as u32;
                if alpha == 255 {
                    dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
                } else {
                    let rest = 255 - alpha;
                    for i in 0..4 {
                        dst[d + i] = (src[s + i] as u32 + mul255(dst[d + i] as u32, rest)) as u8;
                    }
                }
            }
        }
    }
}

impl StaticLayer {
    /// Blits the layer at logical `origin`; `paint` (re)builds it, drawing with the transform it gets.
    pub fn blit(
        &self,
        target: &mut Pixmap,
        size: (f32, f32),
        origin: (f32, f32),
        scale: f32,
        paint: impl FnOnce(&mut Pixmap, Transform),
    ) {
        let (x, y) = (origin.0 * scale, origin.1 * scale);
        let key = (scale.to_bits(), x.fract().to_bits(), y.fract().to_bits());
        let mut cache = self.cache.borrow_mut();
        match cache.iter().position(|(k, _)| *k == key) {
            Some(i) => cache[..=i].rotate_right(1),
            None => {
                // One spare pixel for the sub-pixel shift.
                let w = (size.0 * scale).ceil() as u32 + 1;
                let h = (size.1 * scale).ceil() as u32 + 1;
                let Some(mut pixmap) = Pixmap::new(w, h) else {
                    return;
                };
                paint(
                    &mut pixmap,
                    Transform::from_row(scale, 0.0, 0.0, scale, x.fract(), y.fract()),
                );
                cache.truncate(LAYER_CACHE - 1);
                cache.insert(0, (key, Layer::new(pixmap)));
            }
        }
        cache[0]
            .1
            .blend_onto(target, x.floor() as i32, y.floor() as i32);
    }
}

pub trait Widget {
    /// Natural size in logical px, including room for glow.
    fn size(&self) -> (f32, f32);
    /// Reacts to input; true if the widget needs a redraw.
    fn on_input(&mut self, event: &InputEvent, now: Instant) -> bool;
    /// Per-frame simulation step before `draw` (decay, sampling windows).
    fn tick(&mut self, _now: Instant) {}
    /// True if this widget needs the gamepad reader threads.
    fn uses_gamepads(&self) -> bool {
        false
    }
    /// Clears all held/animating state (used when capture stops).
    fn reset(&mut self);
    /// True while an animation (fade, decay) still needs frames.
    fn is_animating(&self, now: Instant) -> bool;
    /// Draws with top-left at logical `origin`.
    fn draw(&self, canvas: &mut Canvas, origin: (f32, f32), now: Instant);
}

#[cfg(test)]
mod perf {
    use super::*;

    /// Cost of blitting a cached layer: `cargo test blit_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "benchmark"]
    fn blit_cost() {
        for (w, h) in [(650u32, 600u32), (1300, 450)] {
            let mut target = Pixmap::new(w, h).unwrap();
            let layer = StaticLayer::default();
            let size = (w as f32 / 1.25, h as f32 / 1.25);
            for (label, filled) in [("empty", false), ("drawn", true)] {
                let layer_ref = &layer;
                let t = std::time::Instant::now();
                for _ in 0..20 {
                    layer_ref.blit(&mut target, size, (0.0, 0.0), 1.25, |p, _| {
                        if filled {
                            p.fill(Color::from_rgba8(255, 255, 255, 40));
                        }
                    });
                }
                println!(
                    "{w}x{h} {label}: {:.2} ms per blit",
                    t.elapsed().as_secs_f64() * 1000.0 / 20.0
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_layer_keeps_a_few_scales() {
        let layer = StaticLayer::default();
        let mut target = Pixmap::new(64, 64).unwrap();
        let mut paints = 0;
        // Two outputs taking turns: each scale is drawn once.
        for _ in 0..5 {
            for scale in [1.25, 2.0] {
                layer.blit(&mut target, (20.0, 20.0), (0.0, 0.0), scale, |_, _| {
                    paints += 1
                });
            }
        }
        assert_eq!(paints, 2);
        // Past the limit the oldest scale goes.
        for scale in [3.0, 4.0, 1.25] {
            layer.blit(&mut target, (10.0, 10.0), (0.0, 0.0), scale, |_, _| {
                paints += 1
            });
        }
        assert_eq!(paints, 5, "1.25 was dropped for 3.0 and 4.0");
    }
}
