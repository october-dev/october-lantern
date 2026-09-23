#!/usr/bin/env bash
# Publishes what scripts/release.sh built: a GitHub release with the DMG in the public releases
# repo, then the new appcast.xml (which is what installed copies check for updates).
#   scripts/publish.sh "Release notes"      (or a path to a notes file)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="harshsaver/october-lantern-releases"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/engine/Cargo.toml" | head -1)"
DMG="$ROOT/build/October-Lantern.dmg"
APPCAST="$ROOT/build/appcast.xml"
[[ -f "$DMG" && -f "$APPCAST" ]] || { echo "Run scripts/release.sh first" >&2; exit 1; }
grep -q "<sparkle:version>$VERSION<" "$APPCAST" || { echo "appcast.xml isn't for $VERSION" >&2; exit 1; }

NOTES="${1:-October Lantern $VERSION}"
NOTES_FILE="$(mktemp)"
if [[ -f "$NOTES" ]]; then cp "$NOTES" "$NOTES_FILE"; else printf '%s\n' "$NOTES" > "$NOTES_FILE"; fi

echo "==> release v$VERSION"
gh release create "v$VERSION" "$DMG" --repo "$REPO" --title "October Lantern $VERSION" --notes-file "$NOTES_FILE"

echo "==> appcast"
CHECKOUT="$ROOT/build/releases-repo"
rm -rf "$CHECKOUT"
gh repo clone "$REPO" "$CHECKOUT" -- --depth 1 -q
cp "$APPCAST" "$CHECKOUT/appcast.xml"
git -C "$CHECKOUT" add appcast.xml
git -C "$CHECKOUT" commit -q -m "October Lantern $VERSION"
git -C "$CHECKOUT" push -q
echo "==> published v$VERSION"
