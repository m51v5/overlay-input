//! What the window's UI sees of the overlay (`HostView`) and asks it to do (`Command`), plus profile file rules.

use serde_json::Value;
use tiny_skia::Pixmap;

use crate::overlay::MonitorInfo;
use crate::settings::Display;

/// Overlay state the UI shows.
pub struct HostView {
    pub version: &'static str,
    /// Profile names (file stems), sorted like the tray.
    pub profiles: Vec<String>,
    pub active: Option<String>,
    /// An unsaved profile is on the overlay.
    pub previewing: bool,
    pub monitors: Vec<MonitorInfo>,
    /// `toggle_hotkey` as set, even when it couldn't be registered.
    pub hotkey: String,
    pub hotkey_registered: bool,
    pub default_hotkey: &'static str,
    pub display: Display,
    pub hide_from_capture: bool,
    pub window_background: [u8; 3],
    pub show_window_at_start: bool,
    /// A Windows startup entry runs this exe.
    pub start_with_windows: bool,
}

#[derive(Debug, Clone)]
pub enum Command {
    /// Read a profile file as an editable tree.
    Open(String),
    /// Validate and write a profile (TOML).
    Save {
        name: String,
        profile: Value,
    },
    /// Show this profile on the overlay.
    Activate(String),
    /// Show an unsaved profile on the overlay until `EndPreview`.
    Preview(Value),
    /// Back to the active profile.
    EndPreview,
    /// Draw an unsaved profile, optionally with sample input.
    Render {
        profile: Value,
        sample: bool,
    },
    /// New profile from the starter layout.
    Create(String),
    Duplicate {
        from: String,
        to: String,
    },
    Rename {
        from: String,
        to: String,
    },
    Delete(String),
    /// Register and save the show/hide hotkey; empty = none.
    SetHotkey(String),
    SetDisplay(Display),
    SetWindowBackground([u8; 3]),
    SetHideFromCapture(bool),
    SetShowWindowAtStart(bool),
    SetStartWithWindows(bool),
    OpenProfilesFolder,
    OpenLogsFolder,
}

/// What a command gives back.
pub enum Reply {
    Done,
    Profile(Value),
    Image(Pixmap),
}

pub type Outcome = Result<Reply, String>;

/// Profile tree → TOML text. Nulls are dropped (TOML has none; they mean "use the default").
pub fn json_to_toml(name: &str, profile: &Value) -> Result<String, String> {
    let value = toml::Value::try_from(strip_nulls(profile.clone()))
        .map_err(|e| format!("can't convert to TOML: {e}"))?;
    if !value.is_table() {
        return Err("profile must be a table".into());
    }
    let body = toml::to_string(&value).map_err(|e| e.to_string())?;
    Ok(format!(
        "# {name} — saved by Overlay Input.
# Option help lives in the app window; hand-written comments are not kept.

{body}"
    ))
}

fn strip_nulls(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, strip_nulls(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .filter(|v| !v.is_null())
                .map(strip_nulls)
                .collect(),
        ),
        other => other,
    }
}

/// Points a top-level `name = ...` line (if any) at `name`, keeping the rest of the file and its comments.
pub fn with_display_name(text: &str, name: &str) -> String {
    let mut in_root = true;
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            in_root = false;
        }
        let is_name = in_root
            && trimmed
                .strip_prefix("name")
                .is_some_and(|rest| rest.trim_start().starts_with('='));
        out.push(if is_name {
            format!("name = \"{name}\"")
        } else {
            line.to_owned()
        });
    }
    let mut joined = out.join("\n");
    if text.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

/// Why `name` can't be a profile file name, if it can't: letters, digits, space, `_` and `-` only.
pub fn name_problem(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("Type a name.")
    } else if name.len() > 64 {
        Some("At most 64 characters.")
    } else if name.trim() != name {
        Some("No spaces at the start or end.")
    } else if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-'))
    {
        Some("Use letters, digits, spaces, _ and - only.")
    } else if is_reserved_device_name(name) {
        Some("Windows reserves that name.")
    } else {
        None
    }
}

/// Windows device names (`CON`, `NUL`, `COM1`, ...) can't be file names, even with an extension.
fn is_reserved_device_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let numbered = upper.len() == 4
        && (upper.starts_with("COM") || upper.starts_with("LPT"))
        && matches!(upper.as_bytes()[3], b'1'..=b'9');
    numbered || matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names() {
        for ok in ["fps", "My Profile", "game_1-b"] {
            assert_eq!(name_problem(ok), None, "{ok}");
        }
        for bad in [
            "", "../x", "a/b", "a\\b", "x.toml", " lead", "trail ", "con:", "CON", "nul", "Com3",
            "lpt9",
        ] {
            assert!(name_problem(bad).is_some(), "{bad}");
        }
    }

    #[test]
    fn tree_round_trips_to_a_valid_profile() {
        let json = serde_json::json!({
            "name": "T", "anchor": "top-right", "opacity": 0.5, "offset": [10, 20], "monitor": null,
            "layout": { "column": [
                { "widget": "keyboard", "preset": "full" },
                { "row": [ { "widget": "mouse", "scroll_color": [1, 2, 3] } ], "align": "end" }
            ]}
        });
        let text = json_to_toml("T", &json).unwrap();
        assert!(text.starts_with("# T — saved by"));
        assert!(!text.contains("monitor"), "null dropped");
        let profile = crate::profile::Profile::parse(&text).unwrap();
        assert_eq!(profile.anchor, crate::profile::Anchor::TopRight);
        assert_eq!(profile.offset, [10.0, 20.0]);
    }

    #[test]
    fn display_name_is_rewritten_only_at_top_level() {
        let text = "# FPS layout\nname = \"FPS\"\ngap = 8\n\n[layout]\nname = \"keep\"\n";
        let out = with_display_name(text, "FPS copy");
        assert_eq!(
            out,
            "# FPS layout\nname = \"FPS copy\"\ngap = 8\n\n[layout]\nname = \"keep\"\n"
        );
        assert_eq!(with_display_name("gap = 8", "x"), "gap = 8", "no name line");
        assert!(with_display_name("names = 1", "x").contains("names = 1"));
    }

    #[test]
    fn non_table_is_rejected() {
        assert!(json_to_toml("x", &serde_json::json!([1, 2])).is_err());
    }
}
