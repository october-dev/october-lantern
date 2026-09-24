#!/usr/bin/env bash
# Uploads the zips from scripts/build-bus.sh to their own release in the releases repo. A zip that
# is already published is never replaced (its checksum is pinned in shipped apps).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="${RELEASES_REPO:-harshsaver/october-lantern-releases}"
TAG="$(sed -n 's/.*"tag": "\(.*\)".*/\1/p' "$ROOT/engine/bus.json")"
[[ -n "$TAG" ]] || { echo "engine/bus.json has no tag; run scripts/build-bus.sh" >&2; exit 1; }
if ! gh release view "$TAG" -R "$REPO" >/dev/null 2>&1; then
  gh release create "$TAG" -R "$REPO" --title "October Bus for Lantern ($TAG)" \
    --notes "October Bus (github.com/october-dev/october-bus, Apache-2.0), built by Lantern from a pinned commit. Lantern downloads it the first time it's needed." --latest=false
fi
existing="$(gh release view "$TAG" -R "$REPO" --json assets --jq '.assets[].name')"
for zip in "$ROOT"/build/bus/*.zip; do
  name="$(basename "$zip")"
  if grep -qx "$name" <<<"$existing"; then echo "==> $name already published"; continue; fi
  gh release upload "$TAG" "$zip" -R "$REPO"
  echo "==> uploaded $name"
done
