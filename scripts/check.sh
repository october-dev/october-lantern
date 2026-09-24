#!/usr/bin/env bash
# The gate every change must pass: engine format, lints and tests; the app's build and tests; and
# shell syntax. release.sh runs it before building, and CI runs it on every push.
#   scripts/check.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Rust from Homebrew's rustup or a plain rustup install, when cargo isn't already on PATH.
if ! command -v cargo >/dev/null; then
  for dir in /opt/homebrew/opt/rustup/bin "$HOME/.cargo/bin" "$HOME"/.rustup/toolchains/stable-*/bin; do
    if [[ -x "$dir/cargo" ]]; then export PATH="$dir:$PATH"; break; fi
  done
fi

echo "==> engine"
(cd "$ROOT/engine" && cargo fmt --all -- --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked)

echo "==> app"
(cd "$ROOT/macos" && swift build)
if grep -q testTarget "$ROOT/macos/Package.swift"; then
  (cd "$ROOT/macos" && swift test)
fi

echo "==> scripts"
for s in "$ROOT"/scripts/*.sh; do bash -n "$s"; done
echo "==> checks passed"
