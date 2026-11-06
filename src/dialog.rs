//! Native dialogs for the few moments the overlay must ask before going on.

use windows::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, MessageBoxW,
};
use windows::core::HSTRING;

/// Error with an OK button. Blocks until closed.
pub fn error(title: &str, text: &str) {
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from(title),
            MB_OK | MB_ICONERROR | MB_TOPMOST | MB_SETFOREGROUND,
        );
    }
}

/// Warning with Yes / No buttons; true for Yes. Blocks until answered.
pub fn ask_yes_no(title: &str, text: &str) -> bool {
    let answer = unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from(title),
            MB_YESNO | MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND,
        )
    };
    answer == IDYES
}
