//! Gamepad widget (Xbox, DualSense): buttons glow while held and fade on release,
//! sticks move, triggers fill and sink, D-pad segments appear while pressed.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Deserialize;
use tiny_skia::{Color, FillRule, Mask, Path, Transform};

use super::{Canvas, StaticLayer, Widget};
use crate::input::InputEvent;
use crate::input::gamepad::{GamepadState, PadEvent, PadId, PadSource, buttons as b};
use crate::render::glow::GlowCache;
use crate::render::rgb;
use crate::render::svg::{self, SvgShape};

const XBOX_SVG: &str = include_str!("../../assets/widgets/gamepad-xbox.svg");
const DUALSENSE_SVG: &str = include_str!("../../assets/widgets/gamepad-dualsense.svg");
const MARGIN: f32 = 10.0; // body > div margin
const PRESSED: u32 = 0x2DA026;
const PRESSED_RGB: [u8; 3] = [0x2D, 0xA0, 0x26];
const RELEASED: u32 = 0x96CF13;
const RELEASE_FADE: Duration = Duration::from_millis(250);
const GLOW_STD_DEV: f32 = 10.0; // svg units
const TRIGGER_SHIFT: f32 = 60.0; // svg units (triggerShiftFactor)
/// Stick deflection below this shows no tint (rest noise).
const STICK_DEADZONE: f32 = 0.12;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct GamepadOptions {
    pub style: Style,
    /// Which pad to show; `auto` locks onto the first pad that sends input.
    pub source: Source,
    /// XInput user index (0..=3) when `source = "xinput"`.
    pub index: Option<usize>,
    /// Drawing width in px.
    #[schemars(range(min = 100.0, max = 3000.0))]
    pub width: f32,
    /// Tint the stick cap while the stick is pushed (stronger = further).
    pub stick_highlight: bool,
    /// Stick tint color; differs from the click green so a push never looks like L3/R3.
    #[schemars(extend("format" = "color"))]
    pub stick_color: [u8; 3],
}

impl Default for GamepadOptions {
    fn default() -> Self {
        Self {
            style: Style::Xbox,
            source: Source::Auto,
            index: None,
            width: 1000.0,
            stick_highlight: true,
            stick_color: [35, 173, 255], // same blue as the keyboard glow / mouse scroll
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Style {
    Xbox,
    Dualsense,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    Auto,
    Xinput,
    Dualsense,
}

const XBOX_BUTTONS: &[(&str, u16)] = &[
    ("ButtonA", b::A),
    ("ButtonB", b::B),
    ("ButtonX", b::X),
    ("ButtonY", b::Y),
    ("ButtonLB", b::LB),
    ("ButtonRB", b::RB),
    ("ButtonBack", b::BACK),
    ("ButtonStart", b::START),
    ("ButtonL3", b::L3),
    ("ButtonR3", b::R3),
];

/// DualSense SVG: L3/R3 light the stick cores, D-pad directions are plain buttons.
const DUALSENSE_BUTTONS: &[(&str, u16)] = &[
    ("ButtonA", b::A),
    ("ButtonB", b::B),
    ("ButtonX", b::X),
    ("ButtonY", b::Y),
    ("ButtonLB", b::LB),
    ("ButtonRB", b::RB),
    ("ButtonBack", b::BACK),
    ("ButtonStart", b::START),
    ("LeftStickCore", b::L3),
    ("RightStickCore", b::R3),
    ("DPadUp", b::DPAD_UP),
    ("DPadDown", b::DPAD_DOWN),
    ("DPadLeft", b::DPAD_LEFT),
    ("DPadRight", b::DPAD_RIGHT),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

enum Part {
    Static,
    Button(u16),
    /// Xbox D-pad segment D0..D7 (up, up-right, ... clockwise); hidden unless active.
    Segment(usize),
    Trigger(Side),
    /// `*TriggerClip`: clip region only, never drawn.
    Clip,
}

struct Shape {
    svg: SvgShape,
    part: Part,
    stick: Option<Side>,
    /// Run of consecutive non-moving static parts this one is cached in (drawn as one layer, in order).
    band: Option<usize>,
}

#[derive(Clone, Copy)]
enum Press {
    Idle,
    Held,
    Released(Instant),
}

type MaskKey = (u32, u32, [u32; 6]);

pub struct GamepadWidget {
    options: GamepadOptions,
    shapes: Vec<Shape>,
    clips: [Option<Path>; 2],
    size: (f32, f32),
    unit: f32,
    stick_shift: f32,
    locked: Option<PadId>,
    state: GamepadState,
    buttons: [Press; 16],
    segments: [Press; 8],
    masks: RefCell<Option<(MaskKey, [Option<Mask>; 2])>>,
    /// One cached layer per band of static parts.
    bands: Vec<StaticLayer>,
}

impl GamepadWidget {
    pub fn new(options: GamepadOptions) -> Result<Self> {
        anyhow::ensure!(options.width > 0.0, "width must be > 0");
        let (source, table, stick_shift) = match options.style {
            Style::Xbox => (XBOX_SVG, XBOX_BUTTONS, 14.0),
            Style::Dualsense => (DUALSENSE_SVG, DUALSENSE_BUTTONS, 20.0),
        };
        let image = svg::load(source, options.width, (MARGIN, MARGIN))?;

        let mut clips = [None, None];
        let mut shapes = Vec::with_capacity(image.shapes.len());
        for shape in image.shapes {
            let side_of = |left: &str, right: &str| {
                if shape.has_id(left) {
                    Some(Side::Left)
                } else if shape.has_id(right) {
                    Some(Side::Right)
                } else {
                    None
                }
            };
            let stick = side_of("LeftStickCore", "RightStickCore");
            let part = if let Some(side) = side_of("LeftTriggerClip", "RightTriggerClip") {
                clips[side as usize] = Some(shape.path.clone());
                Part::Clip
            } else if let Some(side) = side_of("LeftTrigger", "RightTrigger") {
                Part::Trigger(side)
            } else if let Some(i) = (options.style == Style::Xbox)
                .then(|| (0..8).find(|i| shape.has_id(&format!("D{i}"))))
                .flatten()
            {
                Part::Segment(i)
            } else if let Some((_, bit)) = table.iter().find(|(id, _)| shape.has_id(id)) {
                Part::Button(*bit)
            } else {
                Part::Static
            };
            shapes.push(Shape {
                svg: shape,
                part,
                stick,
                band: None,
            });
        }
        let bands = assign_bands(&mut shapes);

        Ok(Self {
            size: (options.width + 2.0 * MARGIN, image.size.1 + 2.0 * MARGIN),
            unit: image.unit,
            options,
            shapes,
            clips,
            stick_shift,
            locked: None,
            state: GamepadState::default(),
            buttons: [Press::Idle; 16],
            segments: [Press::Idle; 8],
            masks: RefCell::new(None),
            bands: (0..bands).map(|_| StaticLayer::default()).collect(),
        })
    }

    fn accepts(&self, pad: PadId) -> bool {
        match self.options.source {
            Source::Auto => true,
            Source::Xinput => {
                pad.source == PadSource::XInput && self.options.index.is_none_or(|i| i == pad.index)
            }
            Source::Dualsense => pad.source == PadSource::DualSense,
        }
    }

    fn apply(&mut self, new: GamepadState, now: Instant) -> bool {
        let old = self.state;
        if old == new {
            return false;
        }
        for bit in 0..16 {
            let m = 1u16 << bit;
            match (old.buttons & m != 0, new.buttons & m != 0) {
                (false, true) => self.buttons[bit] = Press::Held,
                (true, false) => self.buttons[bit] = Press::Released(now),
                _ => {}
            }
        }
        if self.options.style == Style::Xbox {
            let (was, is) = (dpad_index(old.buttons), dpad_index(new.buttons));
            if was != is {
                if let Some(i) = was {
                    self.segments[i] = Press::Released(now);
                }
                if let Some(i) = is {
                    self.segments[i] = Press::Held;
                }
            }
        }
        self.state = new;
        true
    }

    fn button(&self, bit: u16) -> Press {
        self.buttons[bit.trailing_zeros() as usize]
    }

    /// Stick core offset in logical px, clamped to the unit circle.
    fn stick_offset(&self, side: Side) -> (f32, f32) {
        let (mut x, mut y) = match side {
            Side::Left => self.state.left_stick,
            Side::Right => self.state.right_stick,
        };
        let d = x * x + y * y;
        if d > 1.0 {
            let k = 1.0 / d.sqrt();
            x *= k;
            y *= k;
        }
        let k = self.stick_shift * self.unit;
        (x * k, -y * k)
    }

    /// Stick tint strength 0..=1 from deflection, after the deadzone.
    fn stick_tint(&self, side: Side) -> f32 {
        if !self.options.stick_highlight {
            return 0.0;
        }
        let (x, y) = match side {
            Side::Left => self.state.left_stick,
            Side::Right => self.state.right_stick,
        };
        let magnitude = (x * x + y * y).sqrt().min(1.0);
        ((magnitude - STICK_DEADZONE) / (1.0 - STICK_DEADZONE)).clamp(0.0, 1.0)
    }

    /// Fill for a button shape: click green, release fade, else the stick-push tint (for L3/R3 caps).
    fn button_fill(&self, bit: u16, now: Instant) -> Option<Color> {
        let side = match bit {
            b::L3 => Some(Side::Left),
            b::R3 => Some(Side::Right),
            _ => None,
        };
        let tint = side.map_or(0.0, |side| self.stick_tint(side));
        let [r, g, bl] = self.options.stick_color;
        let pushed =
            (tint > 0.0).then(|| Color::from_rgba8(r, g, bl, (tint * 255.0).round() as u8));
        match self.button(bit) {
            Press::Idle => pushed,
            Press::Held => Some(rgb(PRESSED, 1.0)),
            // Only the opacity fades; once the fade ends the push tint shows again.
            Press::Released(t) => {
                let p = progress(now, t);
                (p < 1.0).then(|| rgb(RELEASED, 1.0 - p)).or(pushed)
            }
        }
    }

    fn trigger(&self, side: Side) -> f32 {
        match side {
            Side::Left => self.state.left_trigger,
            Side::Right => self.state.right_trigger,
        }
    }

    /// Static clip mask for a trigger, cached per target size and transform.
    fn with_clip<R>(
        &self,
        side: Side,
        width: u32,
        height: u32,
        ts: Transform,
        f: impl FnOnce(Option<&Mask>) -> R,
    ) -> R {
        let key: MaskKey = (
            width,
            height,
            [ts.sx, ts.kx, ts.ky, ts.sy, ts.tx, ts.ty].map(f32::to_bits),
        );
        let mut cache = self.masks.borrow_mut();
        if cache.as_ref().is_none_or(|(k, _)| *k != key) {
            let build = |clip: &Option<Path>| {
                let path = clip.as_ref()?;
                let mut mask = Mask::new(width, height)?;
                mask.fill_path(path, FillRule::Winding, true, ts);
                Some(mask)
            };
            *cache = Some((key, [build(&self.clips[0]), build(&self.clips[1])]));
        }
        let masks = cache.as_ref().map(|(_, m)| m);
        f(masks.and_then(|m| m[side as usize].as_ref()))
    }

    fn glow(&self, canvas: &mut Canvas, index: usize, origin: (f32, f32), shift: (f32, f32)) {
        let s = canvas.scale;
        let key = format!(
            "pad-{:?}-{}-glow-{index}",
            self.options.style, self.options.width
        );
        let (layer, glow) = canvas.highlight_and_glow();
        let sprite = glow.sprite(&key, s, || {
            svg::glow_sprite(
                &[&self.shapes[index].svg],
                PRESSED_RGB,
                GLOW_STD_DEV,
                self.unit,
                s,
            )
        });
        GlowCache::blit(
            layer,
            sprite,
            (origin.0 + shift.0) * s,
            (origin.1 + shift.1) * s,
            1.0,
        );
    }
}

/// Groups consecutive static parts that never move into bands; returns how many.
/// Each band is cached as one layer and drawn where its parts were, so nothing changes what covers what.
fn assign_bands(shapes: &mut [Shape]) -> usize {
    let mut bands = 0;
    let mut open = false;
    for shape in shapes {
        match shape.part {
            Part::Clip => {} // never drawn: doesn't split a band
            Part::Static if shape.stick.is_none() => {
                if !open {
                    bands += 1;
                    open = true;
                }
                shape.band = Some(bands - 1);
            }
            _ => open = false,
        }
    }
    bands
}

/// D-pad position: 0 = up, clockwise to 7 = up-left.
fn dpad_index(buttons: u16) -> Option<usize> {
    let (up, down, left, right) = (
        buttons & b::DPAD_UP != 0,
        buttons & b::DPAD_DOWN != 0,
        buttons & b::DPAD_LEFT != 0,
        buttons & b::DPAD_RIGHT != 0,
    );
    match (up, down, left, right) {
        (true, _, true, _) => Some(7),
        (true, _, _, true) => Some(1),
        (true, ..) => Some(0),
        (_, true, true, _) => Some(5),
        (_, true, _, true) => Some(3),
        (_, true, ..) => Some(4),
        (.., true, _) => Some(6),
        (.., true) => Some(2),
        _ => None,
    }
}

fn progress(now: Instant, start: Instant) -> f32 {
    (now.saturating_duration_since(start).as_secs_f32() / RELEASE_FADE.as_secs_f32()).min(1.0)
}

fn fading(p: Press, now: Instant) -> bool {
    matches!(p, Press::Released(t) if now.saturating_duration_since(t) < RELEASE_FADE)
}

impl Widget for GamepadWidget {
    fn size(&self) -> (f32, f32) {
        self.size
    }

    fn on_input(&mut self, event: &InputEvent, now: Instant) -> bool {
        let InputEvent::Gamepad(PadEvent {
            pad,
            state,
            connected,
        }) = *event
        else {
            return false;
        };
        if !self.accepts(pad) {
            return false;
        }
        if !connected {
            if self.locked == Some(pad) {
                self.locked = None;
                return self.apply(GamepadState::default(), now);
            }
            return false;
        }
        match self.locked {
            None if state.is_idle() => return false,
            None => self.locked = Some(pad),
            Some(locked) if locked != pad => return false,
            Some(_) => {}
        }
        self.apply(state, now)
    }

    fn uses_gamepads(&self) -> bool {
        true
    }

    fn reset(&mut self) {
        self.locked = None;
        self.state = GamepadState::default();
        self.buttons = [Press::Idle; 16];
        self.segments = [Press::Idle; 8];
    }

    fn is_animating(&self, now: Instant) -> bool {
        self.buttons
            .iter()
            .chain(&self.segments)
            .any(|p| fading(*p, now))
    }

    fn draw(&self, canvas: &mut Canvas, origin: (f32, f32), now: Instant) {
        let s = canvas.scale;
        let base = Transform::from_row(s, 0.0, 0.0, s, origin.0 * s, origin.1 * s);

        let mut next_band = 0;
        for (index, shape) in self.shapes.iter().enumerate() {
            if let Some(band) = shape.band {
                // First part of a band: draw the whole band from its cached layer.
                if band == next_band {
                    next_band += 1;
                    self.bands[band].blit(canvas.pixmap, self.size, origin, s, |pixmap, ts| {
                        for part in self.shapes.iter().filter(|p| p.band == Some(band)) {
                            part.svg.draw(pixmap, ts, 1.0, None, None);
                        }
                    });
                }
                continue;
            }
            let shift = shape
                .stick
                .map_or((0.0, 0.0), |side| self.stick_offset(side));
            let ts = base.pre_translate(shift.0, shift.1);
            match shape.part {
                Part::Clip => {}
                Part::Static => shape.svg.draw(canvas.pixmap, ts, 1.0, None, None),
                Part::Button(bit) => {
                    if matches!(self.button(bit), Press::Held) {
                        self.glow(canvas, index, origin, shift);
                    }
                    let fill = self.button_fill(bit, now);
                    canvas.draw_svg(&shape.svg, ts, fill, None);
                }
                Part::Segment(i) => match self.segments[i] {
                    Press::Idle => {}
                    Press::Held => {
                        self.glow(canvas, index, origin, shift);
                        shape
                            .svg
                            .draw(canvas.highlight(), ts, 1.0, Some(rgb(PRESSED, 1.0)), None);
                    }
                    // Segments fade as a whole (outline too), then disappear.
                    Press::Released(t) => {
                        let p = progress(now, t);
                        if p < 1.0 {
                            shape.svg.draw(
                                canvas.highlight(),
                                ts,
                                1.0 - p,
                                Some(rgb(RELEASED, 1.0)),
                                None,
                            );
                        }
                    }
                },
                Part::Trigger(side) => {
                    let value = self.trigger(side);
                    let sunk = base.pre_translate(0.0, value * TRIGGER_SHIFT * self.unit);
                    let fill: Color = rgb(PRESSED, value);
                    let (w, h) = (canvas.pixmap.width(), canvas.pixmap.height());
                    self.with_clip(side, w, h, base, |mask| {
                        canvas.draw_svg(&shape.svg, sunk, Some(fill), mask);
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::snapshot::render_snapshot;

    fn pad(state: GamepadState, source: PadSource) -> InputEvent {
        InputEvent::Gamepad(PadEvent {
            pad: PadId { source, index: 0 },
            state,
            connected: true,
        })
    }

    #[test]
    fn cached_parts_render_like_live_ones() {
        for style in [Style::Xbox, Style::Dualsense] {
            let render = |cache: bool| {
                let mut w = GamepadWidget::new(GamepadOptions {
                    style,
                    ..Default::default()
                })
                .unwrap();
                assert!(!w.bands.is_empty(), "{style:?}: something is cached");
                if !cache {
                    w.shapes.iter_mut().for_each(|s| s.band = None);
                }
                let now = Instant::now();
                let busy = GamepadState {
                    buttons: b::A | b::RB | b::DPAD_UP | b::DPAD_RIGHT | b::L3,
                    left_stick: (0.7, 0.7),
                    right_stick: (-0.9, 0.0),
                    left_trigger: 0.5,
                    right_trigger: 1.0,
                };
                w.on_input(&pad(busy, PadSource::XInput), now);
                let s = 1.25;
                let (pw, ph) = ((w.size.0 * s).ceil() as u32, (w.size.1 * s).ceil() as u32);
                let mut pixmap = tiny_skia::Pixmap::new(pw, ph).unwrap();
                w.draw(
                    &mut Canvas {
                        pixmap: &mut pixmap,
                        highlight: None,
                        scale: s,
                        glow: &mut GlowCache::default(),
                    },
                    (3.3, 7.6), // sub-pixel origin, like a widget placed in a layout
                    now,
                );
                pixmap
            };
            let (cached, live) = (render(true), render(false));
            let off = cached
                .pixels()
                .iter()
                .zip(live.pixels())
                .filter(|(a, b)| a.alpha().abs_diff(b.alpha()) > 2)
                .count();
            // Overlapping anti-aliased edges inside one cached band can round a little differently.
            assert!(off <= 10, "{style:?}: {off} pixels differ by more than 2");
            let worst = cached
                .pixels()
                .iter()
                .zip(live.pixels())
                .map(|(a, b)| {
                    [
                        a.red().abs_diff(b.red()),
                        a.green().abs_diff(b.green()),
                        a.blue().abs_diff(b.blue()),
                        a.alpha().abs_diff(b.alpha()),
                    ]
                    .into_iter()
                    .max()
                    .unwrap()
                })
                .max()
                .unwrap();
            assert!(
                worst <= 16,
                "{style:?}: cached differs from live by {worst}"
            );
        }
    }

    #[test]
    fn svg_parts_are_found() {
        for style in [Style::Xbox, Style::Dualsense] {
            let w = GamepadWidget::new(GamepadOptions {
                style,
                ..Default::default()
            })
            .unwrap();
            let count = |f: fn(&Part) -> bool| w.shapes.iter().filter(|s| f(&s.part)).count();
            assert_eq!(
                count(|p| matches!(p, Part::Trigger(_))),
                2,
                "{style:?} triggers"
            );
            assert!(w.clips.iter().all(Option::is_some), "{style:?} clips");
            assert!(
                w.shapes.iter().any(|s| s.stick == Some(Side::Left)),
                "{style:?} left stick"
            );
            let table = if style == Style::Xbox {
                XBOX_BUTTONS
            } else {
                DUALSENSE_BUTTONS
            };
            for (id, bit) in table {
                assert!(
                    w.shapes
                        .iter()
                        .any(|s| matches!(s.part, Part::Button(x) if x == *bit)),
                    "{style:?} {id}"
                );
            }
            if style == Style::Xbox {
                assert_eq!(count(|p| matches!(p, Part::Segment(_))), 8);
            }
        }
    }

    #[test]
    fn auto_locks_to_first_active_pad() {
        let mut w = GamepadWidget::new(GamepadOptions::default()).unwrap();
        let now = Instant::now();
        let a = GamepadState {
            buttons: b::A,
            ..Default::default()
        };
        assert!(
            !w.on_input(&pad(GamepadState::default(), PadSource::XInput), now),
            "idle pad doesn't lock"
        );
        assert!(w.on_input(&pad(a, PadSource::DualSense), now));
        assert!(
            !w.on_input(&pad(GamepadState::default(), PadSource::XInput), now),
            "other pad ignored"
        );
        assert!(matches!(w.button(b::A), Press::Held));
        assert!(w.on_input(&pad(GamepadState::default(), PadSource::DualSense), now));
        assert!(matches!(w.button(b::A), Press::Released(_)));
    }

    #[test]
    fn stick_push_tints_without_clicking() {
        let mut w = GamepadWidget::new(GamepadOptions::default()).unwrap();
        let now = Instant::now();
        let push = GamepadState {
            left_stick: (0.0, 1.0),
            ..Default::default()
        };
        assert!(w.on_input(&pad(push, PadSource::XInput), now));
        assert_eq!(w.stick_tint(Side::Left), 1.0);
        assert_eq!(w.stick_tint(Side::Right), 0.0);
        assert!(
            matches!(w.button(b::L3), Press::Idle),
            "a push is not a click"
        );

        let rest = GamepadState {
            left_stick: (0.05, -0.05),
            buttons: b::A,
            ..Default::default()
        };
        w.on_input(&pad(rest, PadSource::XInput), now);
        assert_eq!(w.stick_tint(Side::Left), 0.0, "rest noise inside deadzone");
    }

    #[test]
    fn push_tint_returns_after_click_release() {
        let mut w = GamepadWidget::new(GamepadOptions::default()).unwrap();
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let pushed = GamepadState {
            left_stick: (1.0, 0.0),
            ..Default::default()
        };
        let clicked = GamepadState {
            buttons: b::L3,
            ..pushed
        };
        w.on_input(&pad(clicked, PadSource::XInput), t0);
        w.on_input(&pad(pushed, PadSource::XInput), at(100));
        let blue = |c: Option<Color>| c.is_some_and(|c| c.blue() > 0.9 && c.green() < 0.8);
        assert!(!blue(w.button_fill(b::L3, at(150))), "release fade first");
        assert!(
            blue(w.button_fill(b::L3, at(400))),
            "tint back after the fade"
        );
    }

    #[test]
    fn dpad_diagonals() {
        assert_eq!(dpad_index(b::DPAD_UP | b::DPAD_RIGHT), Some(1));
        assert_eq!(dpad_index(b::DPAD_DOWN | b::DPAD_LEFT), Some(5));
        assert_eq!(dpad_index(b::DPAD_LEFT), Some(6));
        assert_eq!(dpad_index(0), None);
    }

    #[test]
    #[ignore = "writes target/snapshots/gamepad-*.png"]
    fn snapshot() {
        for (style, name) in [
            (Style::Xbox, "gamepad-xbox"),
            (Style::Dualsense, "gamepad-dualsense"),
        ] {
            let mut w = GamepadWidget::new(GamepadOptions {
                style,
                ..Default::default()
            })
            .unwrap();
            let now = Instant::now();
            let lb = GamepadState {
                buttons: b::LB,
                ..Default::default()
            };
            w.on_input(
                &pad(lb, PadSource::XInput),
                now - Duration::from_millis(200),
            );
            let state = GamepadState {
                buttons: b::A | b::DPAD_UP | b::DPAD_RIGHT | b::R3,
                left_stick: (0.7, 0.7),
                right_stick: (-1.0, 0.0),
                left_trigger: 0.0,
                right_trigger: 0.6,
            };
            w.on_input(
                &pad(state, PadSource::XInput),
                now - Duration::from_millis(100),
            );
            render_snapshot(&mut w, name, now);
        }

        let mut push = GamepadWidget::new(GamepadOptions {
            width: 500.0,
            ..Default::default()
        })
        .unwrap();
        let now = Instant::now();
        let state = GamepadState {
            left_stick: (0.8, 0.5),
            buttons: b::R3,
            ..Default::default()
        };
        push.on_input(&pad(state, PadSource::XInput), now);
        render_snapshot(&mut push, "gamepad-xbox-stick-push", now);
    }
}

#[cfg(test)]
mod perf {
    use super::*;
    use tiny_skia::Pixmap;

    /// Where gamepad draw time goes: `cargo test gamepad_part_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "benchmark"]
    fn gamepad_part_cost() {
        let style = if std::env::var("DS").is_ok() {
            Style::Dualsense
        } else {
            Style::Xbox
        };
        let w = GamepadWidget::new(GamepadOptions {
            style,
            ..Default::default()
        })
        .unwrap();
        let s = 1.25;
        let (pw, ph) = (
            (w.size().0 * s).ceil() as u32,
            (w.size().1 * s).ceil() as u32,
        );
        let mut pixmap = Pixmap::new(pw, ph).unwrap();
        let ts = Transform::from_scale(s, s);
        let mut costs: Vec<(f64, String, usize, bool, bool)> = Vec::new();
        for shape in &w.shapes {
            let t = std::time::Instant::now();
            for _ in 0..50 {
                shape.svg.draw(&mut pixmap, ts, 1.0, None, None);
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0 / 50.0;
            let kind = match shape.part {
                Part::Static => "static",
                Part::Button(_) => "button",
                Part::Segment(_) => "segment",
                Part::Trigger(_) => "trigger",
                Part::Clip => "clip",
            };
            costs.push((
                ms,
                format!("{kind} {:?}", shape.svg.ids.first()),
                shape.svg.path.len(),
                shape.svg.fill.is_some(),
                shape.svg.stroke.is_some(),
            ));
        }
        for cache in [true, false] {
            let mut w = GamepadWidget::new(GamepadOptions {
                style,
                ..Default::default()
            })
            .unwrap();
            if !cache {
                w.shapes.iter_mut().for_each(|s| s.band = None);
            }
            let counts = (
                w.bands.len(),
                w.shapes.iter().filter(|s| s.band.is_none()).count(),
            );
            let mut glow = crate::render::glow::GlowCache::default();
            let mut frame = |pixmap: &mut Pixmap| {
                let t = std::time::Instant::now();
                w.draw(
                    &mut crate::widgets::Canvas {
                        pixmap,
                        highlight: None,
                        scale: s,
                        glow: &mut glow,
                    },
                    (0.0, 0.0),
                    std::time::Instant::now(),
                );
                t.elapsed().as_secs_f64() * 1000.0
            };
            let first = frame(&mut pixmap);
            let next: f64 = (0..20).map(|_| frame(&mut pixmap)).sum::<f64>() / 20.0;
            println!("cache={cache} bands/live={counts:?} first {first:.2} ms, then {next:.2} ms");
        }
        let total: f64 = costs.iter().map(|c| c.0).sum();
        costs.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!("{} shapes, {total:.2} ms total", costs.len());
        for (ms, name, points, fill, stroke) in costs.iter().take(8) {
            println!("{ms:6.3} ms  {name:<40} points {points:<5} fill {fill} stroke {stroke}");
        }
        let live: Vec<String> = w
            .shapes
            .iter()
            .filter(|sh| sh.band.is_none() && !matches!(sh.part, Part::Clip))
            .map(|sh| {
                format!(
                    "{:?}{}",
                    sh.svg.ids.first(),
                    if sh.stick.is_some() { " (stick)" } else { "" }
                )
            })
            .collect();
        println!("live: {live:?}");
    }
}
