//! Profile files (TOML): anchor, opacity, scale and the widget layout tree.

pub mod layout;
pub mod watch;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Shipped profile, written to `profiles/` on first run and used if a profile fails to load.
pub const BUILTIN_DEFAULT: &str = include_str!("../../assets/profiles/default.toml");

/// Layout for profiles created in the app window (New).
pub const STARTER_PROFILE: &str = include_str!("../../assets/profiles/starter.toml");

/// Example profiles (from `assets/profiles/`) written to a fresh `profiles/` folder, so the exe alone is a full install.
const BUILTIN_PROFILES: [(&str, &str); 2] = [
    ("default.toml", BUILTIN_DEFAULT),
    (
        "gamepad.toml",
        include_str!("../../assets/profiles/gamepad.toml"),
    ),
];

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Display name in the tray; defaults to the file name.
    pub name: Option<String>,
    /// 0 = primary, then other monitors left to right.
    #[serde(default)]
    pub monitor: usize,
    #[serde(default)]
    pub anchor: Anchor,
    /// Logical px from the anchor corner, inward.
    #[serde(default = "default_offset")]
    pub offset: [f32; 2],
    /// Opacity of outlines and labels, from 0 (invisible) to 1 (solid); also of highlights unless Highlight opacity is set.
    #[serde(default = "default_opacity")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub opacity: f32,
    /// Opacity of input highlights: pressed fills and glows, scroll and stick tints, the movement ring, Flick!. Empty = same as Opacity.
    #[serde(default)]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub highlight_opacity: Option<f32>,
    /// Extra size multiplier on top of Windows display scaling.
    #[serde(default = "default_scale")]
    #[schemars(range(min = 0.1, max = 5.0))]
    pub scale: f32,
    /// Logical px between widgets.
    #[serde(default = "default_gap")]
    pub gap: f32,
    /// Edited by the layout builder, not as a form field.
    #[schemars(skip)]
    pub layout: Node,
}

fn default_offset() -> [f32; 2] {
    [20.0, 20.0]
}
fn default_opacity() -> f32 {
    0.6
}
fn default_scale() -> f32 {
    1.0
}
fn default_gap() -> f32 {
    8.0
}

#[derive(
    Debug, Deserialize, serde::Serialize, Clone, Copy, Default, PartialEq, Eq, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    TopLeft,
    TopRight,
    #[default]
    BottomLeft,
    BottomRight,
    Center,
}

/// Cross-axis alignment of a row's or column's children.
#[derive(
    Debug, Deserialize, serde::Serialize, Clone, Copy, Default, PartialEq, Eq, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Node {
    Row {
        row: Vec<Node>,
        #[serde(default)]
        align: Align,
    },
    Column {
        column: Vec<Node>,
        #[serde(default)]
        align: Align,
    },
    Widget {
        widget: String,
        /// Widget-specific options (e.g. `preset`), validated by the widget.
        #[serde(flatten)]
        options: toml::Table,
    },
}

impl Profile {
    pub fn parse(text: &str) -> Result<Self> {
        let profile: Self = toml::from_str(text)?;
        anyhow::ensure!(
            (0.0..=1.0).contains(&profile.opacity),
            "opacity must be 0..=1, got {}",
            profile.opacity
        );
        if let Some(h) = profile.highlight_opacity {
            anyhow::ensure!(
                (0.0..=1.0).contains(&h),
                "highlight_opacity must be 0..=1, got {h}"
            );
        }
        anyhow::ensure!(
            profile.scale > 0.0,
            "scale must be > 0, got {}",
            profile.scale
        );
        Ok(profile)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("read {path:?}"))?;
        Self::parse(&text).with_context(|| format!("profile {path:?}"))
    }
}

impl Anchor {
    /// Top-left of a `size` box placed in `area` (left, top, right, bottom), all physical px.
    pub fn position(
        self,
        area: (i32, i32, i32, i32),
        size: (i32, i32),
        offset: (i32, i32),
    ) -> (i32, i32) {
        let (l, t, r, b) = area;
        let (w, h) = size;
        let (ox, oy) = offset;
        match self {
            Anchor::TopLeft => (l + ox, t + oy),
            Anchor::TopRight => (r - ox - w, t + oy),
            Anchor::BottomLeft => (l + ox, b - oy - h),
            Anchor::BottomRight => (r - ox - w, b - oy - h),
            Anchor::Center => (l + (r - l - w) / 2 + ox, t + (b - t - h) / 2 + oy),
        }
    }
}

/// `profiles/` next to the exe; dev builds use the working dir's (`cargo run` from the repo). Not created here: see `seed_profiles`.
pub fn profiles_dir() -> Result<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("profiles")));
    let dir = if cfg!(debug_assertions) {
        let work_dir = std::env::current_dir().ok().map(|d| d.join("profiles"));
        [exe_dir, work_dir.clone()]
            .into_iter()
            .flatten()
            .find(|d| d.is_dir())
            .or(work_dir)
    } else {
        exe_dir
    };
    dir.context("no location for the profiles folder")
}

/// A new folder gets every example profile; an existing one only gets `default.toml` back if it's gone.
/// Nothing is overwritten: names are compared ignoring case, like Windows does.
pub fn seed_profiles(dir: &Path) -> Result<()> {
    let fresh = !dir.is_dir();
    std::fs::create_dir_all(dir).with_context(|| format!("create {dir:?}"))?;
    let seeds = if fresh {
        &BUILTIN_PROFILES[..]
    } else {
        &BUILTIN_PROFILES[..1] // default.toml
    };
    for (name, text) in seeds {
        if find_file(dir, name).is_none() {
            std::fs::write(dir.join(name), text).with_context(|| format!("write {name}"))?;
            tracing::info!(?dir, file = name, "seeded profile");
        }
    }
    Ok(())
}

/// The file called `name` in `dir`, ignoring letter case.
pub fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path())
}

/// Renames a broken `default.toml` to a free `default.toml.bak` name and writes the built-in default in its place.
/// Returns the backup's path.
pub fn replace_with_builtin(default: &Path) -> Result<PathBuf> {
    let dir = default.parent().context("default.toml has no folder")?;
    let backup = (1..)
        .map(|n| match n {
            1 => "default.toml.bak".to_owned(),
            n => format!("default.toml.bak-{n}"),
        })
        .map(|name| dir.join(name))
        .find(|path| {
            find_file(dir, &path.file_name().unwrap_or_default().to_string_lossy()).is_none()
        })
        .expect("a free name");
    std::fs::rename(default, &backup)
        .with_context(|| format!("back up {default:?} as {backup:?}"))?;
    std::fs::write(default, BUILTIN_DEFAULT).with_context(|| format!("write {default:?}"))?;
    Ok(backup)
}

/// Name shown for a profile: the file name, plus its `name` when that differs, so two files with the same `name` stay apart.
pub fn label(profile: &Profile, path: &Path) -> String {
    let file = display_name(path);
    match &profile.name {
        Some(name) if !name.eq_ignore_ascii_case(&file) => format!("{name} ({file})"),
        _ => file,
    }
}

/// `*.toml` files in `dir`, sorted by file name.
pub fn list_profiles(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| is_profile_file(p))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

pub fn is_profile_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
}

/// Tray label: file stem (profile `name` would need a parse per listing).
pub fn display_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_default_parses() {
        let p = Profile::parse(BUILTIN_DEFAULT).unwrap();
        assert_eq!(p.anchor, Anchor::BottomLeft);
    }

    #[test]
    fn every_builtin_profile_parses() {
        for (name, text) in BUILTIN_PROFILES
            .into_iter()
            .chain([("starter", STARTER_PROFILE)])
        {
            Profile::parse(text).unwrap_or_else(|e| panic!("{name}: {e:#}"));
        }
    }

    #[test]
    fn nested_layout_and_defaults() {
        let p = Profile::parse(
            r#"
            anchor = "top-right"
            [layout]
            column = [
              { widget = "keyboard", preset = "default" },
              { row = [ { widget = "keyboard" }, { widget = "keyboard" } ], align = "end" },
            ]
            "#,
        )
        .unwrap();
        assert_eq!(p.anchor, Anchor::TopRight);
        assert_eq!(p.offset, [20.0, 20.0]);
        let Node::Column { column, .. } = &p.layout else {
            panic!("column expected")
        };
        assert!(
            matches!(&column[0], Node::Widget { widget, options } if widget == "keyboard" && options.contains_key("preset"))
        );
        assert!(matches!(&column[1], Node::Row { row, align: Align::End } if row.len() == 2));
    }

    #[test]
    fn rejects_bad_values() {
        assert!(Profile::parse("opacity = 2.0\n[layout]\nwidget = \"keyboard\"").is_err());
        assert!(Profile::parse("typo_key = 1\n[layout]\nwidget = \"keyboard\"").is_err());
        assert!(
            Profile::parse("highlight_opacity = 1.5\n[layout]\nwidget = \"keyboard\"").is_err()
        );
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("overlay_input_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn seeding_creates_what_is_missing_and_overwrites_nothing() {
        let dir = temp_dir("seed");
        seed_profiles(&dir).unwrap();
        assert_eq!(names(&dir), ["default.toml", "gamepad.toml"]);

        // Existing folder: deleted examples stay deleted, default.toml comes back.
        for f in ["gamepad.toml", "default.toml"] {
            std::fs::remove_file(dir.join(f)).unwrap();
        }
        seed_profiles(&dir).unwrap();
        assert_eq!(names(&dir), ["default.toml"]);

        // A differently cased Default.toml counts as the default and is kept as written.
        std::fs::remove_file(dir.join("default.toml")).unwrap();
        std::fs::write(dir.join("Default.toml"), "mine").unwrap();
        seed_profiles(&dir).unwrap();
        assert_eq!(names(&dir), ["Default.toml"]);
        assert_eq!(
            std::fs::read_to_string(dir.join("Default.toml")).unwrap(),
            "mine"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn broken_default_is_backed_up_then_replaced() {
        let dir = temp_dir("broken_default");
        std::fs::create_dir_all(&dir).unwrap();
        let default = dir.join("default.toml");
        std::fs::write(dir.join("default.toml.bak"), "older backup").unwrap();
        std::fs::write(&default, "not = [valid").unwrap();

        let backup = replace_with_builtin(&default).unwrap();
        assert_eq!(
            backup.file_name().unwrap(),
            "default.toml.bak-2",
            "free name"
        );
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "not = [valid");
        assert_eq!(
            std::fs::read_to_string(dir.join("default.toml.bak")).unwrap(),
            "older backup"
        );
        assert_eq!(std::fs::read_to_string(&default).unwrap(), BUILTIN_DEFAULT);
        assert!(
            !is_profile_file(&backup),
            "backups don't show up as profiles"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn labels_tell_same_names_apart() {
        let named = |name: Option<&str>| Profile {
            name: name.map(str::to_owned),
            ..Profile::parse(BUILTIN_DEFAULT).unwrap()
        };
        assert_eq!(label(&named(None), Path::new("fps.toml")), "fps");
        assert_eq!(label(&named(Some("FPS")), Path::new("fps.toml")), "fps");
        assert_eq!(
            label(&named(Some("FPS")), Path::new("fps copy.toml")),
            "FPS (fps copy)"
        );
    }

    #[test]
    fn anchor_positions() {
        let area = (0, 0, 1000, 800);
        let size = (100, 50);
        let off = (10, 20);
        assert_eq!(Anchor::TopLeft.position(area, size, off), (10, 20));
        assert_eq!(Anchor::TopRight.position(area, size, off), (890, 20));
        assert_eq!(Anchor::BottomLeft.position(area, size, off), (10, 730));
        assert_eq!(Anchor::BottomRight.position(area, size, off), (890, 730));
        assert_eq!(Anchor::Center.position(area, size, (0, 0)), (450, 375));
    }
}
