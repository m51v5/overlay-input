# Security policy

## Reporting a vulnerability

Please don't open a public issue. Use [private vulnerability reporting](https://github.com/m51v5/overlay-input/security/advisories/new) instead, with the steps to reproduce it and the version you're on.

Fixes go into the latest release only.

## What the app does with your input

Overlay Input reads your keyboard, mouse and gamepad so it can draw them, so here is what happens to that input:

- **It's passive.** It uses Windows Raw Input, XInput and HID. There are no keyboard or mouse hooks, and nothing is injected into other apps.
- **It stays on your PC.** The app has no network code. The only link out is the one on the About page, which opens your browser.
- **It isn't saved.** Keys are drawn and forgotten. The one exception is logging you turn on yourself: with `RUST_LOG=overlay_input=debug` set, every key and button is written to the log, or to the console in a dev build. It's off by default, so leave it off while typing anything private.
- **Hiding stops it.** While the overlay is hidden, input isn't read at all.
