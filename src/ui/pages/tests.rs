//! Headless egui frames: pages draw, clicks ask for the right commands, opening changes nothing.

use super::*;
use crate::ui::process::system_fonts;

fn host(profiles: &[&str]) -> HostView {
    HostView {
        version: "test",
        profiles: profiles.iter().map(|p| (*p).to_owned()).collect(),
        active: profiles.first().map(|p| (*p).to_owned()),
        previewing: false,
        monitors: crate::overlay::monitor_infos(),
        hotkey: "Ctrl+Alt+F12".into(),
        hotkey_registered: true,
        default_hotkey: "Ctrl+Alt+F12",
        display: crate::settings::Display::Screen,
        hide_from_capture: false,
        window_background: [255, 0, 255],
        show_window_at_start: false,
        start_with_windows: false,
    }
}

/// Runs frames without a window; answers `Open` with `file`, everything else with `Done`.
fn frames(ui_state: &mut AppUi, host: &HostView, text: &str, count: usize) -> Vec<Command> {
    let ctx = context();
    let mut seen = Vec::new();
    for _ in 0..count {
        seen.extend(frame(&ctx, ui_state, host, text, Vec::new()));
    }
    seen
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.set_fonts(system_fonts().unwrap());
    style(&ctx);
    ctx
}

fn frame(
    ctx: &egui::Context,
    ui_state: &mut AppUi,
    host: &HostView,
    text: &str,
    events: Vec<egui::Event>,
) -> Vec<Command> {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(1600.0, 1000.0),
        )),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| ui_state.show(ui, host));
    output.textures_delta.clear();
    let mut seen = Vec::new();
    for command in ui_state.take_commands() {
        let outcome = match &command {
            Command::Open(_) => {
                let value: toml::Value = toml::from_str(text).unwrap();
                Ok(Reply::Profile(serde_json::to_value(value).unwrap()))
            }
            _ => Ok(Reply::Done),
        };
        seen.push(command.clone());
        ui_state.on_result(command, outcome, host.active.as_deref());
    }
    seen
}

/// A real egui click: hover, press and release in their own frames.
fn click(
    ctx: &egui::Context,
    ui_state: &mut AppUi,
    host: &HostView,
    text: &str,
    pos: egui::Pos2,
) -> Vec<Command> {
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let mut seen = Vec::new();
    for events in [
        vec![egui::Event::PointerMoved(pos)],
        vec![button(true)],
        vec![button(false)],
        Vec::new(),
    ] {
        seen.extend(frame(ctx, ui_state, host, text, events));
    }
    seen
}

/// Clicks down a column of the page; what the page asked for, wherever its controls are.
fn commands_down_column(page: Page, x: f32) -> Vec<Command> {
    let text = std::fs::read_to_string("assets/profiles/default.toml").unwrap();
    let host = host(&["default"]);
    let mut seen = Vec::new();
    for y in (0..700).step_by(8) {
        let ctx = context();
        let mut state = AppUi::new();
        state.page = page;
        frame(&ctx, &mut state, &host, &text, Vec::new());
        seen.extend(click(
            &ctx,
            &mut state,
            &host,
            &text,
            egui::pos2(x, y as f32),
        ));
    }
    seen
}

#[test]
fn opening_a_profile_changes_nothing() {
    // No opacity: the slider shows the f32 default (0.6000000238), which must not count as an edit.
    let bare = "scale = 0.6\n[layout]\nwidget = \"keyboard\"\n".to_owned();
    let files = [
        (
            "default",
            std::fs::read_to_string("assets/profiles/default.toml").unwrap(),
        ),
        (
            "gamepad",
            std::fs::read_to_string("assets/profiles/gamepad.toml").unwrap(),
        ),
        ("bare", bare),
    ];
    for (name, text) in files {
        let mut state = AppUi::new();
        state.page = Page::Profiles;
        let host = host(&[name]);
        let seen = frames(&mut state, &host, &text, 4);
        assert!(state.profile.is_some(), "{name} loaded");
        assert!(
            !state.dirty(),
            "{name}: opening marked it changed: {}",
            serde_json::to_string(state.profile.as_ref().unwrap()).unwrap()
        );
        assert!(
            !seen.iter().any(|c| matches!(c, Command::Preview(_))),
            "{name}: no preview without edits"
        );
    }
}

#[test]
fn every_page_draws() {
    let text = std::fs::read_to_string("assets/profiles/default.toml").unwrap();
    for page in [Page::Home, Page::Profiles, Page::Settings, Page::About] {
        let mut state = AppUi::new();
        state.page = page;
        frames(&mut state, &host(&["default"]), &text, 2);
        assert!(state.profile.is_some());
    }
}

#[test]
fn sidebar_switches_pages() {
    let text = std::fs::read_to_string("assets/profiles/default.toml").unwrap();
    let host = host(&["default"]);
    let ctx = context();
    let mut state = AppUi::new();
    frame(&ctx, &mut state, &host, &text, Vec::new());
    let mut pages = Vec::new();
    for y in (40..260).step_by(4) {
        click(&ctx, &mut state, &host, &text, egui::pos2(60.0, y as f32));
        if pages.last() != Some(&state.page) {
            pages.push(state.page);
        }
    }
    for page in [Page::Home, Page::Profiles, Page::Settings, Page::About] {
        assert!(pages.contains(&page));
    }
}

#[test]
fn home_tiles_ask_for_each_display() {
    use crate::settings::Display;
    // Screen is on already, so its tile asks for nothing; the others sit further right.
    let window = commands_down_column(Page::Home, 570.0);
    assert!(
        window
            .iter()
            .any(|c| matches!(c, Command::SetDisplay(Display::Window))),
        "{window:?}"
    );
    let hidden = commands_down_column(Page::Home, 800.0);
    assert!(
        hidden
            .iter()
            .any(|c| matches!(c, Command::SetDisplay(Display::Hidden))),
        "{hidden:?}"
    );
    assert!(
        !window
            .iter()
            .chain(&hidden)
            .any(|c| matches!(c, Command::SetDisplay(Display::Screen)))
    );
}

#[test]
fn settings_checkboxes_ask_for_their_setting() {
    let seen = commands_down_column(Page::Settings, 225.0);
    for (name, found) in [
        (
            "hide from recordings",
            seen.iter()
                .any(|c| matches!(c, Command::SetHideFromCapture(true))),
        ),
        (
            "show window at start",
            seen.iter()
                .any(|c| matches!(c, Command::SetShowWindowAtStart(true))),
        ),
        (
            "start with windows",
            seen.iter()
                .any(|c| matches!(c, Command::SetStartWithWindows(true))),
        ),
    ] {
        assert!(found, "{name}: {seen:?}");
    }
}

#[test]
fn closing_a_dirty_profile_ends_its_preview_without_waiting_for_status() {
    let text = std::fs::read_to_string("assets/profiles/default.toml").unwrap();
    // The overlay hasn't reported the preview yet: `previewing` is still false.
    let host = host(&["default"]);
    let mut state = AppUi::new();
    frames(&mut state, &host, &text, 2);
    state.previewed = Some(serde_json::json!({ "gap": 2 }));
    let seen = frames(&mut state, &host, &text, 1);
    assert!(seen.iter().any(|c| matches!(c, Command::EndPreview)));
}

#[test]
fn about_can_open_the_logs_folder() {
    let seen = commands_down_column(Page::About, 400.0);
    assert!(
        seen.iter().any(|c| matches!(c, Command::OpenLogsFolder)),
        "{seen:?}"
    );
}

#[test]
fn quit_closes_at_once_without_changes() {
    let mut state = AppUi::new();
    state.request_quit();
    assert!(state.quitting());
    assert!(state.take_close());
}

#[test]
fn quit_with_changes_asks_and_can_be_cancelled() {
    let mut state = AppUi::new();
    state.saved = serde_json::json!({ "gap": 1 });
    state.profile = Some(serde_json::json!({ "gap": 2 }));
    state.request_quit();
    assert!(!state.take_close(), "asks first");
    assert!(matches!(state.confirm, Some(Confirm::Close)));
    state.on_result(
        Command::Save {
            name: "x".into(),
            profile: serde_json::json!({}),
        },
        Err("disk full".into()),
        None,
    );
    assert!(
        state.quitting(),
        "only a save started by the prompt cancels"
    );
}
