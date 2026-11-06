//! The click-through, always-on-top overlay window and pixel presentation.

mod color_window;
mod foreground;
mod present;
mod window;

pub use color_window::ColorWindow;
pub use foreground::ForegroundWatcher;
pub use window::{
    MonitorInfo, OverlayWindow, monitor_infos, request_open_window, take_display_changed,
    take_open_window_requested,
};
