#!/usr/bin/env python3
"""Pictures of a terminal program: run it in a pseudo-terminal of a fixed size, type a
script at it, and write each screen it shows as an HTML page (a grid of coloured cells),
for tools/shots/render.mjs to turn into PNGs. No dependencies beyond Python's own.

    pty_shots.py <out dir> <script.json>

The script: {"cols": 120, "rows": 36, "env": {...}, "sessions": [{"name": "main",
"argv": [...], "steps": [...]}]}. A step is ["wait", seconds], ["keys", "text"] (with
\\u001b for Esc, \\r for Enter, \\t for Tab), or ["shot", "name"], which writes
<out dir>/<session>-<name>.html and a plain-text twin. At the end the program gets SIGTERM.

The emulator knows what full-screen programs send (cursor moves, erase, SGR colours: 16,
256 and true colour, bold, dim, italic, underline, reverse, the alternate screen) and
answers the cursor-position and device-attribute queries a terminal library sends at
start. It is not a full terminal: enough for a picture, nothing more.
"""
import fcntl
import html
import json
import os
import pty
import select
import signal
import struct
import sys
import termios
import time
import unicodedata

BASE16 = [
    "#1d1f21", "#cc6666", "#b5bd68", "#f0c674", "#81a2be", "#b294bb", "#8abeb7", "#c5c8c6",
    "#666666", "#ff7b7b", "#d0e08a", "#ffe08a", "#a3c4e6", "#d3b2e0", "#a8e0d8", "#ffffff",
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


class Cell:
    __slots__ = ("ch", "fg", "bg", "bold", "dim", "ital", "under", "rev", "cont")

    def __init__(self, ch=" ", a=None):
        self.ch = ch
        self.cont = False
        if a is None:
            self.fg = self.bg = None
            self.bold = self.dim = self.ital = self.under = self.rev = False
        else:
            self.fg, self.bg, self.bold, self.dim, self.ital, self.under, self.rev = a


class Screen:
    def __init__(self, cols, rows):
        self.cols, self.rows = cols, rows
        self.reset_attr()
        self.grid = self.blank()
        self.x = self.y = 0
        self.saved = (0, 0)
        self.top, self.bottom = 0, rows - 1
        self.answers = b""
        self.buf = ""

    def reset_attr(self):
        self.attr = [None, None, False, False, False, False, False]

    def blank(self):
        return [[Cell() for _ in range(self.cols)] for _ in range(self.rows)]

    def blank_cell(self):
        a = list(self.attr)
        a[2:] = [False] * 5
        return Cell(" ", a)

    def scroll_up(self):
        del self.grid[self.top]
        self.grid.insert(self.bottom, [self.blank_cell() for _ in range(self.cols)])

    def newline(self):
        if self.y == self.bottom:
            self.scroll_up()
        elif self.y < self.rows - 1:
            self.y += 1

    def put(self, ch):
        w = 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1
        if unicodedata.combining(ch):
            return
        if self.x + w > self.cols:
            self.x = 0
            self.newline()
        c = Cell(ch, list(self.attr))
        self.grid[self.y][self.x] = c
        if w == 2 and self.x + 1 < self.cols:
            n = Cell("", list(self.attr))
            n.cont = True
            self.grid[self.y][self.x + 1] = n
        self.x += w

    def feed(self, data):
        self.buf += data
        s, i, n = self.buf, 0, len(self.buf)
        while i < n:
            c = s[i]
            if c == "\x1b":
                j = self.escape(s, i)
                if j is None:
                    break
                i = j
                continue
            if c == "\r":
                self.x = 0
            elif c == "\n" or c == "\x0b" or c == "\x0c":
                self.newline()
            elif c == "\b":
                self.x = max(0, self.x - 1)
            elif c == "\t":
                self.x = min(self.cols - 1, (self.x // 8 + 1) * 8)
            elif c == "\x07" or ord(c) < 32:
                pass
            else:
                self.put(c)
            i += 1
        self.buf = s[i:]

    def escape(self, s, i):
        """Handle the sequence at s[i] (ESC); the index after it, or None if incomplete."""
        if i + 1 >= len(s):
            return None
        k = s[i + 1]
        if k == "[":
            j = i + 2
            while j < len(s) and not ("\x40" <= s[j] <= "\x7e"):
                j += 1
            if j >= len(s):
                return None
            self.csi(s[i + 2:j], s[j])
            return j + 1
        if k == "]" or k == "P" or k == "_":
            j = i + 2
            while j < len(s):
                if s[j] == "\x07":
                    return j + 1
                if s[j] == "\x1b" and j + 1 < len(s) and s[j + 1] == "\\":
                    return j + 2
                j += 1
            return None
        if k in "()*+":
            return i + 3 if i + 2 < len(s) else None
        if k == "7":
            self.saved = (self.x, self.y)
        elif k == "8":
            self.x, self.y = self.saved
        elif k == "M":
            if self.y == self.top:
                del self.grid[self.bottom]
                self.grid.insert(self.top, [self.blank_cell() for _ in range(self.cols)])
            else:
                self.y = max(0, self.y - 1)
        elif k == "c":
            self.grid = self.blank()
            self.x = self.y = 0
        return i + 2

    def csi(self, params, final):
        private = params[:1] in ("?", ">", "=", "<")
        body = params[1:] if private else params
        try:
            nums = [int(p) if p else None for p in body.replace(":", ";").split(";")] if body else []
        except ValueError:
            nums = []

        def arg(k, d):
            v = nums[k] if k < len(nums) else None
            return d if v is None or v == 0 else v

        if private:
            if final in "hl" and any(n in (47, 1047, 1049) for n in nums if n):
                self.grid = self.blank()
                if final == "h":
                    self.x = self.y = 0
            elif final == "c" or (final == "u" and params.startswith("?")):
                pass
            return
        if final in "Hf":
            self.y = min(self.rows - 1, arg(0, 1) - 1)
            self.x = min(self.cols - 1, arg(1, 1) - 1)
        elif final == "A":
            self.y = max(0, self.y - arg(0, 1))
        elif final in "Be":
            self.y = min(self.rows - 1, self.y + arg(0, 1))
        elif final in "Ca":
            self.x = min(self.cols - 1, self.x + arg(0, 1))
        elif final == "D":
            self.x = max(0, self.x - arg(0, 1))
        elif final == "E":
            self.x, self.y = 0, min(self.rows - 1, self.y + arg(0, 1))
        elif final == "F":
            self.x, self.y = 0, max(0, self.y - arg(0, 1))
        elif final in "G`":
            self.x = min(self.cols - 1, arg(0, 1) - 1)
        elif final == "d":
            self.y = min(self.rows - 1, arg(0, 1) - 1)
        elif final == "J":
            m = nums[0] if nums and nums[0] else 0
            if m == 0:
                self.erase_line(self.y, self.x, self.cols)
                for r in range(self.y + 1, self.rows):
                    self.erase_line(r, 0, self.cols)
            elif m == 1:
                for r in range(0, self.y):
                    self.erase_line(r, 0, self.cols)
                self.erase_line(self.y, 0, self.x + 1)
            else:
                for r in range(self.rows):
                    self.erase_line(r, 0, self.cols)
        elif final == "K":
            m = nums[0] if nums and nums[0] else 0
            if m == 0:
                self.erase_line(self.y, self.x, self.cols)
            elif m == 1:
                self.erase_line(self.y, 0, self.x + 1)
            else:
                self.erase_line(self.y, 0, self.cols)
        elif final == "X":
            self.erase_line(self.y, self.x, min(self.cols, self.x + arg(0, 1)))
        elif final == "P":
            row = self.grid[self.y]
            k = arg(0, 1)
            del row[self.x:self.x + k]
            row.extend(self.blank_cell() for _ in range(self.cols - len(row)))
        elif final == "@":
            row = self.grid[self.y]
            for _ in range(arg(0, 1)):
                row.insert(self.x, self.blank_cell())
            del row[self.cols:]
        elif final == "S":
            for _ in range(arg(0, 1)):
                self.scroll_up()
        elif final == "r":
            self.top = arg(0, 1) - 1
            self.bottom = min(self.rows, arg(1, self.rows)) - 1
            self.x = self.y = 0
        elif final == "m":
            self.sgr(nums or [0])
        elif final == "n" and nums and nums[0] == 6:
            self.answers += b"\x1b[%d;%dR" % (self.y + 1, self.x + 1)
        elif final == "c":
            self.answers += b"\x1b[?62;22c"
        elif final == "s":
            self.saved = (self.x, self.y)
        elif final == "u":
            self.x, self.y = self.saved

    def erase_line(self, r, a, b):
        for c in range(max(0, a), min(self.cols, b)):
            self.grid[r][c] = self.blank_cell()

    def sgr(self, nums):
        i = 0
        a = self.attr
        while i < len(nums):
            n = nums[i] or 0
            if n == 0:
                self.reset_attr()
                a = self.attr
            elif n == 1:
                a[2] = True
            elif n == 2:
                a[3] = True
            elif n == 3:
                a[4] = True
            elif n == 4:
                a[5] = True
            elif n == 7:
                a[6] = True
            elif n == 22:
                a[2] = a[3] = False
            elif n == 23:
                a[4] = False
            elif n == 24:
                a[5] = False
            elif n == 27:
                a[6] = False
            elif 30 <= n <= 37:
                a[0] = BASE16[n - 30]
            elif 90 <= n <= 97:
                a[0] = BASE16[n - 90 + 8]
            elif 40 <= n <= 47:
                a[1] = BASE16[n - 40]
            elif 100 <= n <= 107:
                a[1] = BASE16[n - 100 + 8]
            elif n == 39:
                a[0] = None
            elif n == 49:
                a[1] = None
            elif n in (38, 48):
                slot = 0 if n == 38 else 1
                mode = nums[i + 1] if i + 1 < len(nums) else None
                if mode == 5 and i + 2 < len(nums):
                    a[slot] = xterm256(nums[i + 2] or 0)
                    i += 2
                elif mode == 2 and i + 4 < len(nums):
                    r, g, b = (nums[i + 2] or 0, nums[i + 3] or 0, nums[i + 4] or 0)
                    a[slot] = "#%02x%02x%02x" % (r, g, b)
                    i += 4
            i += 1

    def text(self):
        return "\n".join("".join(c.ch for c in row if not c.cont).rstrip() for row in self.grid)

    def html(self, title, fg, bg):
        # ASCII runs share a span; any other character gets a box of its cell width, so a
        # glyph from a fallback font (box drawing, blocks) cannot shift the columns after it.
        out = []
        for row in self.grid:
            spans, last, run = [], None, []

            def flush():
                if run:
                    spans.append('<span style="%s">%s</span>' % (last, html.escape("".join(run))))
                    run.clear()

            for i, c in enumerate(row):
                if c.cont:
                    continue
                f, b = c.fg or "var(--fg)", c.bg or "var(--bg)"
                if c.rev:
                    f, b = b, f
                style = "color:%s;background:%s" % (f, b)
                if c.bold:
                    style += ";font-weight:bold"
                if c.dim:
                    style += ";opacity:.6"
                if c.ital:
                    style += ";font-style:italic"
                if c.under:
                    style += ";text-decoration:underline"
                if ord(c.ch[:1] or " ") >= 128:
                    flush()
                    w = 2 if i + 1 < len(row) and row[i + 1].cont else 1
                    spans.append('<span class=w%d style="%s">%s</span>' % (w, style, html.escape(c.ch)))
                    last = None
                    continue
                if style != last:
                    flush()
                last = style
                run.append(c.ch)
            flush()
            out.append("".join(spans))
        return (
            "<!doctype html><meta charset=utf-8><title>%s</title><style>"
            ":root{--fg:%s;--bg:%s}:root.light{--fg:#1d1f21;--bg:#fafafa}"
            "html,body{margin:0;background:var(--bg)}"
            "pre{margin:0;padding:12px;font:15px/18px 'Liberation Mono','DejaVu Sans Mono',monospace;"
            "display:inline-block;background:var(--bg);color:var(--fg)}"
            "pre span{display:inline-block;height:18px;vertical-align:top}"
            ".w1,.w2{overflow:hidden;text-align:center}.w1{width:1ch}.w2{width:2ch}"
            "</style><pre>%s</pre>" % (html.escape(title), fg, bg, "\n".join(out))
        )


def run_session(out, cfg, sess):
    cols, rows = cfg.get("cols", 120), cfg.get("rows", 36)
    fg, bg = cfg.get("fg", "#d0d0d0"), cfg.get("bg", "#16181c")
    env = dict(os.environ)
    env.update({"TERM": "xterm-256color", "COLORTERM": "truecolor", "COLUMNS": str(cols), "LINES": str(rows)})
    env.update(cfg.get("env", {}))
    pid, fd = pty.fork()
    if pid == 0:
        try:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        except OSError:
            pass
        os.execvpe(sess["argv"][0], sess["argv"], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    scr = Screen(cols, rows)
    dec = __import__("codecs").getincrementaldecoder("utf-8")("replace")
    raw = []

    def pump(seconds):
        end = time.monotonic() + seconds
        while True:
            left = end - time.monotonic()
            if left <= 0:
                return
            r, _, _ = select.select([fd], [], [], min(left, 0.05))
            if r:
                try:
                    data = os.read(fd, 65536)
                except OSError:
                    return
                if not data:
                    return
                raw.append(data)
                scr.feed(dec.decode(data))
                if scr.answers:
                    os.write(fd, scr.answers)
                    scr.answers = b""

    name = sess["name"]
    for step in sess["steps"]:
        kind = step[0]
        if kind == "wait":
            pump(float(step[1]))
        elif kind == "keys":
            for ch in step[1]:
                os.write(fd, ch.encode())
                pump(float(step[2]) if len(step) > 2 else 0.08)
        elif kind == "shot":
            pump(0.3)
            base = os.path.join(out, "%s-%s" % (name, step[1]))
            with open(base + ".html", "w", encoding="utf-8") as f:
                f.write(scr.html("%s %s" % (name, step[1]), fg, bg))
            with open(base + ".txt", "w", encoding="utf-8") as f:
                f.write(scr.text() + "\n")
            print("shot", base, flush=True)
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    pump(0.5)
    os.waitpid(pid, 0)
    with open(os.path.join(out, "%s.raw" % name), "wb") as f:
        f.write(b"".join(raw))


def main():
    out, script = sys.argv[1], sys.argv[2]
    os.makedirs(out, exist_ok=True)
    with open(script, encoding="utf-8") as f:
        cfg = json.load(f)
    only = sys.argv[3:]
    for sess in cfg["sessions"]:
        if only and sess["name"] not in only:
            continue
        s = dict(cfg)
        s.update(sess.get("cfg", {}))
        run_session(out, s, sess)


if __name__ == "__main__":
    main()
