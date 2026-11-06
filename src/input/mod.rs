//! Input sources (winit device events, gilrs, hidapi) turned into one `InputEvent` stream.

mod device;
pub mod gamepad;

use std::collections::HashSet;

pub use device::from_device_event;
pub use gamepad::PadEvent;
pub use winit::keyboard::KeyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputEvent {
    /// Physical key position, independent of keyboard layout.
    Key {
        key: KeyCode,
        pressed: bool,
    },
    MouseButton {
        button: MouseButton,
        pressed: bool,
    },
    /// Wheel notches (winit sign convention); `y > 0` = scroll up.
    MouseWheel {
        x: f32,
        y: f32,
    },
    /// Raw sensor counts, not cursor pixels; unaffected by cursor lock or acceleration.
    MouseMove {
        dx: f64,
        dy: f64,
    },
    /// Full pad snapshot (or disconnect) from a gamepad thread.
    Gamepad(PadEvent),
}

/// Currently held keys and buttons.
#[derive(Default)]
pub struct InputState {
    keys: HashSet<KeyCode>,
    buttons: HashSet<MouseButton>,
}

impl InputState {
    /// Applies an event; returns false when nothing changed (key auto-repeat, duplicate release).
    pub fn apply(&mut self, event: &InputEvent) -> bool {
        match *event {
            InputEvent::Key { key, pressed: true } => self.keys.insert(key),
            InputEvent::Key {
                key,
                pressed: false,
            } => self.keys.remove(&key),
            InputEvent::MouseButton {
                button,
                pressed: true,
            } => self.buttons.insert(button),
            InputEvent::MouseButton {
                button,
                pressed: false,
            } => self.buttons.remove(&button),
            InputEvent::MouseWheel { .. }
            | InputEvent::MouseMove { .. }
            | InputEvent::Gamepad(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_repeat_is_not_a_change() {
        let mut state = InputState::default();
        let down = InputEvent::Key {
            key: KeyCode::KeyW,
            pressed: true,
        };
        let up = InputEvent::Key {
            key: KeyCode::KeyW,
            pressed: false,
        };

        assert!(state.apply(&down));
        assert!(!state.apply(&down), "auto-repeat must be filtered");
        assert!(state.apply(&up));
        assert!(!state.apply(&up), "duplicate release must be filtered");
    }

    #[test]
    fn mouse_motion_always_changes() {
        let mut state = InputState::default();
        let mv = InputEvent::MouseMove { dx: 1.0, dy: 0.0 };
        assert!(state.apply(&mv));
        assert!(state.apply(&mv));
    }
}
