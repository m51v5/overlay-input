//! Keyboard widget: outlined keys, green + blue glow while held, fade on release.

use std::time::{Duration, Instant};

use tiny_skia::{Color, FillRule, Paint, Path, Stroke, Transform};

use super::{Canvas, StaticLayer, Widget};
use crate::input::{InputEvent, KeyCode};
use crate::render::easing::ease_out;
use crate::render::glow::GlowSpec;
use crate::render::text::Font;
use crate::render::{rgb, rounded_rect};

// Key sizes.
const PADDING: f32 = 10.0; // also room for the glow
const BORDER: f32 = 2.0;
const RADIUS: f32 = 10.0;
const FONT_PX: f32 = 20.0;
const KEY_GAP: f32 = 4.0; // between keys
const TEXT_EMBOLDEN: f32 = 0.6; // device px
const PRESSED: u32 = 0x2DA026;
const RELEASED: u32 = 0x96CF13;
const RELEASE_FADE: Duration = Duration::from_millis(500);
const GLOW: GlowSpec = GlowSpec {
    blur: 8.0,
    spread: 3.0,
    rgb: [35, 173, 255],
};

#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyboardOptions {
    #[serde(default)]
    pub preset: Preset,
    /// Keys not to show, by key name: `KeyW`, `Digit1`, `ShiftLeft`, `Space`, `Numpad7`, ...
    #[serde(default)]
    #[schemars(extend("format" = "key-list"))]
    pub hidden_keys: Vec<String>,
    /// What hidden keys leave behind.
    #[serde(default)]
    pub hide_mode: HideMode,
}

#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    serde::Deserialize,
    serde::Serialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum HideMode {
    /// Shown keys stay where they are; the widget shrinks to fit them.
    #[default]
    Crop,
    /// The widget keeps the whole keyboard's size; hidden keys leave holes.
    Keep,
    /// Shown keys slide left to close the holes; empty rows are removed.
    Collapse,
}

#[derive(
    Debug, Default, Clone, Copy, serde::Deserialize, serde::Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    /// Left-hand gaming cluster.
    #[default]
    Default,
    /// Full layout with navigation block and numpad.
    Full,
}

/// One key in a row; sizes without the border, which is added on top.
#[derive(Clone, Copy)]
struct Item {
    key: Option<(KeyCode, &'static str)>,
    w: f32,
    h: f32,
    /// Line height: the label is centered in the first line, even on tall keys.
    line: f32,
    font: f32,
}

impl Item {
    const fn w(mut self, w: f32) -> Self {
        self.w = w;
        self
    }
    /// `div.small-button` (F-row).
    const fn small(mut self) -> Self {
        self.h = 30.0;
        self.line = 30.0;
        self
    }
    /// `div.multirow-button` (numpad + / Enter).
    const fn tall(mut self) -> Self {
        self.h = 89.0;
        self
    }
    /// `div.smaller-font`.
    const fn smaller(mut self) -> Self {
        self.font = 18.0;
        self
    }
}

const fn k(code: KeyCode, label: &'static str) -> Item {
    Item {
        key: Some((code, label)),
        w: 40.0,
        h: 40.0,
        line: 40.0,
        font: FONT_PX,
    }
}

/// `div.white-space`.
const fn gap(w: f32) -> Item {
    Item {
        key: None,
        w,
        h: 0.0,
        line: 0.0,
        font: FONT_PX,
    }
}

const BLANK: Item = gap(40.0);

struct Row {
    /// Outer row pitch (`div.buttons` is fixed at 44px; taller keys overflow down).
    height: f32,
    gap_after: f32,
    items: &'static [Item],
}

use KeyCode as C;

const DEFAULT_LAYOUT: &[Row] = &[
    Row {
        height: 34.0,
        gap_after: 15.0,
        items: &[k(C::Escape, "Esc").small(), BLANK, k(C::F1, "F1").small()],
    },
    Row {
        height: 44.0,
        gap_after: 5.0,
        items: &[
            k(C::Backquote, "~"),
            k(C::Digit1, "1"),
            k(C::Digit2, "2"),
            k(C::Digit3, "3"),
            k(C::Digit4, "4"),
        ],
    },
    Row {
        height: 44.0,
        gap_after: 5.0,
        items: &[
            k(C::Tab, "Tab").w(60.0),
            k(C::KeyQ, "Q"),
            k(C::KeyW, "W"),
            k(C::KeyE, "E"),
            k(C::KeyR, "R"),
        ],
    },
    Row {
        height: 44.0,
        gap_after: 5.0,
        items: &[
            k(C::CapsLock, "Caps").w(70.0),
            k(C::KeyA, "A"),
            k(C::KeyS, "S"),
            k(C::KeyD, "D"),
            k(C::KeyF, "F"),
            k(C::KeyG, "G"),
        ],
    },
    Row {
        height: 44.0,
        gap_after: 5.0,
        items: &[
            k(C::ShiftLeft, "Shift").w(90.0),
            k(C::KeyZ, "Z"),
            k(C::KeyX, "X"),
            k(C::KeyC, "C"),
            k(C::KeyV, "V"),
        ],
    },
    Row {
        height: 44.0,
        gap_after: 0.0,
        items: &[
            k(C::ControlLeft, "Ctrl").w(60.0),
            BLANK,
            k(C::AltLeft, "Alt").w(60.0),
            k(C::Space, "Space").w(280.0),
        ],
    },
];

const FULL_LAYOUT: &[Row] = &[
    Row {
        height: 34.0,
        gap_after: 15.0,
        items: &[
            k(C::Escape, "Esc").small(),
            gap(20.0),
            k(C::F1, "F1").small(),
            k(C::F2, "F2").small(),
            k(C::F3, "F3").small(),
            k(C::F4, "F4").small(),
            gap(12.0),
            k(C::F5, "F5").small(),
            k(C::F6, "F6").small(),
            k(C::F7, "F7").small(),
            k(C::F8, "F8").small(),
            gap(12.0),
            k(C::F9, "F9").small(),
            k(C::F10, "F10").small(),
            k(C::F11, "F11").small(),
            k(C::F12, "F12").small(),
            BLANK,
            k(C::PrintScreen, "PtSn").small(),
            k(C::ScrollLock, "ScLk").small(),
            k(C::Pause, "⏸").small(),
        ],
    },
    Row {
        height: 44.0,
        gap_after: 5.0,
        items: &[
            k(C::Backquote, "~"),
            k(C::Digit1, "1"),
            k(C::Digit2, "2"),
            k(C::Digit3, "3"),
            k(C::Digit4, "4"),
            k(C::Digit5, "5"),
            k(C::Digit6, "6"),
            k(C::Digit7, "7"),
            k(C::Digit8, "8"),
            k(C::Digit9, "9"),
            k(C::Digit0, "0"),
            k(C::Minus, "-"),
            k(C::Equal, "="),
            k(C::Backspace, "⌫").w(60.0),
            BLANK,
            k(C::Insert, "Ins"),
            k(C::Home, "Hm"),
            k(C::PageUp, "PgUp").smaller(),
            BLANK,
            k(C::NumLock, "NLk"),
            k(C::NumpadDivide, "/"),
            k(C::NumpadMultiply, "*"),
            k(C::NumpadSubtract, "-"),
        ],
    },
    Row {
        height: 44.0,
        gap_after: 5.0,
        items: &[
            k(C::Tab, "Tab").w(60.0),
            k(C::KeyQ, "Q"),
            k(C::KeyW, "W"),
            k(C::KeyE, "E"),
            k(C::KeyR, "R"),
            k(C::KeyT, "T"),
            k(C::KeyY, "Y"),
            k(C::KeyU, "U"),
            k(C::KeyI, "I"),
            k(C::KeyO, "O"),
            k(C::KeyP, "P"),
            k(C::BracketLeft, "["),
            k(C::BracketRight, "]"),
            k(C::Backslash, "\\"),
            BLANK,
            k(C::Delete, "Del"),
            k(C::End, "End"),
            k(C::PageDown, "PgDn").smaller(),
            BLANK,
            k(C::Numpad7, "7"),
            k(C::Numpad8, "8"),
            k(C::Numpad9, "9"),
            k(C::NumpadAdd, "+").tall(),
        ],
    },
    Row {
        height: 44.0,
        gap_after: 5.0,
        items: &[
            k(C::CapsLock, "Caps").w(70.0),
            k(C::KeyA, "A"),
            k(C::KeyS, "S"),
            k(C::KeyD, "D"),
            k(C::KeyF, "F"),
            k(C::KeyG, "G"),
            k(C::KeyH, "H"),
            k(C::KeyJ, "J"),
            k(C::KeyK, "K"),
            k(C::KeyL, "L"),
            k(C::Semicolon, ";"),
            k(C::Quote, "'"),
            k(C::Enter, "Enter").w(78.0),
            BLANK,
            BLANK,
            BLANK,
            BLANK,
            BLANK,
            k(C::Numpad4, "4"),
            k(C::Numpad5, "5"),
            k(C::Numpad6, "6"),
        ],
    },
    Row {
        height: 44.0,
        gap_after: 5.0,
        items: &[
            k(C::ShiftLeft, "Shift").w(90.0),
            k(C::KeyZ, "Z"),
            k(C::KeyX, "X"),
            k(C::KeyC, "C"),
            k(C::KeyV, "V"),
            k(C::KeyB, "B"),
            k(C::KeyN, "N"),
            k(C::KeyM, "M"),
            k(C::Comma, ","),
            k(C::Period, "."),
            k(C::Slash, "/"),
            k(C::ShiftRight, "Shift").w(106.0),
            BLANK,
            BLANK,
            k(C::ArrowUp, "↑"),
            BLANK,
            BLANK,
            k(C::Numpad1, "1"),
            k(C::Numpad2, "2"),
            k(C::Numpad3, "3"),
            k(C::NumpadEnter, "⏎").tall(),
        ],
    },
    Row {
        height: 44.0,
        gap_after: 0.0,
        items: &[
            k(C::ControlLeft, "Ctrl").w(60.0),
            BLANK,
            k(C::AltLeft, "Alt").w(60.0),
            k(C::Space, "Space").w(280.0),
            k(C::AltRight, "Alt").w(60.0),
            gap(76.0),
            k(C::ControlRight, "Ctrl").w(60.0),
            BLANK,
            k(C::ArrowLeft, "←"),
            k(C::ArrowDown, "↓"),
            k(C::ArrowRight, "→"),
            BLANK,
            k(C::Numpad0, "0").w(88.0),
            k(C::NumpadDecimal, "."),
        ],
    },
];

#[derive(Clone, Copy)]
enum KeyState {
    Idle,
    Held,
    Released(Instant),
}

struct Key {
    code: KeyCode,
    /// Border box in logical px, relative to the widget.
    rect: (f32, f32, f32, f32),
    fill: Path,
    border: Path,
    label: Option<Path>,
    state: KeyState,
}

pub struct KeyboardWidget {
    keys: Vec<Key>,
    size: (f32, f32),
    /// Outlines and labels never change; stroking 100 keys every frame was the main cost.
    outlines: StaticLayer,
}

impl Preset {
    fn layout(self) -> &'static [Row] {
        match self {
            Preset::Default => DEFAULT_LAYOUT,
            Preset::Full => FULL_LAYOUT,
        }
    }
}

/// A key placed in the layout (logical px, border box).
struct Placed {
    code: KeyCode,
    label: &'static str,
    item: &'static Item,
    at: (f32, f32),
}

/// Lays out `layout` without the `hidden` keys; returns the keys and the full-keyboard size.
fn place(layout: &'static [Row], hidden: &[KeyCode], collapse: bool) -> (Vec<Placed>, (f32, f32)) {
    let mut placed = Vec::new();
    let (mut width, mut bottom): (f32, f32) = (0.0, 0.0);
    let mut y = PADDING;

    for row in layout {
        let mut x = PADDING;
        let mut shown_in_row = false;
        for item in row.items {
            let advance = item.w + 2.0 * BORDER + KEY_GAP;
            match item.key {
                Some((code, label)) if !hidden.contains(&code) => {
                    bottom = bottom.max(y + item.h + 2.0 * BORDER);
                    placed.push(Placed {
                        code,
                        label,
                        item,
                        at: (x, y),
                    });
                    shown_in_row = true;
                    x += advance;
                }
                // Collapse: hidden keys and the spacers before the first shown key take no room.
                Some(_) | None if collapse && (item.key.is_some() || !shown_in_row) => {}
                _ => x += advance,
            }
        }
        width = width.max(x - KEY_GAP);
        if shown_in_row || !collapse {
            y += row.height + row.gap_after;
        }
    }
    (placed, (width + PADDING, bottom.max(y) + PADDING))
}

/// Every key name `hidden_keys` accepts (both presets), e.g. `KeyW`.
fn key_by_name(name: &str) -> Option<KeyCode> {
    FULL_LAYOUT
        .iter()
        .flat_map(|row| row.items)
        .filter_map(|item| item.key)
        .map(|(code, _)| code)
        .find(|code| format!("{code:?}") == name)
}

/// Key positions of a preset for the key picker (logical px).
pub struct KeyLayout {
    pub size: [f32; 2],
    pub keys: Vec<KeyInfo>,
}

pub struct KeyInfo {
    /// `hidden_keys` name, e.g. `KeyW`.
    pub name: String,
    pub label: String,
    /// x, y, width, height.
    pub rect: [f32; 4],
}

pub fn layout_info(preset: Preset) -> KeyLayout {
    let (placed, (w, h)) = place(preset.layout(), &[], false);
    let keys = placed
        .iter()
        .map(|p| KeyInfo {
            name: format!("{:?}", p.code),
            label: p.label.to_owned(),
            rect: [
                p.at.0,
                p.at.1,
                p.item.w + 2.0 * BORDER,
                p.item.h + 2.0 * BORDER,
            ],
        })
        .collect();
    KeyLayout { size: [w, h], keys }
}

impl KeyboardWidget {
    pub fn new(font: &Font, options: KeyboardOptions) -> anyhow::Result<Self> {
        let hidden = options
            .hidden_keys
            .iter()
            .map(|name| {
                key_by_name(name).ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown key {name:?} in hidden_keys (names look like KeyW, Digit1, ShiftLeft, Space, Numpad7)"
                    )
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let mode = options.hide_mode;
        let (mut placed, mut size) =
            place(options.preset.layout(), &hidden, mode == HideMode::Collapse);
        anyhow::ensure!(!placed.is_empty(), "hidden_keys hides every key");

        // Crop (and collapse): move the shown keys to the corner and fit the box to them.
        if mode != HideMode::Keep && !hidden.is_empty() {
            let left = placed.iter().map(|p| p.at.0).fold(f32::MAX, f32::min);
            let top = placed.iter().map(|p| p.at.1).fold(f32::MAX, f32::min);
            let (mut right, mut bottom) = (0.0f32, 0.0f32);
            for p in &mut placed {
                p.at = (p.at.0 - left + PADDING, p.at.1 - top + PADDING);
                right = right.max(p.at.0 + p.item.w + 2.0 * BORDER);
                bottom = bottom.max(p.at.1 + p.item.h + 2.0 * BORDER);
            }
            size = (right + PADDING, bottom + PADDING);
        }

        let keys = placed
            .iter()
            .filter_map(|p| build_key(font, p.code, p.label, p.item, p.at))
            .collect();
        Ok(Self {
            keys,
            size,
            outlines: StaticLayer::default(),
        })
    }
}

fn build_key(font: &Font, code: KeyCode, label: &str, item: &Item, at: (f32, f32)) -> Option<Key> {
    let (x, y) = at;
    let (w, h) = (item.w + 2.0 * BORDER, item.h + 2.0 * BORDER);
    // The border belongs inside the box; tiny-skia strokes centered on the path.
    let half = BORDER / 2.0;
    Some(Key {
        code,
        rect: (x, y, w, h),
        fill: rounded_rect(x, y, w, h, RADIUS)?,
        border: rounded_rect(x + half, y + half, w - BORDER, h - BORDER, RADIUS - half)?,
        label: font.text_path(label, item.font, x + BORDER, y + BORDER, item.w, item.line),
        state: KeyState::Idle,
    })
}

impl Widget for KeyboardWidget {
    fn size(&self) -> (f32, f32) {
        self.size
    }

    fn on_input(&mut self, event: &InputEvent, now: Instant) -> bool {
        let InputEvent::Key { key, pressed } = *event else {
            return false;
        };
        let Some(k) = self.keys.iter_mut().find(|k| k.code == key) else {
            return false;
        };
        k.state = match (pressed, k.state) {
            (true, _) => KeyState::Held,
            (false, KeyState::Held) => KeyState::Released(now),
            (false, state) => state,
        };
        true
    }

    fn reset(&mut self) {
        for key in &mut self.keys {
            key.state = KeyState::Idle;
        }
    }

    fn is_animating(&self, now: Instant) -> bool {
        self.keys.iter().any(|k| {
            matches!(k.state, KeyState::Released(t) if now.saturating_duration_since(t) < RELEASE_FADE)
        })
    }

    fn draw(&self, canvas: &mut Canvas, origin: (f32, f32), now: Instant) {
        let s = canvas.scale;
        let ts = Transform::from_row(s, 0.0, 0.0, s, origin.0 * s, origin.1 * s);

        // Pressed fills and glows first, then the cached outlines on top.
        for key in &self.keys {
            let background = match key.state {
                KeyState::Idle => None,
                KeyState::Held => {
                    let (x, y, w, h) = key.rect;
                    let (layer, glow) = canvas.highlight_and_glow();
                    glow.draw(layer, origin.0 + x, origin.1 + y, w, h, RADIUS, GLOW, s);
                    Some(rgb(PRESSED, 1.0))
                }
                KeyState::Released(t) => {
                    let progress =
                        now.saturating_duration_since(t).as_secs_f32() / RELEASE_FADE.as_secs_f32();
                    (progress < 1.0).then(|| rgb(RELEASED, 1.0 - ease_out(progress)))
                }
            };
            if let Some(color) = background {
                canvas
                    .highlight()
                    .fill_path(&key.fill, &paint(color), FillRule::Winding, ts, None);
            }
        }

        self.outlines
            .blit(canvas.pixmap, self.size, origin, s, |layer, ts| {
                // Borders snap to whole device pixels.
                let stroke = Stroke {
                    width: (BORDER * s).floor().max(1.0) / s,
                    ..Default::default()
                };
                // Light-on-dark DirectWrite text looks heavier than plain AA; a hairline stroke matches it.
                let text_stroke = Stroke {
                    width: TEXT_EMBOLDEN / s,
                    ..Default::default()
                };
                let white = paint(Color::WHITE);
                for key in &self.keys {
                    layer.stroke_path(&key.border, &white, &stroke, ts, None);
                    if let Some(label) = &key.label {
                        layer.fill_path(label, &white, FillRule::Winding, ts, None);
                        layer.stroke_path(label, &white, &text_stroke, ts, None);
                    }
                }
            });
    }
}

fn paint(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::snapshot::render_snapshot;

    fn press(key: KeyCode, pressed: bool) -> InputEvent {
        InputEvent::Key { key, pressed }
    }

    fn font() -> Font {
        Font::load_system(&["calibri.ttf", "seguisym.ttf", "segoeui.ttf", "arial.ttf"]).unwrap()
    }

    /// Renders snapshots to look at: `cargo test snapshot -- --ignored`.
    #[test]
    #[ignore = "writes target/snapshots/*.png"]
    fn snapshot() {
        for (preset, name) in [
            (Preset::Default, "keyboard"),
            (Preset::Full, "keyboard-full"),
        ] {
            let mut widget = KeyboardWidget::new(
                &font(),
                KeyboardOptions {
                    preset,
                    ..Default::default()
                },
            )
            .unwrap();
            let now = Instant::now();
            widget.on_input(&press(KeyCode::KeyW, true), now);
            widget.on_input(&press(KeyCode::NumpadEnter, true), now);
            widget.on_input(&press(KeyCode::KeyA, true), now);
            widget.on_input(
                &press(KeyCode::KeyA, false),
                now - Duration::from_millis(150),
            );
            render_snapshot(&mut widget, name, now);
        }
    }

    #[test]
    fn full_preset_layout() {
        let widget = KeyboardWidget::new(&font(), full(&[], HideMode::Crop)).unwrap();
        assert_eq!(widget.keys.len(), 101, "every key of the full layout");
        let enter = widget
            .keys
            .iter()
            .find(|k| k.code == KeyCode::NumpadEnter)
            .unwrap();
        assert_eq!(enter.rect.3, 93.0, "tall key spans two rows");
        assert!(
            widget.keys.iter().all(|k| k.label.is_some()),
            "every label has glyphs"
        );
    }

    #[test]
    fn release_fades_then_stops() {
        let mut widget = KeyboardWidget::new(&font(), KeyboardOptions::default()).unwrap();
        let t0 = Instant::now();
        assert!(widget.on_input(&press(KeyCode::KeyW, true), t0));
        assert!(!widget.is_animating(t0), "held key is static");
        widget.on_input(&press(KeyCode::KeyW, false), t0);
        assert!(widget.is_animating(t0 + Duration::from_millis(100)));
        assert!(!widget.is_animating(t0 + RELEASE_FADE));
    }

    #[test]
    fn reset_clears_held_and_fading_keys() {
        let mut widget = KeyboardWidget::new(&font(), KeyboardOptions::default()).unwrap();
        let t0 = Instant::now();
        widget.on_input(&press(KeyCode::KeyW, true), t0);
        widget.on_input(&press(KeyCode::KeyA, true), t0);
        widget.on_input(&press(KeyCode::KeyA, false), t0);
        widget.reset();
        assert!(!widget.is_animating(t0));
        assert!(
            widget
                .keys
                .iter()
                .all(|k| matches!(k.state, KeyState::Idle))
        );
    }

    fn full(hidden: &[&str], hide_mode: HideMode) -> KeyboardOptions {
        KeyboardOptions {
            preset: Preset::Full,
            hidden_keys: hidden.iter().map(|s| s.to_string()).collect(),
            hide_mode,
        }
    }

    fn rect_of(widget: &KeyboardWidget, code: KeyCode) -> (f32, f32, f32, f32) {
        widget.keys.iter().find(|k| k.code == code).unwrap().rect
    }

    #[test]
    fn hidden_keys_by_mode() {
        let all = KeyboardWidget::new(&font(), full(&[], HideMode::Crop)).unwrap();
        let wasd = ["KeyW", "KeyA", "KeyS", "KeyD"];
        let others: Vec<&str> = all
            .keys
            .iter()
            .map(|k| format!("{:?}", k.code))
            .filter(|n| !wasd.contains(&n.as_str()))
            .map(|n| &*n.leak())
            .collect();

        let keep = KeyboardWidget::new(&font(), full(&others, HideMode::Keep)).unwrap();
        assert_eq!(keep.keys.len(), 4);
        assert_eq!(keep.size, all.size, "keep: full size");
        assert_eq!(rect_of(&keep, KeyCode::KeyW), rect_of(&all, KeyCode::KeyW));

        let crop = KeyboardWidget::new(&font(), full(&others, HideMode::Crop)).unwrap();
        let (a, w) = (rect_of(&crop, KeyCode::KeyA), rect_of(&crop, KeyCode::KeyW));
        assert_eq!(
            (a.0, w.1),
            (PADDING, PADDING),
            "crop: shown keys moved to the corner"
        );
        assert!(w.0 > a.0, "crop keeps the stagger: W sits right of A");
        assert!(crop.size.0 < all.size.0 / 3.0 && crop.size.1 < all.size.1 / 2.0);

        let collapse = KeyboardWidget::new(&font(), full(&others, HideMode::Collapse)).unwrap();
        let (a, w) = (
            rect_of(&collapse, KeyCode::KeyA),
            rect_of(&collapse, KeyCode::KeyW),
        );
        assert_eq!(a.0, w.0, "collapse: both rows start at the left edge");
        assert_eq!(a.1 - w.1, 49.0, "collapse: empty rows removed");
    }

    #[test]
    fn hidden_key_names_are_checked() {
        let err = KeyboardWidget::new(&font(), full(&["KeyWW"], HideMode::Crop))
            .err()
            .unwrap();
        assert!(err.to_string().contains("KeyWW"));
        // Full-layout names are fine on the small preset too (ignored there).
        let small = KeyboardOptions {
            hidden_keys: vec!["Numpad7".into()],
            ..Default::default()
        };
        assert!(KeyboardWidget::new(&font(), small).is_ok());
        let everything: Vec<String> = KeyboardWidget::new(&font(), KeyboardOptions::default())
            .unwrap()
            .keys
            .iter()
            .map(|k| format!("{:?}", k.code))
            .collect();
        let none_left = KeyboardOptions {
            hidden_keys: everything,
            ..Default::default()
        };
        assert!(KeyboardWidget::new(&font(), none_left).is_err());
    }

    #[test]
    fn layout_info_lists_every_key() {
        let info = layout_info(Preset::Default);
        assert_eq!(info.keys.len(), 26);
        assert_eq!(info.keys[0].name, "Escape");
        assert_eq!(info.keys[0].label, "Esc");
    }

    #[test]
    fn unknown_key_is_ignored() {
        let mut widget = KeyboardWidget::new(&font(), KeyboardOptions::default()).unwrap();
        assert!(!widget.on_input(&press(KeyCode::F12, true), Instant::now()));
    }
}
