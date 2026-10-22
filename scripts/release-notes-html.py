#!/usr/bin/env python3
"""The release notes (Markdown) as the small HTML page Sparkle's update window shows.

    scripts/release-notes-html.py notes.md > notes.html

Handles what the notes use: a first line that is the title (dropped: the window has its own),
**bold** lines as headings, "- " bullets, paragraphs, **bold** and `code` inline. Light and dark.
"""
import html
import re
import sys

STYLE = """<style>
:root { color-scheme: light dark; }
body { font: 13px -apple-system, system-ui, sans-serif; line-height: 1.45; margin: 10px 14px; }
h3 { font-size: 13px; margin: 14px 0 4px; }
h3:first-child { margin-top: 2px; }
ul { margin: 4px 0; padding-left: 18px; }
li { margin: 3px 0; }
p { margin: 8px 0; color: color-mix(in srgb, CanvasText 70%, transparent); }
</style>"""


def inline(text):
    text = html.escape(text, quote=False)
    text = re.sub(r"\*\*(.+?)\*\*", r"<b>\1</b>", text)
    return re.sub(r"`(.+?)`", r"<code>\1</code>", text)


def convert(markdown):
    lines = markdown.strip().splitlines()
    if lines and not lines[0].startswith(("-", "*")):
        lines = lines[1:]  # "Lantern 0.6.13"
    out, in_list = [], False
    for line in lines:
        line = line.rstrip()
        if line.startswith("- "):
            if not in_list:
                out.append("<ul>")
                in_list = True
            out.append(f"<li>{inline(line[2:])}</li>")
            continue
        if in_list:
            out.append("</ul>")
            in_list = False
        heading = re.fullmatch(r"\*\*(.+)\*\*", line)
        if heading:
            out.append(f"<h3>{inline(heading.group(1))}</h3>")
        elif line:
            out.append(f"<p>{inline(line)}</p>")
    if in_list:
        out.append("</ul>")
    return STYLE + "\n" + "\n".join(out)


if __name__ == "__main__":
    print(convert(open(sys.argv[1], encoding="utf-8").read()))
