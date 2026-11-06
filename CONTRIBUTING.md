# Contributing

Thanks for helping out. Bug reports, ideas and pull requests are all welcome.

## Reporting a bug or asking for a feature

Use the [issue forms](https://github.com/m51v5/overlay-input/issues/new/choose). For a bug, the log lines and your profile usually make it quick to fix. Logs are in `logs/` next to the exe.

For a security problem, see [SECURITY.md](SECURITY.md) instead of opening an issue.

## Making a change

The app is Windows-only, so you'll need Windows 10/11 and Rust 1.95 or newer. The [development guide](docs/DEVELOPMENT.md) covers running from source, the tests and how the code is laid out.

1. Fork the repo and branch from `main`.
2. Make your change. For anything big, open an issue first so we can agree on the approach.
3. Run these, which must all pass:

   ```
   cargo fmt
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```

4. If you changed how the app looks or behaves, try it with `cargo run -- --window`, and update `docs/USAGE.md` if users need to know.
5. Open a pull request saying what changed and why.

## Guidelines

- **Keep the overlay light.** It runs alongside games, so avoid work per frame and per input event. Memory-heavy things belong in the app window process. `docs/DEVELOPMENT.md` explains the split.
- **No input hooks or injection.** The overlay only reads input passively, and it should stay that way.
- **Keep comments short:** one line that says why, not what.

By contributing, you agree that your work is released under the [MIT License](LICENSE). Everyone taking part is expected to follow the [code of conduct](https://github.com/m51v5/.github/blob/main/CODE_OF_CONDUCT.md).
