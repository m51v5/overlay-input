//! "Start with Windows": a per-user Run entry holding this exe's path.

use std::path::Path;

use anyhow::{Context, Result};
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, w};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("Overlay Input");

/// On only if the entry starts this exe; a copy moved elsewhere reads as off.
pub fn is_enabled() -> bool {
    let (Ok(exe), Some(current)) = (std::env::current_exe(), read()) else {
        return false;
    };
    current == run_command(&exe)
}

pub fn set(enabled: bool) -> Result<()> {
    if enabled {
        let exe = std::env::current_exe().context("find own exe")?;
        let data: Vec<u16> = run_command(&exe).encode_utf16().chain([0]).collect();
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE,
                REG_SZ.0,
                Some(data.as_ptr().cast()),
                (data.len() * size_of::<u16>()) as u32,
            )
        }
        .ok()
        .context("write the Windows startup entry")
    } else {
        let result = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE) };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        result.ok().context("remove the Windows startup entry")
    }
}

fn run_command(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

fn read() -> Option<String> {
    let mut buffer = [0u16; 1024];
    let mut size = size_of_val(&buffer) as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    }
    .ok()
    .ok()?;
    let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..len]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes this test exe to the real Run key, then removes it.
    #[test]
    #[ignore = "changes the user's registry for a moment"]
    fn round_trip_in_the_registry() {
        assert!(read().is_none(), "a real entry exists; not touching it");
        set(true).unwrap();
        let enabled = is_enabled();
        set(false).unwrap();
        assert!(enabled);
        assert!(read().is_none());
        set(false).unwrap();
    }

    #[test]
    fn paths_with_spaces_are_quoted() {
        assert_eq!(
            run_command(Path::new("C:\\My Apps\\overlay_input.exe")),
            "\"C:\\My Apps\\overlay_input.exe\""
        );
    }
}
