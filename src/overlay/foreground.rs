//! Foreground watcher: re-asserts topmost when another window comes forward (games often grab the top),
//! and warns once per app when an elevated (admin) app is focused but the overlay isn't elevated.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MENUPOPUPSTART, GetWindowThreadProcessId, HWND_TOPMOST,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, WINEVENT_OUTOFCONTEXT,
};
use windows::core::PWSTR;

static OVERLAY: AtomicIsize = AtomicIsize::new(0);

thread_local! {
    static SELF_ELEVATED: bool = process_elevated(None).unwrap_or(false);
    static WARNED: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
}

/// Active while alive; must be created on the thread that runs the message loop.
pub struct ForegroundWatcher {
    hook: HWINEVENTHOOK,
    /// Our own popup menus (the tray menu).
    menu_hook: HWINEVENTHOOK,
}

impl ForegroundWatcher {
    pub fn install(overlay: HWND) -> Option<Self> {
        OVERLAY.store(overlay.0 as isize, Ordering::Relaxed);
        if !SELF_ELEVATED.with(|e| *e) {
            tracing::info!(
                "not running as administrator: input from games running as admin won't reach the overlay"
            );
        }
        let hook = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(on_foreground),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };
        if hook.is_invalid() {
            tracing::warn!("foreground hook failed; overlay may fall behind fullscreen apps");
            return None;
        }
        let menu_hook = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_MENUPOPUPSTART,
                EVENT_SYSTEM_MENUPOPUPSTART,
                None,
                Some(on_menu_popup),
                GetCurrentProcessId(),
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };
        if menu_hook.is_invalid() {
            tracing::warn!("menu hook failed; the tray menu may open behind the taskbar");
        }
        Some(Self { hook, menu_hook })
    }
}

impl Drop for ForegroundWatcher {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWinEvent(self.hook);
            if !self.menu_hook.is_invalid() {
                let _ = UnhookWinEvent(self.menu_hook);
            }
        }
        OVERLAY.store(0, Ordering::Relaxed);
    }
}

unsafe extern "system" fn on_foreground(
    _: HWINEVENTHOOK,
    _: u32,
    window: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    let overlay = HWND(OVERLAY.load(Ordering::Relaxed) as *mut _);
    // Our own windows (the tray menu's) come forward too; raising the overlay then would cover the menu.
    if !overlay.is_invalid() && overlay != window && !is_own_window(window) {
        unsafe {
            let _ = SetWindowPos(
                overlay,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
    warn_if_elevated(window);
}

/// A tray menu opened while another app keeps the foreground lands under the topmost taskbar;
/// lifting the menu itself to the top works whether or not our foreground request was granted.
unsafe extern "system" fn on_menu_popup(
    _: HWINEVENTHOOK,
    _: u32,
    menu: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    unsafe {
        let _ = SetWindowPos(
            menu,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

fn is_own_window(window: HWND) -> bool {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    pid == unsafe { GetCurrentProcessId() }
}

fn warn_if_elevated(window: HWND) {
    if SELF_ELEVATED.with(|e| *e) {
        return;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    if pid == 0 || WARNED.with(|w| w.borrow().contains(&pid)) {
        return;
    }
    if process_elevated(Some(pid)) == Some(true) {
        WARNED.with(|w| w.borrow_mut().insert(pid));
        let name = process_name(pid).unwrap_or_else(|| format!("pid {pid}"));
        tracing::warn!(
            app = %name,
            "focused app runs as administrator; Windows won't send its input to this overlay. Run Overlay Input as administrator to show it."
        );
    }
}

/// Elevation of `pid`, or of this process when `None`.
fn process_elevated(pid: Option<u32>) -> Option<bool> {
    unsafe {
        let process = match pid {
            Some(pid) => OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?,
            None => GetCurrentProcess(),
        };
        let result = token_elevated(process);
        if pid.is_some() {
            let _ = CloseHandle(process);
        }
        result
    }
}

unsafe fn token_elevated(process: HANDLE) -> Option<bool> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        let _ = CloseHandle(token);
        ok.ok()?;
        Some(elevation.TokenIsElevated != 0)
    }
}

fn process_name(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        ok.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_elevation_is_readable() {
        assert!(process_elevated(None).is_some());
        assert!(process_name(std::process::id()).is_some_and(|n| n.ends_with(".exe")));
    }
}
