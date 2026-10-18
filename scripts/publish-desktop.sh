#!/usr/bin/env bash
# Publishes a Windows or Linux build made by the release workflows (windows-release.yml,
# linux-release.yml) to the public releases repo, in the same release as the Mac's DMG. Installed
# copies update from the latest release there.
#   scripts/publish-desktop.sh windows <run id> ["Release notes" | notes file]
#   scripts/publish-desktop.sh linux <run id> ["Release notes" | notes file]
#
# Nothing is uploaded until the build is for this checkout's version, its files match their
# checksums, it updates from the releases repo, and (Windows) every program in it is signed. A
# published file is never replaced: a rerun skips an identical one and refuses a different one.
#
# Environment:
#   RELEASES_REPO    GitHub repo to publish to (default: harshsaver/october-lantern-releases)
#   SOURCE_REPO      repo whose workflow made the build (default: october-dev/october-lantern)
#   ALLOW_UNSIGNED=1 publish an unsigned Windows build (Windows warns before running it)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="${RELEASES_REPO:-harshsaver/october-lantern-releases}"
SOURCE="${SOURCE_REPO:-october-dev/october-lantern}"
SYSTEM="${1:-}"
RUN="${2:-}"
NOTES="${3:-}"
case "$SYSTEM" in
  # The speech runtime zip: what a copy updated through the in-app updater downloads on first
  # dictation, from this same release.
  windows) FILES=(October-Lantern-windows-x86_64-setup.exe October-Lantern-windows-x86_64.zip October-Lantern-whisper-windows-x86_64.zip) ;;
  linux) FILES=(October-Lantern-linux-x86_64.tar.gz) ;;
  *) echo "usage: scripts/publish-desktop.sh windows|linux <run id> [notes]" >&2; exit 2 ;;
esac
[[ -n "$RUN" ]] || { echo "usage: scripts/publish-desktop.sh $SYSTEM <run id> [notes]" >&2; exit 2; }
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/engine/Cargo.toml" | head -1)"
TAG="v$VERSION"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
fail() { echo "$*" >&2; exit 1; }

echo "==> downloading run $RUN from $SOURCE"
[[ "$(gh run view "$RUN" --repo "$SOURCE" --json conclusion -q .conclusion)" == success ]] || fail "run $RUN didn't succeed"
gh run download "$RUN" --repo "$SOURCE" --name "October-Lantern-$SYSTEM-x86_64" --dir "$WORK/build"

echo "==> verifying"
INFO="$WORK/build/release-info.json"
[[ -f "$INFO" ]] || fail "the build has no release-info.json; rebuild it with the current release workflow"
field() { python3 -c 'import json,sys; v=json.load(open(sys.argv[1], encoding="utf-8-sig")).get(sys.argv[2]); print(str(v).lower() if isinstance(v, bool) else v)' "$INFO" "$1"; }
[[ "$(field version)" == "$VERSION" ]] || fail "the build is version $(field version), not $VERSION"
[[ "$(field releases)" == "$REPO" ]] || fail "the build updates from $(field releases), not $REPO"
COMMIT="$(field commit)"
[[ "$COMMIT" == "$(git -C "$ROOT" rev-parse HEAD)" ]] || echo "    note: built from $COMMIT, not this checkout's HEAD"
if [[ "$SYSTEM" == windows ]]; then
  [[ "$(field speech)" == October-Lantern-whisper-windows-x86_64.zip && -f "$WORK/build/October-Lantern-whisper-windows-x86_64.zip" ]] ||
    fail "the build has no speech runtime zip; rebuild it with the current release workflow"
  # The updater zip must stay installable by 0.6.2's updater: exactly the core files.
  listed="$(python3 -c 'import sys, zipfile; print("\n".join(zipfile.ZipFile(sys.argv[1]).namelist()))' "$WORK/build/October-Lantern-windows-x86_64.zip" | sort)"
  core="$(printf '%s\n' LICENSE OCTOBER-BUS-COMMIT OCTOBER-BUS-LICENSE lantern-engine.exe october-bus.exe october-lantern.exe october-lantern.png | sort)"
  [[ "$listed" == "$core" ]] || fail "the updater zip doesn't hold exactly the core files older versions expect"
  speech="$(python3 -c 'import sys, zipfile; print("\n".join(zipfile.ZipFile(sys.argv[1]).namelist()))' "$WORK/build/October-Lantern-whisper-windows-x86_64.zip" | sort)"
  [[ "$speech" == "$(printf '%s\n' WHISPER-LICENSE whisper-cli.exe | sort)" ]] || fail "the speech runtime zip doesn't hold exactly whisper-cli.exe and WHISPER-LICENSE"
fi
if [[ "$SYSTEM" == windows && "$(field signed)" != true ]]; then
  [[ "${ALLOW_UNSIGNED:-}" == 1 ]] || fail "the Windows build isn't signed; set up signing (docs/RELEASING.md) or set ALLOW_UNSIGNED=1"
  echo "    warning: publishing an unsigned Windows build"
fi
UPLOADS=()
for f in "${FILES[@]}"; do
  [[ -f "$WORK/build/$f" && -f "$WORK/build/$f.sha256" ]] || fail "the build is missing $f"
  expected="$(cut -d' ' -f1 < "$WORK/build/$f.sha256" | tr -d '\r\n')"
  actual="$(shasum -a 256 "$WORK/build/$f" | cut -d' ' -f1)"
  [[ "$expected" == "$actual" ]] || fail "$f doesn't match its checksum"
  echo "    $f  $actual"
  UPLOADS+=("$WORK/build/$f" "$WORK/build/$f.sha256")
done

echo "==> release $TAG in $REPO"
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  published="$(gh release view "$TAG" --repo "$REPO" --json assets -q '.assets[].name')"
  NEW=()
  for path in "${UPLOADS[@]}"; do
    name="$(basename "$path")"
    if grep -qxF "$name" <<<"$published"; then
      mkdir -p "$WORK/published"
      gh release download "$TAG" --repo "$REPO" --pattern "$name" --dir "$WORK/published" --clobber
      cmp -s "$path" "$WORK/published/$name" || fail "$TAG already has a different $name; publish a new version instead"
      echo "    $name already published"
    else
      NEW+=("$path")
    fi
  done
  if ((${#NEW[@]})); then gh release upload "$TAG" "${NEW[@]}" --repo "$REPO"; fi
else
  NOTES_FILE="$WORK/notes.md"
  if [[ -n "$NOTES" && -f "$NOTES" ]]; then cp "$NOTES" "$NOTES_FILE"; else printf '%s\n' "${NOTES:-October Lantern $VERSION}" > "$NOTES_FILE"; fi
  gh release create "$TAG" "${UPLOADS[@]}" --repo "$REPO" --title "October Lantern $VERSION" --notes-file "$NOTES_FILE"
fi
echo "==> lantern.october.dev"
# Uploading files to an existing release fires no release event: start the site's update now.
gh workflow run update-lantern-site.yml --repo "$REPO" 2>/dev/null \
  || echo "    couldn't start the site update; it runs hourly, or: gh workflow run update-lantern-site.yml -R $REPO -f force=true"
echo "==> published $SYSTEM $TAG: https://github.com/$REPO/releases/tag/$TAG"
