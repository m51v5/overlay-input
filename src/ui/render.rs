//! Offscreen render of a scene for the Profiles page's mock screen, with optional sample input.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tiny_skia::Pixmap;

use crate::input::gamepad::{GamepadState, PadId, PadSource, buttons};
use crate::input::{InputEvent, KeyCode, MouseButton, PadEvent};
use crate::render::composite_layers;
use crate::render::glow::GlowCache;
use crate::scene::Scene;
use crate::widgets::Canvas;

/// Simulated frames of sample input before the snapshot.
const SAMPLE_FRAMES: u32 = 8;
const FRAME: Duration = Duration::from_millis(16);

/// Draws `scene` at `dpi` (physical px per logical px, before the profile's own scale).
/// Opacity is left to the caller, except with a separate highlight opacity: then both are baked in.
pub fn render_pixmap(scene: &mut Scene, dpi: f32, sample: bool) -> Result<Pixmap> {
    let now = Instant::now();
    if sample {
        feed_sample(scene, now);
    }
    let scale = dpi * scene.scale;
    let (w, h) = scene.size;
    let alloc = || {
        Pixmap::new((w * scale).ceil() as u32, (h * scale).ceil() as u32)
            .context("layout too large to render")
    };
    let mut pixmap = alloc()?;
    let mut highlight = match scene.highlight_opacity {
        Some(_) => Some(alloc()?),
        None => None,
    };
    scene.tick(now);
    scene.draw(
        &mut Canvas {
            pixmap: &mut pixmap,
            highlight: highlight.as_mut(),
            scale,
            glow: &mut GlowCache::default(),
        },
        now,
    );
    if let (Some(layer), Some(highlight_opacity)) = (&highlight, scene.highlight_opacity) {
        let mut frame = alloc()?;
        composite_layers(&mut frame, &pixmap, scene.opacity, layer, highlight_opacity);
        pixmap = frame;
    }
    Ok(pixmap)
}

/// A few held keys and buttons, a scroll, a fast up-right mouse sweep and a busy pad, played as frames up to `now`.
pub fn feed_sample(scene: &mut Scene, now: Instant) {
    let start = now - FRAME * SAMPLE_FRAMES;
    let mut events = vec![
        InputEvent::Key {
            key: KeyCode::KeyW,
            pressed: true,
        },
        InputEvent::Key {
            key: KeyCode::KeyD,
            pressed: true,
        },
        InputEvent::Key {
            key: KeyCode::ShiftLeft,
            pressed: true,
        },
        InputEvent::MouseButton {
            button: MouseButton::Left,
            pressed: true,
        },
    ];
    let pad = GamepadState {
        buttons: buttons::A | buttons::RB,
        left_stick: (0.0, 0.85),
        right_stick: (0.0, 0.0),
        left_trigger: 0.0,
        right_trigger: 0.7,
    };
    // Every pad a widget could be set to listen to; each widget locks onto the first it accepts.
    let pads = (0..4)
        .map(|index| (PadSource::XInput, index))
        .chain([(PadSource::DualSense, 0)]);
    events.extend(pads.map(|(source, index)| {
        InputEvent::Gamepad(PadEvent {
            pad: PadId { source, index },
            state: pad,
            connected: true,
        })
    }));
    for event in &events {
        scene.on_input(event, start);
    }

    for frame in 0..SAMPLE_FRAMES {
        let t = start + FRAME * frame;
        scene.on_input(
            &InputEvent::MouseMove {
                dx: 220.0,
                dy: -70.0,
            },
            t,
        );
        if frame == SAMPLE_FRAMES - 2 {
            scene.on_input(&InputEvent::MouseWheel { x: 0.0, y: -1.0 }, t);
        }
        scene.tick(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;
    use crate::widgets::Resources;

    fn fps_scene() -> Scene {
        let res = Resources::load().unwrap();
        let p = Profile::load(std::path::Path::new("assets/profiles/default.toml")).unwrap();
        Scene::build(&p, &res).unwrap()
    }

    #[test]
    fn renders_at_dpi_and_profile_scale() {
        let mut scene = fps_scene();
        let pixmap = render_pixmap(&mut scene, 1.5, false).unwrap();
        let expected = (scene.size.0 * 1.5 * scene.scale).ceil() as u32;
        assert_eq!(pixmap.width(), expected);
    }

    #[test]
    fn sample_input_lights_things_up() {
        let lit = |sample| {
            render_pixmap(&mut fps_scene(), 1.0, sample)
                .unwrap()
                .pixels()
                .iter()
                .filter(|p| p.alpha() > 0 && u16::from(p.green()) > u16::from(p.red()) + 40)
                .count()
        };
        let (idle, busy) = (lit(false), lit(true));
        assert!(busy > idle + 100, "idle {idle}, sample {busy}");
    }

    #[test]
    #[ignore = "writes target/snapshots/render-*.png"]
    fn render_snapshots() {
        std::fs::create_dir_all("target/snapshots").unwrap();
        let res = Resources::load().unwrap();
        for name in ["default", "gamepad"] {
            let path = format!("assets/profiles/{name}.toml");
            let p = Profile::load(std::path::Path::new(&path)).unwrap();
            let pixmap = render_pixmap(&mut Scene::build(&p, &res).unwrap(), 1.0, true).unwrap();
            pixmap
                .save_png(format!("target/snapshots/render-{name}.png"))
                .unwrap();
        }
    }
}
