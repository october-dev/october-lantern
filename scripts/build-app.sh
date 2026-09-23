#!/usr/bin/env bash
# Builds the engine and the macOS app, then assembles build/October Lantern.app.
#   scripts/build-app.sh            release build
#   scripts/build-app.sh --debug    debug build
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE=release
[[ "${1:-}" == "--debug" ]] && PROFILE=debug
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/engine/Cargo.toml" | head -1)"
APP="$ROOT/build/October Lantern.app"

if [[ -d /opt/homebrew/opt/rustup/bin ]]; then export PATH="/opt/homebrew/opt/rustup/bin:$PATH"; fi

echo "==> engine ($PROFILE)"
(cd "$ROOT/engine" && cargo build $([[ $PROFILE == release ]] && echo --release))

echo "==> app ($PROFILE)"
(cd "$ROOT/macos" && swift build -c "$PROFILE")
BIN_DIR="$(cd "$ROOT/macos" && swift build -c "$PROFILE" --show-bin-path)"

echo "==> bundle"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN_DIR/OctoberLantern" "$APP/Contents/MacOS/OctoberLantern"
cp "$ROOT/engine/target/$PROFILE/lantern-engine" "$APP/Contents/MacOS/lantern-engine"
cp -R "$ROOT/macos/Resources/." "$APP/Contents/Resources/"
cp "$ROOT/logo.png" "$APP/Contents/Resources/logo.png"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>October Lantern</string>
  <key>CFBundleDisplayName</key><string>October Lantern</string>
  <key>CFBundleIdentifier</key><string>dev.october.lantern</string>
  <key>CFBundleExecutable</key><string>OctoberLantern</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>Lantern listens only while you hold the mic, so you can talk to your agents.</string>
  <key>NSSpeechRecognitionUsageDescription</key><string>Lantern turns what you say into text for your agents, on your Mac.</string>
</dict>
</plist>
PLIST

# Ad-hoc signature for local runs. Release builds get a Developer ID signature and notarization.
codesign --force --sign - "$APP/Contents/MacOS/lantern-engine"
codesign --force --sign - "$APP"
echo "==> $APP"
