# Development

How to run, test and change Overlay Input, and how it's built.

---

## Run from source

You need Windows 10/11 and Rust 1.95 or newer (`rustup update stable`), with the Visual Studio C++ build tools that rustup asks for. Run everything from the repo root:

```
cargo run                 # build and start the overlay; the first build is the slow one
cargo run -- --window     # same, and open the app window
```

- The overlay appears on screen and in the tray, and the terminal stays open for logs. Stop it with tray → Quit or Ctrl+C.
- The first run creates `profiles/`, `settings.toml` and `state.toml` in the repo root, all git-ignored. Saving a profile applies it at once, with no rebuild. If `target\debug\profiles` exists, that folder is used instead.
- Shipped example profiles live in `assets/profiles/` and are embedded in the exe. The git-ignored `profiles/` folder is your working copy, so editing profiles while developing never changes the shipped examples.
- Only warnings and errors are logged by default. For more, set `RUST_LOG` before `cargo run`; `Remove-Item Env:RUST_LOG` turns it off again:

  ```
  $env:RUST_LOG="overlay_input=info"    # startup steps
  $env:RUST_LOG="overlay_input=debug"   # also every key and button
  $env:RUST_LOG="overlay_input=trace"   # also mouse movement
  ```

- Quit any running copy first, including a release build:
  - While one runs, a new start only opens the running one's window and exits.
  - If the running copy is the dev build, `cargo build` also fails with `Access is denied` on `overlay_input.exe`. `Stop-Process -Name overlay_input` stops it from a terminal.

## Build a release

```
.\scripts\build-release.ps1
```

This stops a running copy, builds the release exe, and copies it to `dist\OverlayInput\` (`-Out <folder>` to put it elsewhere). If PowerShell blocks scripts, run `powershell -ExecutionPolicy Bypass -File .\scripts\build-release.ps1`.

## Publish a release

Pushing a commit to `main` only runs CI. Pushing a tag publishes a release. A tag adds a name to a commit and changes no code, so CI doesn't run again for it, and the release workflow doesn't run the tests. Wait for CI before tagging.

Replace `0.2.0` with the new version.

1. Start from an up-to-date `main`, with nothing uncommitted:
   ```
   git checkout main
   git pull
   git status
   ```
2. Set `version = "0.2.0"` in `Cargo.toml`, then update `Cargo.lock`:
   ```
   cargo build
   ```
3. Commit and push:
   ```
   git add Cargo.toml Cargo.lock
   git commit -m "chore: release v0.2.0"
   git push
   ```
4. Wait for CI to pass, in the Actions tab or with:
   ```
   gh run list --workflow ci.yml --limit 1
   ```
5. Tag that same commit and push the tag. Don't commit anything in between:
   ```
   git tag v0.2.0
   git push origin v0.2.0
   ```
6. Wait for the Release workflow. It builds the exe, zips it with `LICENSE` and `THIRD_PARTY_LICENSES.txt` as `overlay-input-v0.2.0-windows-x64.zip`, and publishes a GitHub Release.
7. Write the notes. Generated notes only list merged pull requests, so they're nearly empty for commits pushed straight to `main`. Click **Edit** on the release, or run:
   ```
   gh release edit v0.2.0 --notes-file notes.md
   ```
8. Download the zip, unzip it into an empty folder, and run it.

**If the Release workflow fails,** nothing is published. It stops when:

- the tagged commit isn't on `main`
- the tag and the version in `Cargo.toml` differ
- a dependency's license isn't listed in `about.toml`

Remove the tag, push your fix to `main`, wait for CI, then repeat step 5:

```
git tag -d v0.2.0
git push origin --delete v0.2.0
```

**Third-party licenses:** `cargo-about` generates that file from `scripts/licenses/`. A new dependency under a license that isn't listed in `about.toml` stops the release until you've reviewed it and added it. To check before tagging:

```
cargo install cargo-about --locked --features cli
cargo about generate --config scripts/licenses/about.toml --locked -o THIRD_PARTY_LICENSES.txt scripts/licenses/about.hbs
```

## Tests and measurements

```
cargo test                                  # unit tests
cargo test snapshot -- --ignored            # render widgets to target/snapshots/*.png
cargo test gamepad_readers -- --ignored --nocapture   # 3 s smoke test with real pads
cargo test round_trip_in_the_registry -- --ignored    # writes and removes the real Windows startup entry
cargo test --release widget_cost -- --ignored --nocapture   # draw time per widget
cargo test --release frame_cost -- --ignored --nocapture    # full frames of the shipped profiles
```

Dev builds are optimized (`[profile.dev]` in `Cargo.toml`): unoptimized drawing was 20 to 50 times slower and showed up as input lag. When a frame misses the monitor's refresh, the log says how many did and whether drawing or presenting was slow.

---

## Stack

Rust (edition 2024), Windows 10/11 only. No WebView, no async runtime.

| Area | Crate | Why |
|---|---|---|
| Event loop | `winit` 0.30 | Message pump, raw input, events from other threads |
| Overlay window | `windows` (plain Win32 popup) | winit rewrites the whole extended style on any flag change, which drops `WS_EX_TOOLWINDOW` and `WS_EX_NOACTIVATE` |
| Presenting | `UpdateLayeredWindow` | The reliable way to get per-pixel alpha with click-through on Windows |
| Keyboard and mouse | `winit` device events | Raw Input with `RIDEV_INPUTSINK`: works while a game has focus. Passive, no hooks, no injection |
| Xbox pads | `gilrs` (`xinput` feature) | Its default backend needs a focused window; XInput doesn't |
| DualSense | `hidapi` + a small report parser | Reads the pad directly, no focus needed |
| 2D drawing | `tiny-skia` | CPU renderer, premultiplied RGBA |
| SVG | `usvg` | Parsed and drawn element by element, so buttons can light up one at a time |
| Text | `ab_glyph` | System fonts, glyph outlines drawn by tiny-skia |
| Profiles | `serde` + `toml`, `notify` | Readable files, live reload |
| Forms | `schemars` | JSON Schema from the option structs, so the forms follow the code |
| App window | `egui`, `eframe` (`glow`) | Native window, OpenGL, in its own process |
| Tray, hotkey | `tray-icon`, `global-hotkey` | |
| Logs | `tracing`, `tracing-subscriber` | |
| Icon, manifest | `embed-resource` | Per-Monitor-V2 DPI awareness |

---

## Architecture

```
gamepad threads (gilrs, hidapi) ──► InputEvent ─┐  EventLoopProxy
                                                ▼
main thread (winit loop, App)
  raw input ─► InputEvent ─► InputState
  Profile ─► Scene (layout + widgets: on_input, tick, draw)
  Scene ─► tiny-skia Pixmap ─► overlay window  or  overlay color window
  tray, hotkey, profile file watcher
       ▲
       │  JSON lines over the child's stdin/stdout
       ▼
app window process (overlay_input.exe --ui, eframe)
  AppUi ─► Command ─► run here (profile files, validation, offscreen render)
                     └─► or ask the overlay (preview, show profile, hotkey)
```

- **Redraws:** only when input arrives or an animation runs, at most once per monitor refresh. Otherwise the loop sleeps.
- **Static layers:** outlines and labels are drawn once per scale into a cached layer, then copied as runs of non-empty pixels. Only lit parts are redrawn each frame. Drawing everything every frame was the main cause of input lag.
- **Split opacity:** with `highlight_opacity`, pressed fills and glows go to a second layer, and the two blend in one pass when presenting.
- **Display:** on screen, in a window or hidden. Only Hide stops reading input: raw input is unregistered and gamepad readers pause.
- **App window:** its own process, so the graphics driver and egui never load into the overlay, and closing the window frees all of it. The overlay starts it and owns the pipes; a closed pipe means the other side is gone, crash included. If the window goes, its preview ends; if the overlay goes, the window exits. The UI emits `Command`s, and replies from the overlay come back in order. The working copy is a JSON tree, so an unset option stays unset and is left out of the saved file.
- **App window pages:** Home, Profiles, Settings and About share one `AppUi`, so unsaved edits survive switching pages. The overlay writes `settings.toml` and `state.toml`, so the window asks it to change them instead of writing them itself; "Start with Windows" is the exception, as it lives in the registry.
- **Validation:** the app window and the overlay run the same `Profile::parse` and `Scene::build`, so the window can't save a profile the overlay would reject.

---

## Source layout

```
src/
  main.rs            entry: overlay or window process, single instance, logging, profile seeding, event loop
  app.rs             owns windows, tray and scene; display modes, frame pacing, window messages
  scene.rs           loaded profile: widgets + layout + overlay settings
  tray.rs            tray icon and menu
  settings.rs        settings.toml (hotkey, start option), state.toml (what the app remembers)
  autostart.rs       "Start with Windows": a per-user Run entry
  instance.rs        named mutex: one overlay per session
  dialog.rs          native dialogs
  logfile.rs         daily log files, local time + UTC offset
  input/             InputEvent, raw input mapping, gamepad readers
  overlay/           layered window, presenting, color window for OBS, topmost + tray menu fixes
  profile/           TOML structs, validation, anchors, layout, seeding, watcher
  render/            text, glow sprites, easing, gradients, SVG, layer blending
  widgets/           keyboard, mouse, mouse-movement, mouse-speed, mouse-flick, gamepad
  ui/                window process, pipe messages, schema → forms, layout tree, offscreen render
  ui/pages/          AppUi and sidebar (mod), theme, home, profiles + its parts, settings, about, tests
assets/windows/      icon, manifest, resource script
assets/widgets/      SVGs
docs/images/         screenshots used by the README and the usage guide
assets/profiles/     shipped profiles, embedded and seeded on first run; starter.toml is what New starts from
scripts/             build-release.ps1; licenses/ (third-party license file for release zips)
```

---

## Things that will bite you

- **Static C runtime:** `.cargo/config.toml` sets `crt-static`, so the exe starts on PCs without the Visual C++ redistributable. Check with `objdump -p` or `dumpbin /imports`: no `VCRUNTIME140.dll`.
- **Stdout is the app window's pipe:** nothing in the `--ui` process may print to it. Logs go to stderr or the log file.
- **`eframe` doesn't run `ui` while minimized:** handle overlay messages in `App::logic`, which still runs.
- **`eframe` shows its window but doesn't raise it:** the first frame asks for focus.
- **egui widgets in `dnd_drag_source` scopes don't wrap:** use `Response::dnd_set_drag_payload`.
- **`ui.columns` is justified:** wrapped text in it gets stretched letter spacing; set a plain top-down layout inside.
- **egui `Grid` cells don't limit width:** wide controls (key picker, gradient bar) get full-width rows.
- **Page width:** the Profiles page has three columns. The screen preview hides when the form would get under `FORM_MIN_WIDTH`; check a narrow window after changing a page.
- **Sliders snap f32 noise:** tidy values before showing them, or opening a profile marks it as changed. A headless egui test covers this.
- **The OpenGL driver stays loaded after its window closes,** until the process exits. That's why the app window is a separate process. `OPENGL32.dll` itself shows in the overlay's modules because the exe links it; the costly vendor driver doesn't load until a context is created.
- **Tray menu:** Windows can open it under the taskbar when focus isn't granted. `foreground.rs` lifts our popup menus to topmost as they appear.
