# Contributing to October Lantern

Thanks for helping. Lantern is a small macOS app (Swift, `macos/`) with a Rust engine (`engine/`)
that finds your coding agents, reads their sessions and types replies for you.

## Build and check

- macOS 14+, Xcode 26, Rust (stable), Go (only to build October Bus: `scripts/build-bus.sh`).
- `scripts/check.sh` runs everything CI runs: `cargo fmt --check`, strict Clippy, the engine tests,
  `swift build`, `swift test` and a syntax check of the scripts. Please run it before a PR.
- `scripts/build-app.sh` builds `build/October Lantern.app` (ad-hoc signed without a Developer ID).
- `OctoberLantern --snapshot <dir>` renders the panels offscreen, handy for UI changes.

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
