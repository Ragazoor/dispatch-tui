"""Turn `tmux capture-pane -e -p` output into a styled HTML page.

usage: ansi2html.py <in.ansi> <out.html> [title]
"""
import html
import re
import sys

BG = "#1a1b26"
FG = "#c0caf5"
# Tokyo Night-ish 16-colour palette
BASE16 = [
    "#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6",
    "#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5",
]


def xterm256(n):
    if n < 16:
        return BASE16[n]
    if n < 232:
        n -= 16
        steps = [0, 95, 135, 175, 215, 255]
        return "#%02x%02x%02x" % (steps[n // 36], steps[(n // 6) % 6], steps[n % 6])
    v = 8 + (n - 232) * 10
    return "#%02x%02x%02x" % (v, v, v)


class Style:
    def __init__(self):
        self.reset()

    def reset(self):
        self.fg = None
        self.bg = None
        self.bold = self.dim = self.italic = self.underline = self.reverse = False

    def css(self):
        fg, bg = self.fg, self.bg
        if self.reverse:
            fg, bg = (bg or BG), (fg or FG)
        out = []
        if fg:
            out.append(f"color:{fg}")
        if bg:
            out.append(f"background:{bg}")
        if self.bold:
            out.append("font-weight:700")
        if self.dim:
            out.append("opacity:.6")
        if self.italic:
            out.append("font-style:italic")
        if self.underline:
            out.append("text-decoration:underline")
        return ";".join(out)


def apply(style, params):
    p = [int(x) if x else 0 for x in params.split(";")] if params else [0]
    i = 0
    while i < len(p):
        c = p[i]
        if c == 0:
            style.reset()
        elif c == 1:
            style.bold = True
        elif c == 2:
            style.dim = True
        elif c == 3:
            style.italic = True
        elif c == 4:
            style.underline = True
        elif c == 7:
            style.reverse = True
        elif c == 22:
            style.bold = style.dim = False
        elif c == 23:
            style.italic = False
        elif c == 24:
            style.underline = False
        elif c == 27:
            style.reverse = False
        elif 30 <= c <= 37:
            style.fg = BASE16[c - 30]
        elif 90 <= c <= 97:
            style.fg = BASE16[c - 90 + 8]
        elif 40 <= c <= 47:
            style.bg = BASE16[c - 40]
        elif 100 <= c <= 107:
            style.bg = BASE16[c - 100 + 8]
        elif c == 39:
            style.fg = None
        elif c == 49:
            style.bg = None
        elif c in (38, 48):
            if i + 1 < len(p) and p[i + 1] == 5:
                col = xterm256(p[i + 2])
                i += 2
            elif i + 1 < len(p) and p[i + 1] == 2:
                col = "#%02x%02x%02x" % tuple(p[i + 2 : i + 5])
                i += 4
            else:
                col = None
            if c == 38:
                style.fg = col
            else:
                style.bg = col
        i += 1


SGR = re.compile(r"\x1b\[([0-9;]*)m")


def convert(text):
    style = Style()
    lines = []
    for raw in text.rstrip("\n").split("\n"):
        out = []
        # split() alternates text, SGR params, text, ... ending on text.
        parts = SGR.split(raw)
        for i, part in enumerate(parts):
            if i % 2:
                apply(style, part)
            elif part:
                out.append(f'<span style="{style.css()}">{html.escape(part)}</span>')
        lines.append("".join(out))
    return "\n".join(lines)


def main():
    src, dst = sys.argv[1], sys.argv[2]
    title = sys.argv[3] if len(sys.argv) > 3 else "dispatch"
    body = convert(open(src, encoding="utf-8").read())
    page = f"""<!doctype html><html><head><meta charset="utf-8"><style>
html,body{{margin:0;background:#0f0f14}}
.win{{display:inline-block;margin:28px;border-radius:10px;overflow:hidden;
  box-shadow:0 12px 40px rgba(0,0,0,.55);border:1px solid #2a2b3d}}
.bar{{background:#24253a;height:30px;display:flex;align-items:center;padding:0 12px;gap:8px;
  font:13px system-ui,sans-serif;color:#8089b3}}
.dot{{width:12px;height:12px;border-radius:50%}}
.t{{flex:1;text-align:center;margin-right:60px}}
pre{{margin:0;padding:10px 14px;background:{BG};color:{FG};
  font-family:'JetBrainsMono Nerd Font Mono','JetBrains Mono',monospace;font-size:14px;line-height:1.25}}
</style></head><body><div class="win"><div class="bar">
<span class="dot" style="background:#ff5f57"></span><span class="dot" style="background:#febc2e"></span>
<span class="dot" style="background:#28c840"></span><span class="t">{html.escape(title)}</span></div>
<pre>{body}</pre></div></body></html>"""
    open(dst, "w", encoding="utf-8").write(page)


main()
