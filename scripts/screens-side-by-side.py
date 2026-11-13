#!/usr/bin/env python3
"""Mac and Linux screens side by side (#23).

Both sides render saved sample state (no engine or network on either side):
  Mac:   OctoberLantern --snapshot <dir>            -> welcome-*.png, settings-*.png, panel-*.png, ...
  Linux: october-lantern --fixture <surface>.json --out <png> --scale 2

  scripts/screens-side-by-side.py render <binary> <out-dir>
      renders every paired Linux fixture (run under xvfb-run; needs a display)
  scripts/screens-side-by-side.py page <mac-dir> <linux-dir> <out-dir>
      writes <out-dir>/index.html with each pair side by side (fails when an
      image is missing, so a renamed snapshot can't pass silently)

A pixel diff isn't the goal (fonts differ); the page is for a ten-second parity
scan of layout, spacing and wording.
"""
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "desktop" / "fixtures"

# (Mac snapshot stem, desktop fixture stem). No twin, no row: the Mac's
# permissions pages (welcome-1, welcome-permissions-only) and account-signed-out
# have no desktop fixture, and the desktop's welcome-apps (its step 1) and the
# extra panel/settings states have no Mac snapshot.
PAIRS = [
    ("Panels", [
        ("panel-inbox", "panel-inbox"),
        ("panel-agents", "panel-agents"),
        ("panel-new", "panel-new-session"),
        ("panel-october", "panel-october"),
        ("panel-chat", "panel-chat"),
        ("panel-assistant", "panel-assistant-home"),
        ("panel-assistant-recent", "panel-assistant-recent"),
        ("panel-assistant-task", "panel-assistant-task"),
        ("panel-assistant-signed-out", "panel-assistant-signed-out"),
        ("panel-starting", "panel-starting"),
        ("panel-starting-failed", "panel-starting-failed"),
    ]),
    ("Point & Ask", [
        ("point-ask", "point-ask"),
        ("point-ask-answer", "point-ask-answer"),
    ]),
    ("Welcome", [
        ("welcome-0", "welcome-intro"),
        ("welcome-2", "welcome-setup"),
        ("welcome-3", "welcome-done"),
    ]),
    ("Settings", [
        ("settings-0", "settings-general"),
        ("settings-1", "settings-agents"),
        ("settings-2", "settings-october"),
        ("settings-3", "settings-about"),
    ]),
]


def cmd_render(binary, out_dir):
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    for _, pairs in PAIRS:
        for mac_stem, fixture_stem in pairs:
            fixture = FIXTURES / f"{fixture_stem}.json"
            out = out_dir / f"{mac_stem}.png"
            print(f"+ {binary} --fixture {fixture.name} --out {out.name} --scale 2", flush=True)
            subprocess.run(
                [binary, "--fixture", str(fixture), "--out", str(out), "--scale", "2"],
                check=True,
            )


def cmd_page(mac_dir, linux_dir, out_dir):
    mac_dir, linux_dir, out_dir = Path(mac_dir), Path(linux_dir), Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    rows = []
    missing = []
    for group, pairs in PAIRS:
        rows.append(f"<h2>{group}</h2>")
        for mac_stem, _ in pairs:
            cells = []
            for side, src_dir in (("Mac", mac_dir), ("Linux", linux_dir)):
                src = src_dir / f"{mac_stem}.png"
                if src.is_file():
                    dest = f"{mac_stem}-{side.lower()}.png"
                    shutil.copyfile(src, out_dir / dest)
                    cells.append(f'<td><div class="cap">{side}</div><img src="{dest}" alt="{mac_stem} on {side}"></td>')
                else:
                    missing.append(str(src))
                    cells.append(f'<td><div class="cap">{side}</div><div class="missing">missing: {mac_stem}.png</div></td>')
            rows.append(f'<h3>{mac_stem}</h3><table><tr>{"".join(cells)}</tr></table>')
    (out_dir / "index.html").write_text(PAGE.format(body="\n".join(rows)))
    print(f"wrote {out_dir / 'index.html'}", flush=True)
    if missing:
        print("missing images:", *missing, sep="\n  ", file=sys.stderr)
        raise SystemExit(1)


PAGE = """<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Lantern screens: Mac vs Linux</title>
<style>
body {{ background: #111; color: #eee; font: 14px -apple-system, sans-serif; margin: 24px; }}
h1 {{ font-size: 20px; }}
h2 {{ font-size: 17px; margin-top: 36px; border-bottom: 1px solid #333; padding-bottom: 6px; }}
h3 {{ font: 13px ui-monospace, monospace; color: #aaa; margin-bottom: 6px; }}
p.note {{ color: #999; max-width: 70ch; }}
table {{ border-collapse: collapse; margin-bottom: 12px; }}
td {{ vertical-align: top; padding: 0 16px 8px 0; }}
.cap {{ color: #999; font-size: 12px; margin-bottom: 4px; }}
img {{ width: 520px; border: 1px solid #333; border-radius: 8px; background: #000; }}
.missing {{ color: #c66; font-size: 13px; }}
</style>
</head>
<body>
<h1>Lantern screens: Mac vs Linux</h1>
<p class="note">The same surfaces on both UIs, each in the same kind of state.
Fonts differ, so compare layout, spacing and wording — not pixels.</p>
{body}
</body>
</html>
"""


def main(argv):
    if len(argv) == 4 and argv[1] == "render":
        cmd_render(argv[2], argv[3])
    elif len(argv) == 5 and argv[1] == "page":
        cmd_page(argv[2], argv[3], argv[4])
    else:
        sys.exit(f"usage:\n  {Path(argv[0]).name} render <binary> <out-dir>\n  {Path(argv[0]).name} page <mac-dir> <linux-dir> <out-dir>")


if __name__ == "__main__":
    main(sys.argv)
