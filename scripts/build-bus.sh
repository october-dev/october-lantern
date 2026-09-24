#!/usr/bin/env bash
# Builds the October Bus that Lantern downloads the first time it's needed: one small, signed zip
# per chip, from a pinned commit of github.com/october-dev/october-bus (Apache-2.0). Writes
# engine/bus.json (commit, download URLs, SHA-256), which the engine compiles in and checks
# every download against. Upload with scripts/publish-bus.sh.
#
# Environment: BUS_COMMIT (pinned commit), OCTOBER_BUS_SRC (a checkout), SIGN_IDENTITY.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUS_COMMIT="${BUS_COMMIT:-20b745682818f33b7d03dc7a3e65d32a71ded35b}"
SHORT="${BUS_COMMIT:0:7}"
BUS_SRC="${OCTOBER_BUS_SRC:-$ROOT/build/october-bus-src}"
OUT="$ROOT/build/bus"
TAG="bus-$SHORT"
REPO="${RELEASES_REPO:-harshsaver/october-lantern-releases}"
if [[ -z "${SIGN_IDENTITY:-}" ]]; then
  SIGN_IDENTITY="$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application:.*\)"/\1/p' | head -1)"
fi
[[ -n "$SIGN_IDENTITY" ]] || { echo "No Developer ID Application identity found" >&2; exit 1; }
command -v go >/dev/null || { echo "Go is needed to build October Bus" >&2; exit 1; }

if [[ ! -d "$BUS_SRC/.git" ]]; then git clone -q https://github.com/october-dev/october-bus.git "$BUS_SRC"; fi
git -C "$BUS_SRC" cat-file -e "$BUS_COMMIT^{commit}" 2>/dev/null || git -C "$BUS_SRC" fetch -q origin
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
(cd "$BUS_SRC" && git archive "$BUS_COMMIT" | tar -x -C "$WORK")
rm -rf "$OUT" && mkdir -p "$OUT"

entries=()
for arch in arm64 x86_64; do
  goarch=$arch; [[ $arch == x86_64 ]] && goarch=amd64
  bin="$WORK/october-bus"
  (cd "$WORK" && CGO_ENABLED=0 GOOS=darwin GOARCH=$goarch go build -trimpath -ldflags "-s -w" -o "$bin" ./cmd/october-bus)
  codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$bin"
  zip="$OUT/october-bus-$SHORT-$arch.zip"
  (cd "$WORK" && ditto -c -k october-bus "$zip")
  rm -f "$bin"
  sum="$(shasum -a 256 "$zip" | cut -d' ' -f1)"
  echo "==> $arch: $(du -h "$zip" | cut -f1) $sum"
  entries+=("\"$arch\": {\"url\": \"https://github.com/$REPO/releases/download/$TAG/october-bus-$SHORT-$arch.zip\", \"sha256\": \"$sum\"}")
done

{
  echo "{"
  echo "  \"commit\": \"$BUS_COMMIT\","
  echo "  \"tag\": \"$TAG\","
  echo "  ${entries[0]},"
  echo "  ${entries[1]}"
  echo "}"
} > "$ROOT/engine/bus.json"
echo "==> engine/bus.json written; now run scripts/publish-bus.sh"
