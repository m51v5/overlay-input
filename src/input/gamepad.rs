//! Gamepads: XInput pads via gilrs, DualSense via hidapi. Both read in background threads, no focus needed.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use gilrs::{Axis, Button, EventType, GamepadId, GilrsBuilder};

/// XInput button bits.
pub mod buttons {
    pub const DPAD_UP: u16 = 0x0001;
    pub const DPAD_DOWN: u16 = 0x0002;
    pub const DPAD_LEFT: u16 = 0x0004;
    pub const DPAD_RIGHT: u16 = 0x0008;
    pub const START: u16 = 0x0010;
    pub const BACK: u16 = 0x0020;
    pub const L3: u16 = 0x0040;
    pub const R3: u16 = 0x0080;
    pub const LB: u16 = 0x0100;
    pub const RB: u16 = 0x0200;
    pub const A: u16 = 0x1000;
    pub const B: u16 = 0x2000;
    pub const X: u16 = 0x4000;
    pub const Y: u16 = 0x8000;
}

/// Normalized pad snapshot: sticks -1..=1 (up = +y), triggers 0..=1.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GamepadState {
    pub buttons: u16,
    pub left_stick: (f32, f32),
    pub right_stick: (f32, f32),
    pub left_trigger: f32,
    pub right_trigger: f32,
}

impl GamepadState {
    pub fn is_idle(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PadSource {
    XInput,
    DualSense,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PadId {
    pub source: PadSource,
    pub index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PadEvent {
    pub pad: PadId,
    pub state: GamepadState,
    pub connected: bool,
}

/// Background readers; dropping stops them.
pub struct GamepadThreads {
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
}

impl GamepadThreads {
    pub fn spawn(sink: impl Fn(PadEvent) + Send + Clone + 'static) -> Self {
        let paused = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::new();

        let (p, s, out) = (paused.clone(), stop.clone(), sink.clone());
        handles.extend(
            thread::Builder::new()
                .name("xinput".into())
                .spawn(move || run_xinput(&p, &s, out))
                .ok(),
        );
        let (p, s, out) = (paused.clone(), stop.clone(), sink);
        handles.extend(
            thread::Builder::new()
                .name("dualsense".into())
                .spawn(move || run_dualsense(&p, &s, out))
                .ok(),
        );

        Self {
            paused,
            stop,
            handles,
        }
    }

    /// Paused threads stop reading devices (overlay hidden or no gamepad widget).
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }
}

impl Drop for GamepadThreads {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
    }
}

const IDLE_SLEEP: Duration = Duration::from_millis(100);

fn run_xinput(paused: &AtomicBool, stop: &AtomicBool, sink: impl Fn(PadEvent)) {
    // No default filters: raw values without deadzones.
    let mut gilrs = match GilrsBuilder::new().with_default_filters(false).build() {
        Ok(g) => g,
        Err(e) => {
            tracing::warn!(error = %e, "XInput unavailable");
            return;
        }
    };
    let mut last: std::collections::HashMap<GamepadId, GamepadState> = Default::default();

    while !stop.load(Ordering::Relaxed) {
        if paused.load(Ordering::Relaxed) {
            while gilrs.next_event().is_some() {}
            if !last.is_empty() {
                for id in last.keys() {
                    sink(PadEvent {
                        pad: xinput_pad(*id),
                        state: GamepadState::default(),
                        connected: false,
                    });
                }
                last.clear();
            }
            thread::sleep(IDLE_SLEEP);
            continue;
        }

        let mut next = gilrs.next_event_blocking(Some(IDLE_SLEEP));
        while let Some(event) = next {
            if let EventType::Disconnected = event.event
                && last.remove(&event.id).is_some()
            {
                tracing::info!(index = usize::from(event.id), "XInput pad disconnected");
                sink(PadEvent {
                    pad: xinput_pad(event.id),
                    state: GamepadState::default(),
                    connected: false,
                });
            }
            next = gilrs.next_event();
        }

        for (id, pad) in gilrs.gamepads() {
            let trigger = |b| pad.button_data(b).map_or(0.0, |d| d.value());
            let mut bits = 0;
            for (button, bit) in XINPUT_BUTTONS {
                if pad.is_pressed(button) {
                    bits |= bit;
                }
            }
            let state = GamepadState {
                buttons: bits,
                left_stick: (pad.value(Axis::LeftStickX), pad.value(Axis::LeftStickY)),
                right_stick: (pad.value(Axis::RightStickX), pad.value(Axis::RightStickY)),
                left_trigger: trigger(Button::LeftTrigger2),
                right_trigger: trigger(Button::RightTrigger2),
            };
            if last.get(&id) != Some(&state) {
                if !last.contains_key(&id) {
                    tracing::info!(
                        name = pad.name(),
                        index = usize::from(id),
                        "XInput pad connected"
                    );
                }
                last.insert(id, state);
                sink(PadEvent {
                    pad: xinput_pad(id),
                    state,
                    connected: true,
                });
            }
        }
    }
}

fn xinput_pad(id: GamepadId) -> PadId {
    PadId {
        source: PadSource::XInput,
        index: usize::from(id),
    }
}

const XINPUT_BUTTONS: [(Button, u16); 14] = [
    (Button::South, buttons::A),
    (Button::East, buttons::B),
    (Button::West, buttons::X),
    (Button::North, buttons::Y),
    (Button::LeftTrigger, buttons::LB),
    (Button::RightTrigger, buttons::RB),
    (Button::Select, buttons::BACK),
    (Button::Start, buttons::START),
    (Button::LeftThumb, buttons::L3),
    (Button::RightThumb, buttons::R3),
    (Button::DPadUp, buttons::DPAD_UP),
    (Button::DPadDown, buttons::DPAD_DOWN),
    (Button::DPadLeft, buttons::DPAD_LEFT),
    (Button::DPadRight, buttons::DPAD_RIGHT),
];

const SONY_VID: u16 = 0x054C;
const DUALSENSE_PIDS: [u16; 2] = [0x0CE6, 0x0DF2]; // DualSense, DualSense Edge
const RESCAN: Duration = Duration::from_secs(2);
const DUALSENSE_PAD: PadId = PadId {
    source: PadSource::DualSense,
    index: 0,
};

fn run_dualsense(paused: &AtomicBool, stop: &AtomicBool, sink: impl Fn(PadEvent)) {
    let mut api: Option<hidapi::HidApi> = None;
    let mut device: Option<hidapi::HidDevice> = None;
    let mut last: Option<GamepadState> = None;
    let mut last_scan: Option<Instant> = None;
    let mut buf = [0u8; 128];

    let disconnect = |last: &mut Option<GamepadState>| {
        if last.take().is_some() {
            sink(PadEvent {
                pad: DUALSENSE_PAD,
                state: GamepadState::default(),
                connected: false,
            });
        }
    };

    while !stop.load(Ordering::Relaxed) {
        if paused.load(Ordering::Relaxed) {
            device = None;
            disconnect(&mut last);
            thread::sleep(IDLE_SLEEP);
            continue;
        }
        let Some(dev) = &device else {
            if last_scan.is_some_and(|t| t.elapsed() < RESCAN) {
                thread::sleep(IDLE_SLEEP);
                continue;
            }
            last_scan = Some(Instant::now());
            device = open_dualsense(&mut api);
            continue;
        };
        match dev.read_timeout(&mut buf, IDLE_SLEEP.as_millis() as i32) {
            Ok(0) => {}
            Ok(n) => {
                if let Some(state) = parse_dualsense(&buf[..n])
                    && last != Some(state)
                {
                    last = Some(state);
                    sink(PadEvent {
                        pad: DUALSENSE_PAD,
                        state,
                        connected: true,
                    });
                }
            }
            Err(e) => {
                tracing::info!(error = %e, "DualSense disconnected");
                device = None;
                disconnect(&mut last);
            }
        }
    }
}

fn open_dualsense(api: &mut Option<hidapi::HidApi>) -> Option<hidapi::HidDevice> {
    match api {
        Some(a) => {
            let _ = a.refresh_devices();
        }
        None => {
            *api = hidapi::HidApi::new()
                .inspect_err(|e| tracing::warn!(error = %e, "hidapi init"))
                .ok()
        }
    }
    let api = api.as_ref()?;
    let info = api
        .device_list()
        .find(|d| d.vendor_id() == SONY_VID && DUALSENSE_PIDS.contains(&d.product_id()))?;
    let device = info
        .open_device(api)
        .inspect_err(|e| tracing::warn!(error = %e, "open DualSense"))
        .ok()?;
    tracing::info!(product = ?info.product_string(), "DualSense connected");
    Some(device)
}

/// DualSense input report → state. USB report 0x01, Bluetooth full report 0x31, Bluetooth simple 0x01.
pub fn parse_dualsense(report: &[u8]) -> Option<GamepadState> {
    // Common layout: LX LY RX RY L2 R2 seq buttons0 buttons1 buttons2
    let (sticks, triggers, b0, b1) = match (*report.first()?, report.len()) {
        (0x01, n) if n >= 64 => (&report[1..5], (report[5], report[6]), report[8], report[9]),
        (0x31, n) if n >= 12 => (&report[2..6], (report[6], report[7]), report[9], report[10]),
        (0x01, n) if n >= 10 => (&report[1..5], (report[8], report[9]), report[5], report[6]),
        _ => return None,
    };

    let mut bits = match b0 & 0x0F {
        0 => buttons::DPAD_UP,
        1 => buttons::DPAD_UP | buttons::DPAD_RIGHT,
        2 => buttons::DPAD_RIGHT,
        3 => buttons::DPAD_DOWN | buttons::DPAD_RIGHT,
        4 => buttons::DPAD_DOWN,
        5 => buttons::DPAD_DOWN | buttons::DPAD_LEFT,
        6 => buttons::DPAD_LEFT,
        7 => buttons::DPAD_UP | buttons::DPAD_LEFT,
        _ => 0,
    };
    // Square/Cross/Circle/Triangle → X/A/B/Y like the XInput mapping.
    for (mask, bit) in [
        (0x10, buttons::X),
        (0x20, buttons::A),
        (0x40, buttons::B),
        (0x80, buttons::Y),
    ] {
        if b0 & mask != 0 {
            bits |= bit;
        }
    }
    // L1 R1 (L2 R2 digital skipped) Create Options L3 R3
    for (mask, bit) in [
        (0x01, buttons::LB),
        (0x02, buttons::RB),
        (0x10, buttons::BACK),
        (0x20, buttons::START),
        (0x40, buttons::L3),
        (0x80, buttons::R3),
    ] {
        if b1 & mask != 0 {
            bits |= bit;
        }
    }

    let axis = |v: u8| ((v as f32 - 128.0) / 127.0).clamp(-1.0, 1.0);
    Some(GamepadState {
        buttons: bits,
        left_stick: (axis(sticks[0]), -axis(sticks[1])),
        right_stick: (axis(sticks[2]), -axis(sticks[3])),
        left_trigger: triggers.0 as f32 / 255.0,
        right_trigger: triggers.1 as f32 / 255.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usb_report() -> [u8; 64] {
        let mut r = [0u8; 64];
        r[0] = 0x01;
        r[1..5].copy_from_slice(&[128, 128, 128, 128]);
        r[8] = 0x08; // hat neutral
        r
    }

    #[test]
    fn usb_neutral_is_idle() {
        let s = parse_dualsense(&usb_report()).unwrap();
        assert_eq!(s.buttons, 0);
        assert!(s.left_stick.0.abs() < 0.01 && s.left_stick.1.abs() < 0.01);
    }

    #[test]
    fn usb_buttons_sticks_triggers() {
        let mut r = usb_report();
        r[1] = 255; // LX right
        r[2] = 0; // LY up
        r[5] = 255; // L2
        r[8] = 0x20 | 0x03; // cross + hat down-right
        r[9] = 0x01 | 0x40; // L1 + L3
        let s = parse_dualsense(&r).unwrap();
        assert_eq!(
            s.buttons,
            buttons::A | buttons::DPAD_DOWN | buttons::DPAD_RIGHT | buttons::LB | buttons::L3
        );
        assert_eq!(s.left_stick, (1.0, 1.0), "right and up are positive");
        assert_eq!(s.left_trigger, 1.0);
    }

    #[test]
    fn bluetooth_full_report_is_offset_by_one() {
        let mut r = [0u8; 78];
        r[0] = 0x31;
        r[2..6].copy_from_slice(&[128, 128, 128, 128]);
        r[9] = 0x08 | 0x80; // triangle
        let s = parse_dualsense(&r).unwrap();
        assert_eq!(s.buttons, buttons::Y);
    }

    #[test]
    fn unknown_report_is_ignored() {
        assert!(parse_dualsense(&[0x05, 1, 2]).is_none());
        assert!(parse_dualsense(&[]).is_none());
    }
}

#[cfg(test)]
mod live {
    use super::*;
    use std::sync::Mutex;

    /// Starts both readers for 3 s against real hardware: `cargo test gamepad_readers -- --ignored --nocapture`.
    #[test]
    #[ignore = "uses real devices"]
    fn gamepad_readers() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let events = events.clone();
            move |e: PadEvent| events.lock().unwrap().push(e)
        };
        let threads = GamepadThreads::spawn(sink);
        thread::sleep(Duration::from_secs(3));
        threads.set_paused(true);
        thread::sleep(Duration::from_millis(300));
        let stop = Instant::now();
        drop(threads);
        let stop_ms = stop.elapsed().as_millis();

        let events = events.lock().unwrap();
        let pads: std::collections::HashSet<_> = events.iter().map(|e| e.pad).collect();
        println!(
            "events: {}, pads: {pads:?}, stop took {stop_ms} ms",
            events.len()
        );
        assert!(stop_ms < 1000, "threads must stop promptly");
    }
}
