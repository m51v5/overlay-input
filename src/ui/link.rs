//! Overlay ↔ window process: one JSON message per line over the child's stdin and stdout.
//! A closed pipe means the other side is gone, crash included.

use std::io::{self, BufRead, BufReader, Write};
use std::process::{ChildStdin, Command, Stdio};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::settings::Display;

/// Window → overlay; each one gets a `ToWindow::Reply`, in order.
#[derive(Debug, Serialize, Deserialize)]
pub enum ToOverlay {
    /// Show this unsaved profile (TOML) until `EndPreview`.
    Preview(String),
    EndPreview,
    /// Show this profile.
    Activate(String),
    /// Register and save the show/hide hotkey; empty = none.
    SetHotkey(String),
    SetDisplay(Display),
    SetWindowBackground([u8; 3]),
    SetHideFromCapture(bool),
    SetShowWindowAtStart(bool),
    /// Quit the whole app; the window has already asked about unsaved changes.
    Quit,
}

/// Overlay → window. The first message is always `Status`.
#[derive(Debug, Serialize, Deserialize)]
pub enum ToWindow {
    /// Sent when any of it changes, and when the profile list may have.
    Status(Status),
    Reply(Result<(), String>),
    /// Come to the front.
    Focus,
    /// Tray Quit: ask about unsaved changes, then answer with `ToOverlay::Quit`.
    Quit,
}

/// What the window can't work out by itself.
#[derive(Debug, Serialize, Deserialize)]
pub struct Status {
    pub active: Option<String>,
    /// An unsaved profile is on the overlay.
    pub previewing: bool,
    /// `toggle_hotkey` as set, even when it couldn't be registered.
    pub hotkey: String,
    pub hotkey_registered: bool,
    pub display: Display,
    pub hide_from_capture: bool,
    pub window_background: [u8; 3],
    pub show_window_at_start: bool,
}

pub fn send<T: Serialize>(mut pipe: impl Write, message: &T) -> io::Result<()> {
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    pipe.write_all(&line)?;
    pipe.flush()
}

/// Next message; `None` once the pipe is closed.
pub fn receive<T: DeserializeOwned>(
    lines: &mut impl Iterator<Item = io::Result<String>>,
) -> Option<T> {
    loop {
        let line = lines.next()?.ok()?;
        match serde_json::from_str(&line) {
            Ok(message) => return Some(message),
            Err(e) => tracing::warn!(error = %e, "bad message from the other process"),
        }
    }
}

/// The overlay's handle on the running window process. Dropping it closes the pipe, which ends the process.
pub struct WindowProcess {
    stdin: ChildStdin,
}

impl WindowProcess {
    /// Starts `overlay_input.exe --ui`; `on_message` runs on a reader thread, with `None` when the process ends.
    pub fn spawn(mut on_message: impl FnMut(Option<ToOverlay>) + Send + 'static) -> Result<Self> {
        let exe = std::env::current_exe().context("find own exe")?;
        let mut child = Command::new(exe)
            .arg("--ui")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("start the app window process")?;
        let stdin = child.stdin.take().context("window stdin")?;
        let stdout = child.stdout.take().context("window stdout")?;
        std::thread::Builder::new()
            .name("window-link".into())
            .spawn(move || {
                let mut lines = BufReader::new(stdout).lines();
                while let Some(message) = receive(&mut lines) {
                    on_message(Some(message));
                }
                on_message(None);
            })
            .context("start the window reader")?;
        Ok(Self { stdin })
    }

    /// A failed write needs no handling: the reader thread sees the same closed pipe.
    pub fn send(&self, message: &ToWindow) {
        if let Err(e) = send(&self.stdin, message) {
            tracing::debug!(error = %e, "app window not reachable");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_survive_the_pipe_format() {
        let mut pipe = Vec::new();
        send(
            &mut pipe,
            &ToOverlay::Preview("name = \"a\"\n[layout]\n".into()),
        )
        .unwrap();
        send(&mut pipe, &ToWindow::Reply(Err("line 1\nline 2".into()))).unwrap();
        assert_eq!(
            pipe.iter().filter(|b| **b == b'\n').count(),
            2,
            "one line each"
        );

        let mut lines = std::io::Cursor::new(pipe).lines();
        match receive(&mut lines) {
            Some(ToOverlay::Preview(text)) => assert_eq!(text, "name = \"a\"\n[layout]\n"),
            other => panic!("{other:?}"),
        }
        match receive(&mut lines) {
            Some(ToWindow::Reply(Err(e))) => assert_eq!(e, "line 1\nline 2"),
            other => panic!("{other:?}"),
        }
        assert!(receive::<ToWindow>(&mut lines).is_none(), "closed pipe");
    }

    #[test]
    fn junk_lines_are_skipped() {
        let mut lines = std::io::Cursor::new(b"not json\n\"Focus\"\n".to_vec()).lines();
        assert!(matches!(receive(&mut lines), Some(ToWindow::Focus)));
    }
}
