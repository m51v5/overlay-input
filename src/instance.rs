//! One overlay per user session: a second launch opens the running one's window instead of another overlay.

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::w;

/// True for the first instance. The lock is a named mutex held until the process exits.
pub fn claim() -> bool {
    match unsafe { CreateMutexW(None, false, w!("Local\\OverlayInput.SingleInstance")) } {
        // The handle is deliberately never closed: Windows releases it when the process ends.
        Ok(_) => (unsafe { GetLastError() }) != ERROR_ALREADY_EXISTS,
        // Can't tell: better two overlays than none.
        Err(_) => true,
    }
}
