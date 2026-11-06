//! Plain Win32 layered popup: winit rewrites ex-styles on flag changes, so we own this window.

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use tiny_skia::Pixmap;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, TRUE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowW, HTTRANSPARENT, MA_NOACTIVATE,
    MONITORINFOF_PRIMARY, PostMessageW, RegisterClassExW, SPI_SETWORKAREA, SW_HIDE,
    SW_SHOWNOACTIVATE, SetWindowDisplayAffinity, ShowWindow, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
    WM_APP, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_MOUSEACTIVATE, WM_NCHITTEST, WM_SETTINGCHANGE,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::BOOL;
use windows::core::{PCWSTR, w};

use super::present::LayeredSurface;
use crate::profile::Anchor;

const CLASS_NAME: PCWSTR = w!("OverlayInputWindow");

/// Set by the window procedure when resolution, DPI or the taskbar (work area) changes.
static DISPLAY_CHANGED: AtomicBool = AtomicBool::new(false);

/// True once after a display change; the app then re-places the overlay.
pub fn take_display_changed() -> bool {
    DISPLAY_CHANGED.swap(false, Ordering::Relaxed)
}

/// Posted by a second launch: open the app window.
const WM_OPEN_WINDOW_REQUEST: u32 = WM_APP + 1;
static OPEN_WINDOW_REQUESTED: AtomicBool = AtomicBool::new(false);

/// True once after another launch asked for the app window.
pub fn take_open_window_requested() -> bool {
    OPEN_WINDOW_REQUESTED.swap(false, Ordering::Relaxed)
}

/// From a second launch: asks the running overlay to open the app window.
pub fn request_open_window() {
    unsafe {
        if let Ok(hwnd) = FindWindowW(CLASS_NAME, None) {
            let _ = PostMessageW(Some(hwnd), WM_OPEN_WINDOW_REQUEST, WPARAM(0), LPARAM(0));
        }
    }
}

pub struct OverlayWindow {
    hwnd: HWND,
    surface: LayeredSurface,
    work_area: RECT,
    scale: f32,
    /// What the user asked for: the overlay shows on the screen.
    visible: bool,
    /// Whether the window is actually on screen.
    shown: bool,
}

impl OverlayWindow {
    /// Creates the hidden window on the primary monitor; call `present` to show it.
    pub fn create() -> Result<Self> {
        let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
        let work_area = monitor_work_area(monitor)?;
        let scale = monitor_scale(monitor);

        let hwnd = unsafe {
            let instance = GetModuleHandleW(None).context("GetModuleHandleW")?;
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance.into(),
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            if RegisterClassExW(&class) == 0 {
                anyhow::bail!("RegisterClassExW: {}", windows::core::Error::from_thread());
            }

            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
                CLASS_NAME,
                w!("Overlay Input"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .context("CreateWindowExW")?
        };

        tracing::info!(?work_area, scale, "overlay window created");
        Ok(Self {
            hwnd,
            surface: LayeredSurface::new(),
            work_area,
            scale,
            visible: true,
            shown: false,
        })
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Hides immediately; showing waits for the next `present` so stale pixels never flash.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if !visible && self.shown {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.shown = false;
        }
    }

    /// Keeps the overlay out of screenshots, screen recordings and screen sharing (Windows 10 2004+).
    pub fn set_hidden_from_capture(&self, hidden: bool) {
        let affinity = if hidden {
            WDA_EXCLUDEFROMCAPTURE
        } else {
            WDA_NONE
        };
        if let Err(e) = unsafe { SetWindowDisplayAffinity(self.hwnd, affinity) } {
            tracing::warn!(error = %e, hidden, "could not change capture visibility");
        }
    }

    /// Physical pixels per logical pixel on the overlay's monitor.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Moves to monitor `index` (0 = primary, then left to right); falls back to primary.
    pub fn set_monitor(&mut self, index: usize) -> Result<()> {
        let monitors = monitors();
        let monitor = match monitors.get(index) {
            Some(m) => *m,
            None => {
                tracing::warn!(
                    index,
                    available = monitors.len(),
                    "monitor not found, using primary"
                );
                unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }
            }
        };
        self.work_area = monitor_work_area(monitor)?;
        self.scale = monitor_scale(monitor);
        tracing::debug!(index, work_area = ?self.work_area, scale = self.scale, "monitor selected");
        Ok(())
    }

    /// Places `pixmap` at `anchor` of the work area, `offset` in physical px; `highlight` blends under it.
    pub fn present(
        &mut self,
        pixmap: &Pixmap,
        highlight: Option<(&Pixmap, f32)>,
        anchor: Anchor,
        offset: (i32, i32),
        opacity: f32,
    ) -> Result<()> {
        let area = &self.work_area;
        let (x, y) = anchor.position(
            (area.left, area.top, area.right, area.bottom),
            (pixmap.width() as i32, pixmap.height() as i32),
            offset,
        );
        self.surface
            .present(self.hwnd, POINT { x, y }, pixmap, opacity, highlight)?;

        if self.visible && !self.shown {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
            self.shown = true;
        }
        Ok(())
    }
}

impl Drop for OverlayWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            DISPLAY_CHANGED.store(true, Ordering::Relaxed);
            LRESULT(0)
        }
        WM_SETTINGCHANGE if wparam.0 as u32 == SPI_SETWORKAREA.0 => {
            DISPLAY_CHANGED.store(true, Ordering::Relaxed);
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_OPEN_WINDOW_REQUEST => {
            OPEN_WINDOW_REQUESTED.store(true, Ordering::Relaxed);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Monitor as shown in the app window (physical px).
#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub index: usize,
    pub primary: bool,
    /// left, top, right, bottom of the whole monitor.
    pub bounds: [i32; 4],
    /// Same, minus the taskbar.
    pub work_area: [i32; 4],
    pub scale: f32,
}

/// All monitors in profile `monitor` index order.
pub fn monitor_infos() -> Vec<MonitorInfo> {
    monitors()
        .into_iter()
        .enumerate()
        .map(|(index, m)| {
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = unsafe { GetMonitorInfoW(m, &mut info) };
            let rect = |r: RECT| [r.left, r.top, r.right, r.bottom];
            MonitorInfo {
                index,
                primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
                bounds: rect(info.rcMonitor),
                work_area: rect(info.rcWork),
                scale: monitor_scale(m),
            }
        })
        .collect()
}

/// Primary first, then the rest ordered left to right.
fn monitors() -> Vec<HMONITOR> {
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        list.push(monitor);
        TRUE
    }
    let mut list: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut list as *mut _ as isize),
        );
    }

    let info = |m: HMONITOR| {
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = unsafe { GetMonitorInfoW(m, &mut info) };
        info
    };
    list.sort_by_key(|m| {
        let i = info(*m);
        (i.dwFlags & MONITORINFOF_PRIMARY == 0, i.rcMonitor.left)
    });
    list
}

fn monitor_work_area(monitor: HMONITOR) -> Result<RECT> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetMonitorInfoW(monitor, &mut info) }
        .ok()
        .context("GetMonitorInfoW")?;
    Ok(info.rcWork)
}

fn monitor_scale(monitor: HMONITOR) -> f32 {
    let (mut dpi_x, mut dpi_y) = (96, 96);
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.is_err() {
        tracing::warn!("GetDpiForMonitor failed, assuming 100% scale");
    }
    dpi_x as f32 / 96.0
}
