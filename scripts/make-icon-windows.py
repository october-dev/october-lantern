#!/usr/bin/env python3
"""Builds windows/october-lantern.ico from logo.png: the lantern, transparent, at every size
Windows asks for (16 to 256 px; each entry PNG-compressed). Run from anywhere:

    python3 scripts/make-icon-windows.py

Needs Pillow (pip install Pillow). The .ico is committed; desktop/build.rs embeds it in the
executable and windows/installer.iss uses it for Setup and the uninstaller.
"""
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SIZES = [16, 24, 32, 48, 64, 128, 256]


def main() -> None:
    logo = Image.open(ROOT / "logo.png").convert("RGBA")
    # Square, with the art centred, so no size is stretched.
    side = max(logo.size)
    canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    canvas.paste(logo, ((side - logo.width) // 2, (side - logo.height) // 2))
    largest = canvas.resize((SIZES[-1], SIZES[-1]), Image.LANCZOS)
    out = ROOT / "windows" / "october-lantern.ico"
    # Pillow downsamples from the largest image for each listed size.
    largest.save(out, format="ICO", sizes=[(size, size) for size in SIZES])
    print(f"wrote {out.relative_to(ROOT)} ({out.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
