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
