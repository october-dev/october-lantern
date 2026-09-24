#!/usr/bin/env bash
# Builds a signed, notarized, stapled DMG: build/October-Lantern.dmg
#
# One-time setup (stores the Apple ID app-specific password in the Keychain):
#   xcrun notarytool store-credentials lantern-notary --apple-id <id> --team-id <team> --password <app-specific>
#
# Environment:
#   SIGN_IDENTITY   Developer ID Application identity (default: the first one in the Keychain)
#   NOTARY_PROFILE  notarytool keychain profile (default: lantern-notary)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/build/October Lantern.app"
DMG="$ROOT/build/October-Lantern.dmg"
NOTARY_PROFILE="${NOTARY_PROFILE:-lantern-notary}"
if [[ -z "${SIGN_IDENTITY:-}" ]]; then
  SIGN_IDENTITY="$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application:.*\)"/\1/p' | head -1)"
fi
[[ -n "$SIGN_IDENTITY" ]] || { echo "No Developer ID Application identity found" >&2; exit 1; }
echo "==> signing as: $SIGN_IDENTITY"

echo "==> checks"
if [[ -d /opt/homebrew/opt/rustup/bin ]]; then export PATH="/opt/homebrew/opt/rustup/bin:$PATH"; fi
(cd "$ROOT/engine" && cargo fmt --all -- --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked)
bash -n "$ROOT/scripts/build-app.sh" "$ROOT/scripts/publish.sh"

UNIVERSAL=1 SIGN_IDENTITY="$SIGN_IDENTITY" "$ROOT/scripts/build-app.sh"
codesign --verify --deep --strict "$APP"

echo "==> notarizing app"
ZIP="$ROOT/build/OctoberLantern-notarize.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
xcrun notarytool submit "$ZIP" --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple "$APP"
rm -f "$ZIP"

echo "==> dmg"
STAGE="$ROOT/build/dmg"
rm -rf "$STAGE" "$DMG"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "October Lantern" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$DMG" >/dev/null
rm -rf "$STAGE"
codesign --force --timestamp --sign "$SIGN_IDENTITY" "$DMG"

echo "==> notarizing dmg"
xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple "$DMG"
spctl --assess --type open --context context:primary-signature -v "$DMG"
echo "==> $DMG"

echo "==> sparkle signature and appcast"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/engine/Cargo.toml" | head -1)"
SIGN_UPDATE="$ROOT/macos/.build/artifacts/sparkle/Sparkle/bin/sign_update"
# The update-signing key: an exported copy if present (no Keychain prompt), else the Keychain.
KEY_FILE="$HOME/Library/Application Support/October Lantern Release/sparkle_private_key"
if [[ -f "$KEY_FILE" ]]; then
  SIGNATURE="$("$SIGN_UPDATE" --ed-key-file "$KEY_FILE" "$DMG")"   # sparkle:edSignature="…" length="…"
else
  SIGNATURE="$("$SIGN_UPDATE" --account october-lantern "$DMG")"
fi
cat > "$ROOT/build/appcast.xml" <<XML
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>October Lantern</title>
    <link>https://lantern.october.dev</link>
    <item>
      <title>October Lantern $VERSION</title>
      <pubDate>$(LC_ALL=C date -u "+%a, %d %b %Y %H:%M:%S +0000")</pubDate>
      <sparkle:version>$VERSION</sparkle:version>
      <sparkle:shortVersionString>$VERSION</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
      <sparkle:releaseNotesLink>https://github.com/harshsaver/october-lantern-releases/releases/tag/v$VERSION</sparkle:releaseNotesLink>
      <enclosure url="https://github.com/harshsaver/october-lantern-releases/releases/download/v$VERSION/October-Lantern.dmg" type="application/octet-stream" $SIGNATURE />
    </item>
  </channel>
</rss>
XML
echo "==> build/appcast.xml for $VERSION"
