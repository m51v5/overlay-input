//! App files next to `profiles/`: `settings.toml` (user-edited) and `state.toml` (written by the app).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const SETTINGS_FILE: &str = "settings.toml";
const STATE_FILE: &str = "state.toml";
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+F12";
const DEFAULT_SETTINGS: &str = r#"# Overlay Input settings. Hand edits apply after a restart; the app window applies them at once.

# Global hotkey to show/hide the overlay. Modifiers: Ctrl, Alt, Shift, Super.
# Keys: A-Z, 0-9, F1-F24, ... (e.g. "Ctrl+Alt+F12", "Shift+F9"). Empty = no hotkey.
toggle_hotkey = "Ctrl+Alt+F12"

# Open the app window when Overlay Input starts.
show_window_at_start = false
"#;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Settings {
    pub toggle_hotkey: String,
    pub show_window_at_start: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            toggle_hotkey: DEFAULT_HOTKEY.to_owned(),
            show_window_at_start: false,
        }
    }
}

impl Settings {
    /// Loads `settings.toml` from `dir`, creating it with defaults if missing; bad files fall back to defaults.
    pub fn load_or_create(dir: &Path) -> Self {
        let path = dir.join(SETTINGS_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                tracing::error!(?path, "invalid settings, using defaults: {e}");
                Self::default()
            }),
            Err(_) => {
                if let Err(e) = std::fs::write(&path, DEFAULT_SETTINGS) {
                    tracing::warn!(?path, error = %e, "could not create settings file");
                }
                Self::default()
            }
        }
    }

    /// Writes `settings.toml` (the commented template with these values).
    pub fn save(&self, dir: &Path) -> Result<()> {
        let value = toml::Value::String(self.toggle_hotkey.clone()).to_string();
        let text = DEFAULT_SETTINGS
            .replace(
                &format!("toggle_hotkey = \"{DEFAULT_HOTKEY}\""),
                &format!("toggle_hotkey = {value}"),
            )
            .replace(
                "show_window_at_start = false",
                &format!("show_window_at_start = {}", self.show_window_at_start),
            );
        std::fs::write(dir.join(SETTINGS_FILE), text).context("write settings.toml")
    }
}

/// Remembered between runs; rewritten by the app.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct State {
    /// Profile file name in `profiles/`.
    pub last_profile: Option<String>,
    /// Where the overlay shows (Hide isn't remembered: it starts shown).
    pub output: Output,
    /// Keep the on-screen overlay out of screenshots and recordings.
    pub hide_from_capture: bool,
    /// Overlay window client size, physical px.
    pub window_size: Option<[u32; 2]>,
    /// Color behind the overlay in its window. Magenta by default: pressed keys and the movement
    /// ring are green, so a green key would remove them.
    pub window_background: [u8; 3],
}

impl Default for State {
    fn default() -> Self {
        Self {
            last_profile: None,
            output: Output::default(),
            hide_from_capture: false,
            window_size: None,
            window_background: [255, 0, 255],
        }
    }
}

/// Where the overlay shows while it isn't hidden.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Output {
    #[default]
    Screen,
    Window,
}

/// Where the overlay shows right now; only `Hidden` stops reading input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum Display {
    Screen,
    Window,
    Hidden,
}

impl From<Output> for Display {
    fn from(output: Output) -> Self {
        match output {
            Output::Screen => Display::Screen,
            Output::Window => Display::Window,
        }
    }
}

impl State {
    /// False on a first run, or after the user deleted the file.
    pub fn exists(dir: &Path) -> bool {
        dir.join(STATE_FILE).exists()
    }

    /// Like `load`; writes an empty state file if there is none yet.
    pub fn load_or_create(dir: &Path) -> Self {
        if !dir.join(STATE_FILE).exists()
            && let Err(e) = Self::default().save(dir)
        {
            tracing::warn!("{e:#}");
        }
        Self::load(dir)
    }

    pub fn load(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join(STATE_FILE))
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        let body = toml::to_string(self).context("serialize state")?;
        let text = format!("# Managed by Overlay Input; safe to delete.\n{body}");
        std::fs::write(dir.join(STATE_FILE), text).context("write state.toml")
    }
}

/// Folder holding `profiles/`, `settings.toml`, `state.toml` and the log.
pub fn app_dir(profiles_dir: &Path) -> PathBuf {
    profiles_dir
        .parent()
        .map_or_else(|| profiles_dir.to_path_buf(), Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_have_a_parseable_hotkey() {
        use std::str::FromStr;
        let s = Settings::default();
        assert!(
            global_hotkey::hotkey::HotKey::from_str(&s.toggle_hotkey).is_ok(),
            "{}",
            s.toggle_hotkey
        );
    }

    #[test]
    fn default_file_matches_default_struct() {
        let parsed: Settings = toml::from_str(DEFAULT_SETTINGS).unwrap();
        assert_eq!(parsed.toggle_hotkey, Settings::default().toggle_hotkey);
    }

    #[test]
    fn settings_save_keeps_comments_and_round_trips() {
        let dir =
            std::env::temp_dir().join(format!("overlay_input_settings_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (hotkey, at_start) in [("Shift+F9", true), ("", false)] {
            let settings = Settings {
                toggle_hotkey: hotkey.into(),
                show_window_at_start: at_start,
            };
            settings.save(&dir).unwrap();
            let text = std::fs::read_to_string(dir.join(SETTINGS_FILE)).unwrap();
            assert!(text.contains("# Global hotkey"), "comments kept");
            let loaded = Settings::load_or_create(&dir);
            assert_eq!(loaded.toggle_hotkey, hotkey);
            assert_eq!(loaded.show_window_at_start, at_start);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn old_settings_files_keep_defaults() {
        let parsed: Settings = toml::from_str("toggle_hotkey = \"F9\"\n").unwrap();
        assert_eq!(parsed.toggle_hotkey, "F9");
        assert!(!parsed.show_window_at_start);
    }

    #[test]
    fn state_round_trips() {
        let dir = std::env::temp_dir().join(format!("overlay_input_state_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        State {
            last_profile: Some("fps.toml".into()),
            output: Output::Window,
            window_size: Some([640, 360]),
            window_background: [1, 2, 3],
            ..State::default()
        }
        .save(&dir)
        .unwrap();
        let state = State::load(&dir);
        assert_eq!(state.last_profile.as_deref(), Some("fps.toml"));
        assert_eq!(state.output, Output::Window);
        assert_eq!(state.window_size, Some([640, 360]));
        assert_eq!(state.window_background, [1, 2, 3]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_a_first_run_has_no_state_file() {
        let dir = std::env::temp_dir().join(format!("overlay_input_first_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!State::exists(&dir), "first run");
        State::load_or_create(&dir);
        assert!(State::exists(&dir), "every later start");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn old_state_files_keep_defaults() {
        let old = "last_profile = \"a.toml\"\non_screen = false\nobs_window = true\n";
        let state: State = toml::from_str(old).unwrap();
        assert_eq!(state.last_profile.as_deref(), Some("a.toml"));
        assert_eq!(state.output, Output::Screen);
        assert_eq!(state.window_background, [255, 0, 255]);
    }
}
