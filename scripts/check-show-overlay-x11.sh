#!/usr/bin/env bash
# Run under xvfb-run, on a disposable CI display. No engine, account, model or user desktop.
set -euo pipefail
cd "$(dirname "$0")/.."
test -n "${DISPLAY:-}" || { echo 'Needs an isolated Xvfb display' >&2; exit 1; }
logs=$(mktemp -d)
openbox >"$logs/openbox.log" 2>&1 &
wm_pid=$!
xcompmgr >"$logs/compositor.log" 2>&1 &
compositor_pid=$!
cleanup() {
  kill "$wm_pid" "$compositor_pid" 2>/dev/null || true
  wait "$wm_pid" "$compositor_pid" 2>/dev/null || true
  rm -rf "$logs"
}
trap cleanup EXIT
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/build/target}"
cargo test --locked --manifest-path desktop/Cargo.toml native_x11_overlay -- --ignored --test-threads=1
