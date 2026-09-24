#!/usr/bin/env bash
# Publishes what scripts/release.sh built: a GitHub release with the DMG in the public releases
# repo, then the new appcast.xml (which is what installed copies check for updates).
#   scripts/publish.sh "Release notes"      (or a path to a notes file)
#
# Nothing is uploaded until the DMG, its Sparkle signature, its length and its version all match
# the appcast. A published DMG is never replaced: a rerun skips an identical one and refuses a
# different one. Rerunning after any failed step finishes the publication.
#
# Environment (for testing against stand-ins):
#   RELEASES_REPO     GitHub repo to publish to (default: harshsaver/october-lantern-releases)
#   SIGN_UPDATE       Sparkle's sign_update (default: the one in the Swift build's artifacts)
#   SPARKLE_KEY_FILE  update-signing key file (default: the exported release key; else the Keychain)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="${RELEASES_REPO:-harshsaver/october-lantern-releases}"
ASSET="October-Lantern.dmg"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/engine/Cargo.toml" | head -1)"
DMG="$ROOT/build/$ASSET"
APPCAST="$ROOT/build/appcast.xml"
SIGN_UPDATE="${SIGN_UPDATE:-$ROOT/macos/.build/artifacts/sparkle/Sparkle/bin/sign_update}"
KEY_FILE="${SPARKLE_KEY_FILE:-$HOME/Library/Application Support/October Lantern Release/sparkle_private_key}"
[[ -f "$DMG" && -f "$APPCAST" ]] || { echo "Run scripts/release.sh first" >&2; exit 1; }

WORK="$(mktemp -d)"
MOUNT="$WORK/mount"
MOUNTED=0
cleanup() {
  if [[ $MOUNTED == 1 ]]; then hdiutil detach "$MOUNT" -quiet || hdiutil detach "$MOUNT" -force -quiet || true; fi
  rm -rf "$WORK"
}
trap cleanup EXIT
fail() { echo "$*" >&2; exit 1; }

echo "==> verifying the DMG against the appcast"
attr() { sed -n "s/.*$1=\"\([^\"]*\)\".*/\1/p" "$APPCAST" | head -1; }
FEED_VERSION="$(sed -n 's/.*<sparkle:version>\(.*\)<\/sparkle:version>.*/\1/p' "$APPCAST" | head -1)"
FEED_URL="$(attr 'enclosure url')"
FEED_LENGTH="$(attr length)"
FEED_SIGNATURE="$(attr 'sparkle:edSignature')"
EXPECTED_URL="https://github.com/$REPO/releases/download/v$VERSION/$ASSET"
[[ "$FEED_VERSION" == "$VERSION" ]] || fail "appcast.xml is for '$FEED_VERSION', not $VERSION"
[[ "$FEED_URL" == "$EXPECTED_URL" ]] || fail "appcast.xml points at '$FEED_URL', not $EXPECTED_URL"
SIZE="$(stat -f %z "$DMG")"
[[ "$FEED_LENGTH" == "$SIZE" ]] || fail "appcast.xml says $FEED_LENGTH bytes; the DMG is $SIZE"
[[ -n "$FEED_SIGNATURE" ]] || fail "appcast.xml has no sparkle:edSignature"
if [[ -f "$KEY_FILE" ]]; then KEY_ARGS=(--ed-key-file "$KEY_FILE"); else KEY_ARGS=(--account october-lantern); fi
"$SIGN_UPDATE" --verify "${KEY_ARGS[@]}" "$DMG" "$FEED_SIGNATURE" >/dev/null \
  || fail "the appcast's signature doesn't match the DMG"

# The app inside the DMG must be the version the appcast announces.
mkdir "$MOUNT"
hdiutil attach -nobrowse -readonly -mountpoint "$MOUNT" "$DMG" >/dev/null
MOUNTED=1
DMG_VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$MOUNT/October Lantern.app/Contents/Info.plist")"
hdiutil detach "$MOUNT" -quiet
MOUNTED=0
[[ "$DMG_VERSION" == "$VERSION" ]] || fail "DMG holds $DMG_VERSION, not $VERSION"
SHA="$(shasum -a 256 "$DMG" | cut -d' ' -f1)"
echo "    $VERSION, $SIZE bytes, sha256 $SHA"

NOTES="${1:-October Lantern $VERSION}"
NOTES_FILE="$WORK/notes.md"
if [[ -f "$NOTES" ]]; then cp "$NOTES" "$NOTES_FILE"; else printf '%s\n' "$NOTES" > "$NOTES_FILE"; fi

echo "==> release v$VERSION"
if gh release view "v$VERSION" --repo "$REPO" >/dev/null 2>&1; then
  if gh release view "v$VERSION" --repo "$REPO" --json assets -q '.assets[].name' | grep -qxF "$ASSET"; then
    # Already uploaded (a rerun): fine only if it is these exact bytes.
    gh release download "v$VERSION" --repo "$REPO" --pattern "$ASSET" --dir "$WORK/published"
    PUBLISHED_SHA="$(shasum -a 256 "$WORK/published/$ASSET" | cut -d' ' -f1)"
    [[ "$PUBLISHED_SHA" == "$SHA" ]] || fail "v$VERSION already has a different $ASSET (sha256 $PUBLISHED_SHA). Published assets aren't replaced: bump the version, or delete that asset by hand if it was never announced."
    echo "    $ASSET already published, identical"
  else
    gh release upload "v$VERSION" "$DMG" --repo "$REPO"
  fi
else
  gh release create "v$VERSION" "$DMG" --repo "$REPO" --title "October Lantern $VERSION" --notes-file "$NOTES_FILE"
fi

echo "==> appcast"
CHECKOUT="$WORK/releases-repo"
gh repo clone "$REPO" "$CHECKOUT" -- --depth 1 -q
cp "$APPCAST" "$CHECKOUT/appcast.xml"
git -C "$CHECKOUT" add appcast.xml
if git -C "$CHECKOUT" diff --cached --quiet; then
  echo "    appcast.xml already published"
else
  git -C "$CHECKOUT" commit -q -m "October Lantern $VERSION"
  git -C "$CHECKOUT" push -q
fi
echo "==> published v$VERSION"
