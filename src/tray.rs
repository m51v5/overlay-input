//! Tray icon: quick controls and the way back to the app window, which has everything else.

use anyhow::{Context, Result};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

/// tray-icon calls `SetForegroundWindow` right after this event; Windows ignores that call when another
/// app owns the foreground, and the menu then opens unfocused, sometimes under the taskbar.
/// Attaching to the foreground thread's input makes the call succeed. Returns the thread to detach later.
pub fn on_tray_icon_event(event: &TrayIconEvent) -> Option<u32> {
    let TrayIconEvent::Click {
        button: MouseButton::Right,
        button_state: MouseButtonState::Up,
        ..
    } = event
    else {
        return None;
    };
    unsafe {
        let foreground = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let current = GetCurrentThreadId();
        (foreground != 0
            && foreground != current
            && AttachThreadInput(current, foreground, true).as_bool())
        .then_some(foreground)
    }
}

/// Undoes `on_tray_icon_event` once the menu has taken focus.
pub fn detach_input(thread: u32) {
    unsafe {
        let _ = AttachThreadInput(GetCurrentThreadId(), thread, false);
    }
}

/// Double-click on the icon opens the app window.
pub fn is_open_request(event: &TrayIconEvent) -> bool {
    matches!(
        event,
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        }
    )
}

pub enum TrayAction {
    SelectProfile(usize),
    OpenWindow,
    ToggleHidden,
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    profiles_menu: Submenu,
    profile_items: Vec<CheckMenuItem>,
    open: MenuItem,
    show_hide: MenuItem,
    quit: MenuItem,
    profile_name: Option<String>,
    /// An unsaved profile is on screen.
    previewing: bool,
    hidden: bool,
    hotkey_hint: Option<String>,
}

impl Tray {
    /// Must run on the event-loop thread after the loop has started.
    pub fn create() -> Result<Self> {
        let open = MenuItem::new("Open", true, None);
        let profiles_menu = Submenu::new("Profile", true);
        let show_hide = MenuItem::new("Hide", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &open,
            &profiles_menu,
            &show_hide,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .context("tray menu")?;

        // Resource 1 = assets/windows/app.ico embedded by build.rs.
        let icon = Icon::from_resource(1, None).context("tray icon resource")?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("Overlay Input")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            // Otherwise the first click of a double-click opens the menu.
            .with_menu_on_left_click(false)
            .build()
            .context("tray icon")?;

        Ok(Self {
            icon: tray,
            profiles_menu,
            profile_items: Vec::new(),
            open,
            show_hide,
            quit,
            profile_name: None,
            previewing: false,
            hidden: false,
            hotkey_hint: None,
        })
    }

    pub fn action(&self, event: &MenuEvent) -> Option<TrayAction> {
        if let Some(i) = self
            .profile_items
            .iter()
            .position(|item| *item.id() == event.id)
        {
            Some(TrayAction::SelectProfile(i))
        } else if event.id == *self.open.id() {
            Some(TrayAction::OpenWindow)
        } else if event.id == *self.show_hide.id() {
            Some(TrayAction::ToggleHidden)
        } else if event.id == *self.quit.id() {
            Some(TrayAction::Quit)
        } else {
            None
        }
    }

    /// Replaces the Profile submenu entries.
    pub fn set_profiles(&mut self, names: &[String], active: Option<usize>) {
        for item in self.profile_items.drain(..) {
            let _ = self.profiles_menu.remove(&item);
        }
        for (i, name) in names.iter().enumerate() {
            let item = CheckMenuItem::new(name, true, Some(i) == active, None);
            if let Err(e) = self.profiles_menu.append(&item) {
                tracing::warn!(error = %e, name, "tray profile item");
            }
            self.profile_items.push(item);
        }
    }

    /// Check items toggle themselves on click, so re-sync all of them.
    pub fn set_active_profile(&self, active: Option<usize>) {
        for (i, item) in self.profile_items.iter().enumerate() {
            item.set_checked(Some(i) == active);
        }
    }

    pub fn set_profile_name(&mut self, name: &str) {
        self.profile_name = Some(name.to_owned());
        self.refresh_tooltip();
    }

    pub fn set_previewing(&mut self, previewing: bool) {
        self.previewing = previewing;
        self.refresh_tooltip();
    }

    fn refresh_tooltip(&self) {
        let mut text = String::from("Overlay Input");
        if let Some(name) = &self.profile_name {
            text.push_str(" — ");
            text.push_str(name);
        }
        if self.previewing {
            text.push_str(" · preview");
        }
        if let Err(e) = self.icon.set_tooltip(Some(text)) {
            tracing::warn!(error = %e, "tray tooltip");
        }
    }

    pub fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
        self.refresh_show_hide();
    }

    /// Shows the show/hide hotkey right-aligned next to the item, e.g. "Ctrl+Alt+F12".
    pub fn set_hotkey_hint(&mut self, hint: Option<&str>) {
        self.hotkey_hint = hint.map(str::to_owned);
        self.refresh_show_hide();
    }

    fn refresh_show_hide(&self) {
        let label = if self.hidden { "Show" } else { "Hide" };
        match &self.hotkey_hint {
            Some(hint) => self.show_hide.set_text(format!("{label}\t{hint}")),
            None => self.show_hide.set_text(label),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tray_icon::dpi::PhysicalPosition;
    use tray_icon::{Rect, TrayIconId};

    fn double_click(button: MouseButton) -> TrayIconEvent {
        TrayIconEvent::DoubleClick {
            id: TrayIconId::new("test"),
            position: PhysicalPosition::new(0.0, 0.0),
            rect: Rect::default(),
            button,
        }
    }

    #[test]
    fn only_a_left_double_click_opens_the_window() {
        assert!(is_open_request(&double_click(MouseButton::Left)));
        assert!(!is_open_request(&double_click(MouseButton::Right)));
    }
}
