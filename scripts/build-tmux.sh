#!/usr/bin/env bash
# Builds the tmux bundled in the Mac app, for Macs without one (macOS doesn't come with tmux, and
# Lantern runs agents in the background with it). Prints the binary's path; reused when built.
# Licenses (tmux ISC, libevent BSD, utf8proc MIT) go to build/tmux/licenses.
#   scripts/build-tmux.sh            this Mac's architecture
#   UNIVERSAL=1 scripts/build-tmux.sh  Apple silicon and Intel
# libevent and utf8proc are linked in statically; ncurses is the system's.
# Sources are pinned by sha256 (the same tarballs and checksums as Homebrew's formulae).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TMUX_VERSION=3.8
TMUX_URL="https://github.com/tmux/tmux/releases/download/$TMUX_VERSION/tmux-$TMUX_VERSION.tar.gz"
TMUX_SHA=e79c699c7e949dccd0a4a125e17b8d1261e311b16979dbc8eb34542f3966d82e
LIBEVENT_VERSION=2.1.13
LIBEVENT_URL="https://github.com/libevent/libevent/releases/download/release-$LIBEVENT_VERSION-stable/libevent-$LIBEVENT_VERSION-stable.tar.gz"
LIBEVENT_SHA=f7e9383b8c0baa81b687e5b5eecc01beefaf1b19b64151d95ed61647fe7a315c
UTF8PROC_VERSION=2.12.0
UTF8PROC_URL="https://github.com/JuliaStrings/utf8proc/archive/refs/tags/v$UTF8PROC_VERSION.tar.gz"
UTF8PROC_SHA=f564011d38b2888d583d510b08e69ffa15aa117155db1b9b49ef1dfe1fa25111
MIN_MACOS=14.0

if [[ "${UNIVERSAL:-}" == 1 ]]; then ARCHS=(arm64 x86_64); else ARCHS=("$(uname -m)"); fi
WORK="$ROOT/build/tmux"
OUT="$WORK/tmux-$TMUX_VERSION-$(IFS=-; echo "${ARCHS[*]}")"
if [[ -x "$OUT" && -d "$WORK/licenses" ]]; then echo "$OUT"; exit 0; fi
mkdir -p "$WORK/src"

fetch() { # url sha256 file
  local file="$WORK/src/$3"
  if [[ ! -f "$file" ]]; then curl -fsSL "$1" -o "$file.part" && mv "$file.part" "$file"; fi
  if [[ "$(shasum -a 256 "$file" | cut -d' ' -f1)" != "$2" ]]; then
    rm -f "$file"
    echo "checksum mismatch: $1" >&2
    exit 1
  fi
}
fetch "$TMUX_URL" "$TMUX_SHA" "tmux-$TMUX_VERSION.tar.gz"
fetch "$LIBEVENT_URL" "$LIBEVENT_SHA" "libevent-$LIBEVENT_VERSION.tar.gz"
fetch "$UTF8PROC_URL" "$UTF8PROC_SHA" "utf8proc-$UTF8PROC_VERSION.tar.gz"

SLICES=()
for arch in "${ARCHS[@]}"; do
  dir="$WORK/$arch"
  prefix="$dir/prefix"
  rm -rf "$dir" && mkdir -p "$dir" "$prefix/lib" "$prefix/include"
  export CC="clang -arch $arch -mmacosx-version-min=$MIN_MACOS"
  host="$([[ $arch == arm64 ]] && echo aarch64 || echo x86_64)-apple-darwin"
  echo "==> tmux $TMUX_VERSION ($arch)" >&2

  tar -xzf "$WORK/src/libevent-$LIBEVENT_VERSION.tar.gz" -C "$dir"
  (cd "$dir/libevent-$LIBEVENT_VERSION-stable" &&
    ./configure --host="$host" --prefix="$prefix" --disable-shared --enable-static --disable-openssl \
      --disable-samples --disable-libevent-regress --disable-debug-mode >/dev/null &&
    make -j8 >/dev/null && make install >/dev/null)

  tar -xzf "$WORK/src/utf8proc-$UTF8PROC_VERSION.tar.gz" -C "$dir"
  (cd "$dir/utf8proc-$UTF8PROC_VERSION" &&
    $CC -O2 -DUTF8PROC_STATIC -c utf8proc.c -o utf8proc.o &&
    ar rcs "$prefix/lib/libutf8proc.a" utf8proc.o && cp utf8proc.h "$prefix/include/")

  tar -xzf "$WORK/src/tmux-$TMUX_VERSION.tar.gz" -C "$dir"
  (cd "$dir/tmux-$TMUX_VERSION" &&
    PKG_CONFIG_PATH="$prefix/lib/pkgconfig" PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig" \
      CFLAGS="-O2 -I$prefix/include -DUTF8PROC_STATIC" LDFLAGS="-L$prefix/lib" \
      LIBUTF8PROC_CFLAGS="-I$prefix/include" LIBUTF8PROC_LIBS="$prefix/lib/libutf8proc.a" \
      ./configure --host="$host" --enable-utf8proc --disable-jemalloc --disable-dependency-tracking >/dev/null &&
    make -j8 >/dev/null)
  SLICES+=("$dir/tmux-$TMUX_VERSION/tmux")
done

lipo -create -output "$OUT" "${SLICES[@]}"
# Their licenses, shipped beside it (scripts/build-app.sh).
mkdir -p "$WORK/licenses"
cp "$dir/tmux-$TMUX_VERSION/COPYING" "$WORK/licenses/tmux.txt"
cp "$dir/libevent-$LIBEVENT_VERSION-stable/LICENSE" "$WORK/licenses/libevent.txt"
cp "$dir/utf8proc-$UTF8PROC_VERSION/LICENSE.md" "$WORK/licenses/utf8proc.txt"
# Only system libraries: a Mac without Homebrew must be able to run it.
if otool -L "$OUT" | grep -E '^\s' | grep -v -E '^\s+/(usr/lib|System)/' >&2; then
  echo "tmux links a library outside the system" >&2
  exit 1
fi
echo "$OUT"
