//! winit `DeviceEvent` → `InputEvent`. Raw Input under the hood, delivered while unfocused.

use winit::event::{DeviceEvent, MouseScrollDelta, RawKeyEvent};
use winit::keyboard::PhysicalKey;

use super::{InputEvent, MouseButton};

pub fn from_device_event(event: &DeviceEvent) -> Option<InputEvent> {
    match *event {
        DeviceEvent::Key(RawKeyEvent {
            physical_key,
            state,
        }) => match physical_key {
            PhysicalKey::Code(key) => Some(InputEvent::Key {
                key,
                pressed: state.is_pressed(),
            }),
            PhysicalKey::Unidentified(code) => {
                tracing::debug!(?code, "unidentified key ignored");
                None
            }
        },
        DeviceEvent::Button { button, state } => Some(InputEvent::MouseButton {
            button: mouse_button(button)?,
            pressed: state.is_pressed(),
        }),
        DeviceEvent::MouseWheel {
            delta: MouseScrollDelta::LineDelta(x, y),
        } => Some(InputEvent::MouseWheel { x, y }),
        DeviceEvent::MouseMotion { delta: (dx, dy) } => Some(InputEvent::MouseMove { dx, dy }),
        _ => None,
    }
}

/// winit's Windows backend numbers raw buttons 0..=4 as RI_MOUSE_BUTTON_1..5.
fn mouse_button(id: u32) -> Option<MouseButton> {
    Some(match id {
        0 => MouseButton::Left,
        1 => MouseButton::Right,
        2 => MouseButton::Middle,
        3 => MouseButton::Back,
        4 => MouseButton::Forward,
        _ => return None,
    })
}
