#!/usr/bin/env bash
# Builds the engine and the macOS app, then assembles build/October Lantern.app.
#   scripts/build-app.sh            release build
#   scripts/build-app.sh --debug    debug build
# Environment:
#   UNIVERSAL=1        build for Apple silicon and Intel
#   SIGN_IDENTITY=...  sign with this Developer ID (hardened runtime) instead of ad-hoc
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE=release
[[ "${1:-}" == "--debug" ]] && PROFILE=debug
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/engine/Cargo.toml" | head -1)"
APP="$ROOT/build/October Lantern.app"

if [[ -d /opt/homebrew/opt/rustup/bin ]]; then export PATH="/opt/homebrew/opt/rustup/bin:$PATH"; fi

CARGO_FLAGS=()
[[ $PROFILE == release ]] && CARGO_FLAGS+=(--release)
SWIFT_ARCHS=()
if [[ "${UNIVERSAL:-}" == 1 ]]; then
  echo "==> engine ($PROFILE, universal)"
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    (cd "$ROOT/engine" && cargo build ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"} --target "$t")
  done
  ENGINE="$ROOT/engine/target/lantern-engine-universal"
  lipo -create -output "$ENGINE" \
    "$ROOT/engine/target/aarch64-apple-darwin/$PROFILE/lantern-engine" \
    "$ROOT/engine/target/x86_64-apple-darwin/$PROFILE/lantern-engine"
  SWIFT_ARCHS=(--arch arm64 --arch x86_64)
else
  echo "==> engine ($PROFILE)"
  (cd "$ROOT/engine" && cargo build ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"})
  ENGINE="$ROOT/engine/target/$PROFILE/lantern-engine"
fi

echo "==> app ($PROFILE)"
(cd "$ROOT/macos" && swift build -c "$PROFILE" ${SWIFT_ARCHS[@]+"${SWIFT_ARCHS[@]}"})
BIN_DIR="$(cd "$ROOT/macos" && swift build -c "$PROFILE" ${SWIFT_ARCHS[@]+"${SWIFT_ARCHS[@]}"} --show-bin-path)"

echo "==> bundle"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"
cp "$BIN_DIR/OctoberLantern" "$APP/Contents/MacOS/OctoberLantern"
cp "$ENGINE" "$APP/Contents/MacOS/lantern-engine"
cp -R "$ROOT/macos/Resources/." "$APP/Contents/Resources/"
cp "$ROOT/logo.png" "$APP/Contents/Resources/logo.png"
ditto "$BIN_DIR/Sparkle.framework" "$APP/Contents/Frameworks/Sparkle.framework"

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
  <key>NSAppleEventsUsageDescription</key><string>Lantern types your replies into the terminal tab where each agent is running.</string>
  <key>SUFeedURL</key><string>https://raw.githubusercontent.com/harshsaver/october-lantern-releases/main/appcast.xml</string>
  <key>SUPublicEDKey</key><string>TcvtDFVfXw92Ot6tGT+5OKMmGYPg6kcNmRWXsfw0/IQ=</string>
  <key>SUEnableAutomaticChecks</key><true/>
  <key>SUScheduledCheckInterval</key><integer>86400</integer>
</dict>
</plist>
PLIST

if [[ -n "${SIGN_IDENTITY:-}" ]]; then
  # Hardened runtime (required for notarization), inside out: Sparkle's helpers, the engine, the
  # app. The app needs audio input (dictation) and Apple Events (typing into Terminal/iTerm2);
  # the engine sends those Apple Events through osascript.
  SPARKLE="$APP/Contents/Frameworks/Sparkle.framework/Versions/B"
  for part in XPCServices/Installer.xpc XPCServices/Downloader.xpc Autoupdate Updater.app; do
    codesign --force --options runtime --timestamp --preserve-metadata=entitlements --sign "$SIGN_IDENTITY" "$SPARKLE/$part"
  done
  codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP/Contents/Frameworks/Sparkle.framework"
  codesign --force --options runtime --timestamp --entitlements "$ROOT/macos/Engine.entitlements" \
    --sign "$SIGN_IDENTITY" "$APP/Contents/MacOS/lantern-engine"
  codesign --force --options runtime --timestamp --entitlements "$ROOT/macos/OctoberLantern.entitlements" \
    --sign "$SIGN_IDENTITY" "$APP"
else
  # Ad-hoc signature for local runs.
  codesign --force --deep --sign - "$APP/Contents/Frameworks/Sparkle.framework"
  codesign --force --sign - "$APP/Contents/MacOS/lantern-engine"
  codesign --force --sign - "$APP"
fi
echo "==> $APP"
