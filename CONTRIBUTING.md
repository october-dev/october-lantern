# Contributing to October Lantern

Thanks for helping. Lantern has a macOS app (Swift, `macos/`) and a Rust app for Linux and Windows (`desktop/`, the Mac app's design in egui) with a shared Rust engine (`engine/`)
that finds your coding agents, reads their sessions and types replies for you.

## Build and check

- macOS: macOS 14+, Xcode 26, stable Rust. Linux: stable Rust, C compiler, Wayland/X11 and OpenGL runtimes; tmux for delivery. Go is only needed to build October Bus.
- `scripts/check.sh` runs everything CI runs: `cargo fmt --check`, strict Clippy, the engine tests,
  the current platform UI checks and a syntax check of the scripts. Please run it before a PR.
- `scripts/build-app.sh` builds `build/October Lantern.app` on macOS (ad-hoc signed without a Developer ID), or the Rust UI and engine in `build/linux/` on Linux. Use `--debug` for development.
- `python3 scripts/smoke-linux.py` exercises Linux discovery, replies and stale-target refusal in an isolated tmux server after building.
- `build/linux/october-lantern --fixture desktop/fixtures/<surface>.json --out <file.png> --scale 2` renders one surface from a saved model (no engine, network or agents), for visual checks against the Mac.
- `--new-session`, `--point-ask`, `--ask`, `--connections`, `--settings` and `--welcome` open each part of the running app. `--check-desktop` verifies the installed portal APIs and shortcut session lifecycle without binding keys.
- `scripts/check-windows-types.sh` type-checks the Windows code from Linux or macOS (clippy `-D warnings`, nothing linked).
- `scripts/build-bus-linux.sh` bundles the pinned native Bus; `scripts/package-linux.sh` produces Linux release assets. CI creates artifacts; its optional Publish input explicitly uploads a release. The updater source is selected by the build-time `LANTERN_RELEASES_REPO` variable.
- `scripts/setup-dictation.sh` installs local whisper.cpp and the multilingual model without root access and verifies transcription with the upstream sample.
- `OctoberLantern --snapshot <dir>` renders the panels offscreen, handy for UI changes.
- UI PRs get a `screens` artifact from the Screens workflow: the same surfaces on Mac and Linux side by side — check it before merging.

## How the pieces talk

The app and the engine speak JSON lines over stdio; `protocol/README.md` is the contract. Any
change to a message goes there in the same PR.

## Ground rules

- **Safety first.** Lantern types into terminals. Anything that sends keys or text goes through the
  engine's checks (`engine/src/deliver.rs`, `engine/src/actions.rs`); don't add paths around them.
- **Privacy.** Never send code, messages, prompts or file paths anywhere. Usage counts are listed
  in `docs/PRODUCT.md`; a new one needs a line there.
- **Don't edit users' config files** beyond the documented, backed-up hook install.
- Keep the UI calm and small: one pill, one panel, permissions asked only when a feature needs them.

## Pull requests

Small, focused PRs with a short description of what changed and how you checked it. For UI changes,
a screenshot helps. By contributing you agree your work is licensed under Apache-2.0 (see `LICENSE`).
