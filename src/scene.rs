//! A loaded profile: its widgets, their layout, and the overlay settings.

use std::time::Instant;

use anyhow::{Context, Result};

use crate::input::InputEvent;
use crate::profile::layout::{LayoutTree, arrange};
use crate::profile::{Anchor, Node, Profile};
use crate::widgets::{self, Canvas, Resources, Widget};

pub struct Scene {
    widgets: Vec<Box<dyn Widget>>,
    origins: Vec<(f32, f32)>,
    /// Logical px.
    pub size: (f32, f32),
    pub anchor: Anchor,
    pub offset: (f32, f32),
    pub opacity: f32,
    /// Separate highlight opacity; `None` when it equals `opacity` (one layer is enough).
    pub highlight_opacity: Option<f32>,
    pub scale: f32,
    pub monitor: usize,
}

impl Scene {
    pub fn build(profile: &Profile, res: &Resources) -> Result<Self> {
        let mut widgets = Vec::new();
        let tree = build_tree(&profile.layout, res, &mut widgets)?;
        anyhow::ensure!(!widgets.is_empty(), "layout has no widgets");

        let sizes: Vec<(f32, f32)> = widgets.iter().map(|w| w.size()).collect();
        let (origins, size) = arrange(&tree, &sizes, profile.gap);
        Ok(Self {
            widgets,
            origins,
            size,
            anchor: profile.anchor,
            offset: (profile.offset[0], profile.offset[1]),
            opacity: profile.opacity,
            highlight_opacity: profile
                .highlight_opacity
                .filter(|h| (h - profile.opacity).abs() > f32::EPSILON),
            scale: profile.scale,
            monitor: profile.monitor,
        })
    }

    /// True if any widget needs a redraw.
    pub fn on_input(&mut self, event: &InputEvent, now: Instant) -> bool {
        // No short-circuit: every widget must see every event.
        self.widgets
            .iter_mut()
            .fold(false, |dirty, w| w.on_input(event, now) | dirty)
    }

    /// Per-frame simulation step for every widget.
    pub fn tick(&mut self, now: Instant) {
        self.widgets.iter_mut().for_each(|w| w.tick(now));
    }

    pub fn uses_gamepads(&self) -> bool {
        self.widgets.iter().any(|w| w.uses_gamepads())
    }

    pub fn reset(&mut self) {
        self.widgets.iter_mut().for_each(|w| w.reset());
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        self.widgets.iter().any(|w| w.is_animating(now))
    }

    pub fn draw(&self, canvas: &mut Canvas, now: Instant) {
        for (widget, origin) in self.widgets.iter().zip(&self.origins) {
            widget.draw(canvas, *origin, now);
        }
    }
}

fn build_tree(
    node: &Node,
    res: &Resources,
    widgets: &mut Vec<Box<dyn Widget>>,
) -> Result<LayoutTree> {
    Ok(match node {
        Node::Widget { widget, options } => {
            widgets.push(widgets::build(widget, options, res)?);
            LayoutTree::Leaf(widgets.len() - 1)
        }
        Node::Row { row, align } => LayoutTree::Row(build_children(row, res, widgets)?, *align),
        Node::Column { column, align } => {
            LayoutTree::Column(build_children(column, res, widgets)?, *align)
        }
    })
}

fn build_children(
    nodes: &[Node],
    res: &Resources,
    widgets: &mut Vec<Box<dyn Widget>>,
) -> Result<Vec<LayoutTree>> {
    nodes
        .iter()
        .enumerate()
        .map(|(i, n)| build_tree(n, res, widgets).with_context(|| format!("layout item {i}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile;

    #[test]
    fn shipped_profiles_build() {
        let res = Resources::load().unwrap();
        let files = profile::list_profiles(std::path::Path::new("assets/profiles"));
        assert!(files.len() >= 2, "default, gamepad");
        for path in files {
            let p = Profile::load(&path).unwrap();
            let scene = Scene::build(&p, &res).unwrap_or_else(|e| panic!("{path:?}: {e:#}"));
            assert!(scene.size.0 > 0.0 && scene.size.1 > 0.0);
        }
        let starter = Profile::parse(profile::STARTER_PROFILE).unwrap();
        Scene::build(&starter, &res).expect("starter profile builds");
    }
}

#[cfg(test)]
mod perf {
    use super::*;
    use crate::render::glow::GlowCache;
    use crate::render::{alpha_u8, over};
    use std::time::Duration;
    use tiny_skia::{Color, Pixmap};

    /// Draw time per widget type at 125% DPI, idle and with keys held: `cargo test widget_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "benchmark"]
    fn widget_cost() {
        let res = Resources::load().unwrap();
        let widgets = [
            ("keyboard", "preset = \"default\""),
            ("keyboard", "preset = \"full\""),
            ("mouse", ""),
            ("mouse-movement", ""),
            ("mouse-speed", ""),
            ("mouse-flick", ""),
            ("gamepad", ""),
            ("gamepad", "style = \"dualsense\""),
        ];
        for (name, options) in widgets {
            let text = format!("[layout]\nwidget = \"{name}\"\n{options}\n");
            let mut scene = Scene::build(&Profile::parse(&text).unwrap(), &res).unwrap();
            let scale = 1.25;
            let (w, h) = scene.size;
            let mut pixmap =
                Pixmap::new((w * scale).ceil() as u32, (h * scale).ceil() as u32).unwrap();
            let mut glow = GlowCache::default();
            let start = Instant::now();
            let mut time = |scene: &mut Scene| {
                let frames = 60;
                let t = Instant::now();
                for i in 0..frames {
                    let now = start + Duration::from_millis(16 * i);
                    scene.tick(now);
                    pixmap.fill(Color::TRANSPARENT);
                    scene.draw(
                        &mut Canvas {
                            pixmap: &mut pixmap,
                            highlight: None,
                            scale,
                            glow: &mut glow,
                        },
                        now,
                    );
                }
                t.elapsed().as_secs_f64() * 1000.0 / frames as f64
            };
            let idle = time(&mut scene);
            crate::ui::feed_sample(&mut scene, start);
            let busy = time(&mut scene);
            println!("{name:<15} {options:<22} idle {idle:6.2} ms  busy {busy:6.2} ms");
        }
    }

    /// Per-frame cost like the app pays it: `cargo test --release frame_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "benchmark"]
    fn frame_cost() {
        let res = Resources::load().unwrap();
        let dpi = 1.25;
        for path in crate::profile::list_profiles(std::path::Path::new("assets/profiles")) {
            for split in [false, true] {
                let mut p = Profile::load(&path).unwrap();
                p.highlight_opacity = split.then_some(1.0).filter(|h| *h != p.opacity);
                let mut scene = Scene::build(&p, &res).unwrap();
                let scale = dpi * scene.scale;
                let (w, h) = scene.size;
                let (pw, ph) = ((w * scale).ceil() as u32, (h * scale).ceil() as u32);
                let mut base = Pixmap::new(pw, ph).unwrap();
                let mut layer = Pixmap::new(pw, ph).unwrap();
                let mut bgra = vec![0u8; (pw * ph * 4) as usize];
                let mut glow = GlowCache::default();
                let start = Instant::now();
                crate::ui::feed_sample(&mut scene, start);

                let frames = 120;
                let (mut draw, mut copy) = (Duration::ZERO, Duration::ZERO);
                for i in 0..frames {
                    let now = start + Duration::from_millis(16 * i);
                    // Keep the ring and speed busy, like a moving mouse.
                    scene.on_input(
                        &crate::input::InputEvent::MouseMove {
                            dx: 40.0,
                            dy: -12.0,
                        },
                        now,
                    );
                    let t = Instant::now();
                    scene.tick(now);
                    base.fill(Color::TRANSPARENT);
                    let split_layer = scene.highlight_opacity.map(|_| {
                        layer.fill(Color::TRANSPARENT);
                        &mut layer
                    });
                    scene.draw(
                        &mut Canvas {
                            pixmap: &mut base,
                            highlight: split_layer,
                            scale,
                            glow: &mut glow,
                        },
                        now,
                    );
                    draw += t.elapsed();
                    // Same work as LayeredSurface::present: BGRA copy, blending the highlight layer in.
                    let t = Instant::now();
                    let dst = bgra.as_chunks_mut::<4>().0.iter_mut();
                    let src = base.data().as_chunks::<4>().0;
                    match scene.highlight_opacity {
                        Some(hl) => {
                            let (bo, ho) = (alpha_u8(scene.opacity), alpha_u8(hl));
                            for ((d, s), h) in dst.zip(src).zip(layer.data().as_chunks::<4>().0) {
                                let [r, g, b, a] = over(*s, bo, *h, ho);
                                *d = [b, g, r, a];
                            }
                        }
                        None => {
                            for (d, s) in dst.zip(src) {
                                *d = [s[2], s[1], s[0], s[3]];
                            }
                        }
                    }
                    copy += t.elapsed();
                }
                let ms = |d: Duration| d.as_secs_f64() * 1000.0 / frames as f64;
                println!(
                    "{:<14} split={:<5} {pw}x{ph}  draw {:6.2} ms  copy {:5.2} ms",
                    crate::profile::display_name(&path),
                    split,
                    ms(draw),
                    ms(copy)
                );
            }
        }
    }
}

#[cfg(test)]
mod live_scene {
    use super::*;
    use crate::render::glow::GlowCache;
    use std::time::Duration;
    use tiny_skia::{Color, Pixmap};

    /// Full fps scene as the app draws it, after a wheel event; counts blue pixels.
    #[test]
    #[ignore = "writes target/snapshots/scene-fps-scroll.png"]
    fn fps_scene_scroll() {
        let res = Resources::load().unwrap();
        let p = Profile::load(std::path::Path::new("assets/profiles/default.toml")).unwrap();
        let mut scene = Scene::build(&p, &res).unwrap();
        let scale = 1.25 * scene.scale;
        let now = Instant::now();
        let ev = InputEvent::MouseWheel { x: 0.0, y: -1.0 };
        assert!(scene.on_input(&ev, now - Duration::from_millis(30)));
        let (w, h) = scene.size;
        let mut pixmap = Pixmap::new((w * scale).ceil() as u32, (h * scale).ceil() as u32).unwrap();
        pixmap.fill(Color::BLACK);
        let mut glow = GlowCache::default();
        scene.tick(now);
        scene.draw(
            &mut Canvas {
                pixmap: &mut pixmap,
                highlight: None,
                scale,
                glow: &mut glow,
            },
            now,
        );
        let blue = pixmap
            .pixels()
            .iter()
            .filter(|p| p.blue() > 200 && p.red() < 100)
            .count();
        std::fs::create_dir_all("target/snapshots").unwrap();
        pixmap
            .save_png("target/snapshots/scene-fps-scroll.png")
            .unwrap();
        println!("blue pixels: {blue}");
        assert!(blue > 50, "scroll highlight missing in full scene");
    }
}
