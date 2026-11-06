//! Mouse widget, drawn from `mouse.svg`: held button turns green with a white glow, fades on release;
//! wheel arrows slide and fade. Addition: scrolling lights the wheel half in the scroll direction.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use anyhow::Result;
use tiny_skia::{Color, FillRule, Mask, Paint, PathBuilder, Rect, Transform};

use super::{Canvas, Widget};
use crate::input::{InputEvent, MouseButton};
use crate::render::glow::{GlowCache, Sprite};
use crate::render::rgb;
use crate::render::svg::{self, SvgShape};

const SVG: &str = include_str!("../../assets/widgets/mouse.svg");
const MARGIN: f32 = 10.0; // div.mouse margin
const WIDTH: f32 = 200.0; // object width
const PRESSED: u32 = 0x2DA026;
const PRESSED_RGB: [u8; 3] = [0x2D, 0xA0, 0x26];
const RELEASED: u32 = 0x96CF13;
const RELEASE_FADE: Duration = Duration::from_millis(250);
const SCROLL_TIME: Duration = Duration::from_millis(250);
const SCROLL_SHIFT: f32 = 10.0; // svg units
const GLOW_STD_DEV: f32 = 10.0; // svg units

/// Element ids in mouse.svg; Mouse4/5 = X1/X2.
const BUTTONS: [(&str, MouseButton); 5] = [
    ("Mouse1", MouseButton::Left),
    ("Mouse2", MouseButton::Right),
    ("Mouse3", MouseButton::Middle),
    ("Mouse4", MouseButton::Back),
    ("Mouse5", MouseButton::Forward),
];
const MIDDLE: usize = 2;
const SCROLL_UP: usize = 0;
const SCROLL_DOWN: usize = 1;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct MouseOptions {
    /// Light the wheel half in the scroll direction while scrolling, then fade.
    pub scroll_highlight: bool,
    /// Scroll highlight color; differs from the click green so the two never look alike.
    #[schemars(extend("format" = "color"))]
    pub scroll_color: [u8; 3],
}

impl Default for MouseOptions {
    fn default() -> Self {
        Self {
            scroll_highlight: true,
            scroll_color: [35, 173, 255], // keyboard glow blue
        }
    }
}

type MaskKey = (u32, u32, [u32; 6]);

enum Role {
    Static,
    Button(usize),
    Scroll(usize),
}

#[derive(Clone, Copy)]
enum ButtonState {
    Idle,
    Held,
    Released(Instant),
}

pub struct MouseWidget {
    shapes: Vec<(SvgShape, Role)>,
    buttons: [ButtonState; 5],
    scroll: [Option<Instant>; 2],
    scroll_highlight: bool,
    scroll_color: [u8; 3],
    /// Wheel top/bottom halves as clip masks, cached per target size and transform.
    half_masks: RefCell<Option<(MaskKey, [Option<Mask>; 2])>>,
    size: (f32, f32),
    /// SVG user unit in logical px.
    unit: f32,
}

impl MouseWidget {
    pub fn new(options: MouseOptions) -> Result<Self> {
        let image = svg::load(SVG, WIDTH, (MARGIN, MARGIN))?;
        let shapes = image
            .shapes
            .into_iter()
            .map(|shape| {
                let role = if shape.has_id("MouseScrollUp") {
                    Role::Scroll(SCROLL_UP)
                } else if shape.has_id("MouseScrollDown") {
                    Role::Scroll(SCROLL_DOWN)
                } else if let Some(i) = BUTTONS.iter().position(|(id, _)| shape.has_id(id)) {
                    Role::Button(i)
                } else {
                    Role::Static
                };
                (shape, role)
            })
            .collect();
        Ok(Self {
            shapes,
            buttons: [ButtonState::Idle; 5],
            scroll: [None; 2],
            scroll_highlight: options.scroll_highlight,
            scroll_color: options.scroll_color,
            half_masks: RefCell::new(None),
            size: (WIDTH + 2.0 * MARGIN, image.size.1 + 2.0 * MARGIN),
            unit: image.unit,
        })
    }

    /// Scroll highlight strength for a direction: full while scrolling, then fades out.
    fn scroll_alpha(&self, dir: usize, now: Instant) -> f32 {
        let Some(t) = self.scroll[dir].filter(|_| self.scroll_highlight) else {
            return 0.0;
        };
        let elapsed = now.saturating_duration_since(t);
        if elapsed < SCROLL_TIME {
            1.0
        } else {
            1.0 - progress(now, t + SCROLL_TIME, RELEASE_FADE)
        }
    }

    /// Fills the wheel half for each active scroll direction (top = up, bottom = down).
    fn draw_scroll_halves(
        &self,
        canvas: &mut Canvas,
        wheel: &SvgShape,
        ts: Transform,
        now: Instant,
    ) {
        let alphas = [
            self.scroll_alpha(SCROLL_UP, now),
            self.scroll_alpha(SCROLL_DOWN, now),
        ];
        if alphas.iter().all(|a| *a <= 0.0) {
            return;
        }
        let (w, h) = (canvas.pixmap.width(), canvas.pixmap.height());
        let key: MaskKey = (
            w,
            h,
            [ts.sx, ts.kx, ts.ky, ts.sy, ts.tx, ts.ty].map(f32::to_bits),
        );
        let mut cache = self.half_masks.borrow_mut();
        if cache.as_ref().is_none_or(|(k, _)| *k != key) {
            let b = wheel.path.bounds();
            let half = |top: f32| {
                let rect =
                    Rect::from_xywh(b.left() - 1.0, top, b.width() + 2.0, b.height() / 2.0 + 1.0)?;
                let mut mask = Mask::new(w, h)?;
                mask.fill_path(&PathBuilder::from_rect(rect), FillRule::Winding, true, ts);
                Some(mask)
            };
            let mid = b.top() + b.height() / 2.0;
            *cache = Some((key, [half(b.top() - 1.0), half(mid)]));
        }
        let Some((_, masks)) = cache.as_ref() else {
            return;
        };
        let [r, g, bl] = self.scroll_color;
        for (dir, alpha) in alphas.into_iter().enumerate() {
            if let (true, Some(mask)) = (alpha > 0.0, &masks[dir]) {
                let mut paint = Paint::default();
                paint.set_color(Color::from_rgba8(r, g, bl, (alpha * 255.0).round() as u8));
                canvas.highlight().fill_path(
                    &wheel.path,
                    &paint,
                    FillRule::Winding,
                    ts,
                    Some(mask),
                );
            }
        }
    }

    fn glow_sprite(&self, button: usize, scale: f32) -> Sprite {
        let parts: Vec<&SvgShape> = self
            .shapes
            .iter()
            .filter(|(_, r)| matches!(r, Role::Button(b) if *b == button))
            .map(|(s, _)| s)
            .collect();
        svg::glow_sprite(&parts, PRESSED_RGB, GLOW_STD_DEV, self.unit, scale)
    }
}

impl Widget for MouseWidget {
    fn size(&self) -> (f32, f32) {
        self.size
    }

    fn on_input(&mut self, event: &InputEvent, now: Instant) -> bool {
        match *event {
            InputEvent::MouseButton { button, pressed } => {
                let Some(i) = BUTTONS.iter().position(|(_, b)| *b == button) else {
                    return false;
                };
                self.buttons[i] = match (pressed, self.buttons[i]) {
                    (true, _) => ButtonState::Held,
                    (false, ButtonState::Held) => ButtonState::Released(now),
                    (false, state) => state,
                };
                true
            }
            InputEvent::MouseWheel { y, .. } if y != 0.0 => {
                self.scroll[if y > 0.0 { SCROLL_UP } else { SCROLL_DOWN }] = Some(now);
                true
            }
            _ => false,
        }
    }

    fn reset(&mut self) {
        self.buttons = [ButtonState::Idle; 5];
        self.scroll = [None; 2];
    }

    fn is_animating(&self, now: Instant) -> bool {
        let fading = self.buttons.iter().any(
            |b| matches!(b, ButtonState::Released(t) if now.saturating_duration_since(*t) < RELEASE_FADE),
        );
        let scrolling = self
            .scroll
            .iter()
            .flatten()
            .any(|t| now.saturating_duration_since(*t) < SCROLL_TIME);
        let wheel_lit = (0..2).any(|dir| self.scroll_alpha(dir, now) > 0.0);
        fading || scrolling || wheel_lit
    }

    fn draw(&self, canvas: &mut Canvas, origin: (f32, f32), now: Instant) {
        let s = canvas.scale;
        let ts = Transform::from_row(s, 0.0, 0.0, s, origin.0 * s, origin.1 * s);

        for (shape, role) in &self.shapes {
            match *role {
                Role::Static => shape.draw(canvas.pixmap, ts, 1.0, None, None),
                Role::Button(b) => {
                    if b == MIDDLE {
                        self.draw_scroll_halves(canvas, shape, ts, now);
                    }
                    let fill = match self.buttons[b] {
                        ButtonState::Idle => None,
                        ButtonState::Held => {
                            let (layer, glow) = canvas.highlight_and_glow();
                            let sprite = glow
                                .sprite(&format!("mouse-glow-{b}"), s, || self.glow_sprite(b, s));
                            GlowCache::blit(layer, sprite, origin.0 * s, origin.1 * s, 1.0);
                            Some(rgb(PRESSED, 1.0))
                        }
                        ButtonState::Released(t) => {
                            let p = progress(now, t, RELEASE_FADE);
                            (p < 1.0).then(|| rgb(RELEASED, 1.0 - p))
                        }
                    };
                    canvas.draw_svg(shape, ts, fill, None);
                }
                Role::Scroll(dir) => {
                    let Some(t) = self.scroll[dir] else { continue };
                    let p = progress(now, t, SCROLL_TIME);
                    if p < 1.0 {
                        let sign = if dir == SCROLL_UP { -1.0 } else { 1.0 };
                        let moved = ts.pre_translate(0.0, sign * SCROLL_SHIFT * p * self.unit);
                        shape.draw(canvas.highlight(), moved, 1.0 - p, None, None);
                    }
                }
            }
        }
    }
}

/// Linear 0..=1 progress of an animation that started at `start`.
fn progress(now: Instant, start: Instant, duration: Duration) -> f32 {
    (now.saturating_duration_since(start).as_secs_f32() / duration.as_secs_f32()).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::snapshot::render_snapshot;

    #[test]
    fn svg_ids_are_found() {
        let widget = MouseWidget::new(MouseOptions::default()).unwrap();
        for b in 0..BUTTONS.len() {
            assert!(
                widget
                    .shapes
                    .iter()
                    .any(|(_, r)| matches!(r, Role::Button(i) if *i == b)),
                "button {b}"
            );
        }
        assert_eq!(
            widget
                .shapes
                .iter()
                .filter(|(_, r)| matches!(r, Role::Scroll(_)))
                .count(),
            6
        );
        assert_eq!(widget.size, (220.0, 300.0));
    }

    #[test]
    fn scroll_lights_its_half_not_the_click_state() {
        let mut widget = MouseWidget::new(MouseOptions::default()).unwrap();
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        widget.on_input(&InputEvent::MouseWheel { x: 0.0, y: -1.0 }, t0);
        assert!(
            matches!(widget.buttons[MIDDLE], ButtonState::Idle),
            "scroll is not a click"
        );
        assert_eq!(widget.scroll_alpha(SCROLL_DOWN, at(100)), 1.0);
        assert_eq!(widget.scroll_alpha(SCROLL_UP, at(100)), 0.0);
        let fading = widget.scroll_alpha(SCROLL_DOWN, at(375));
        assert!(fading > 0.0 && fading < 1.0, "{fading}");
        assert!(widget.is_animating(at(400)));
        assert!(!widget.is_animating(at(500)));

        let off = MouseWidget::new(MouseOptions {
            scroll_highlight: false,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(off.scroll_alpha(SCROLL_DOWN, t0), 0.0);
    }

    #[test]
    fn wheel_animates_then_stops() {
        let mut widget = MouseWidget::new(MouseOptions::default()).unwrap();
        let t0 = Instant::now();
        assert!(widget.on_input(&InputEvent::MouseWheel { x: 0.0, y: 1.0 }, t0));
        assert!(widget.is_animating(t0 + Duration::from_millis(100)));
        assert!(!widget.is_animating(t0 + SCROLL_TIME + RELEASE_FADE));
    }

    #[test]
    #[ignore = "writes target/snapshots/mouse.png"]
    fn snapshot() {
        let mut widget = MouseWidget::new(MouseOptions::default()).unwrap();
        let now = Instant::now();
        let press = |button, pressed| InputEvent::MouseButton { button, pressed };
        widget.on_input(&press(MouseButton::Left, true), now);
        widget.on_input(&press(MouseButton::Back, true), now);
        widget.on_input(
            &press(MouseButton::Back, false),
            now - Duration::from_millis(80),
        );
        widget.on_input(
            &InputEvent::MouseWheel { x: 0.0, y: -1.0 },
            now - Duration::from_millis(60),
        );
        render_snapshot(&mut widget, "mouse", now);

        let wheel = |y: f32, name: &str| {
            let mut w = MouseWidget::new(MouseOptions::default()).unwrap();
            w.on_input(
                &InputEvent::MouseWheel { x: 0.0, y },
                now - Duration::from_millis(60),
            );
            render_snapshot(&mut w, name, now);
        };
        wheel(1.0, "mouse-scroll-up");
        wheel(-1.0, "mouse-scroll-down");
        let mut click = MouseWidget::new(MouseOptions::default()).unwrap();
        click.on_input(&press(MouseButton::Middle, true), now);
        render_snapshot(&mut click, "mouse-middle-click", now);
    }
}
