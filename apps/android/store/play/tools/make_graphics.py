#!/usr/bin/env python3
"""Google Play store graphics: 512x512 icon and 1024x500 feature graphic.

A terminal window with the prompt ">DSPi_". Rendered 4x and downscaled for
smooth edges. Font: Noto Sans Mono (SIL OFL).
Usage: make_graphics.py <outdir> [app/src/main/res]  (the second argument also writes
the adaptive launcher icon layers into mipmap-*/)
       make_graphics.py --icons <dir>  (job ICON1: the icon of every client, see icons())
Run it in the icon container: assets/icon/render.sh.
"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

SS = 4  # supersampling
FONT_DIR = Path("/usr/share/fonts/google-noto")
BOLD = str(FONT_DIR / "NotoSansMono-Bold.ttf")
REGULAR = str(FONT_DIR / "NotoSansMono-Regular.ttf")

BG = (24, 27, 32)            # behind the window (full bleed)
WINDOW = (10, 12, 15)        # console body
TITLEBAR = (44, 48, 56)
DOTS = [(237, 106, 94), (245, 191, 79), (98, 197, 84)]
PHOSPHOR = (74, 222, 128)    # prompt text
PROMPT = (62, 196, 160)      # ">" (the app's teal accent)
DIM = (120, 132, 140)


def s(v, k=SS):
    """Design px to canvas px; k is the canvas px per design px (SS unless noted)."""
    return int(round(v * k))


def console(d, box, radius, bar_h, dot_r, k=SS):
    x0, y0, x1, y1 = box
    d.rounded_rectangle([s(x0, k), s(y0, k), s(x1, k), s(y1, k)], s(radius, k), fill=WINDOW)
    # title bar: top corners rounded, bottom square
    d.rounded_rectangle([s(x0, k), s(y0, k), s(x1, k), s(y0 + bar_h, k)], s(radius, k), fill=TITLEBAR)
    d.rectangle([s(x0, k), s(y0 + bar_h - radius, k), s(x1, k), s(y0 + bar_h, k)], fill=TITLEBAR)
    cy = y0 + bar_h / 2
    for i, c in enumerate(DOTS):
        cx = x0 + bar_h * 0.55 + i * dot_r * 3.2
        d.ellipse([s(cx - dot_r, k), s(cy - dot_r, k), s(cx + dot_r, k), s(cy + dot_r, k)], fill=c)


def prompt(d, x, baseline, size, cursor_on=True, k=SS):
    """Draw ">DSPi" + a block underscore cursor; returns the right edge."""
    f = ImageFont.truetype(BOLD, s(size, k))
    adv = f.getlength("M") / k
    d.text((s(x, k), s(baseline, k)), ">", font=f, fill=PROMPT, anchor="ls")
    d.text((s(x + adv, k), s(baseline, k)), "DSPi", font=f, fill=PHOSPHOR, anchor="ls")
    cx = x + adv * 5
    if cursor_on:
        h = size * 0.12
        d.rectangle([s(cx + adv * 0.06, k), s(baseline - h, k), s(cx + adv * 0.94, k), s(baseline, k)], fill=PHOSPHOR)
    return cx + adv


def prompt_width(size, k=SS):
    return ImageFont.truetype(BOLD, s(size, k)).getlength("M") / k * 6


def draw_icon(d, k=SS):
    """The Play icon's drawing (512 design px, full bleed) on a canvas of 512 * k px."""
    W = 512
    # Play masks the icon to a rounded square; keep the window well inside it.
    m = 52
    bar = 62
    console(d, (m, m + 18, W - m, W - m - 18), 34, bar, 11, k)
    size = 96
    tw = prompt_width(size, k)
    body_top, body_bot = m + 18 + bar, W - m - 18
    baseline = (body_top + body_bot) / 2 + size * 0.36
    prompt(d, (W - tw) / 2, baseline, size, k=k)


def icon(out):
    W = 512
    im = Image.new("RGB", (s(W), s(W)), BG)
    draw_icon(ImageDraw.Draw(im))
    im.resize((W, W), Image.LANCZOS).save(out / "icon-512.png", optimize=True)


def feature(out):
    W, H = 1024, 500
    im = Image.new("RGB", (s(W), s(H)), BG)
    d = ImageDraw.Draw(im)
    box = (112, 70, W - 112, H - 70)
    bar = 48
    console(d, box, 26, bar, 9)
    size = 120
    x = box[0] + 64
    baseline = box[1] + bar + 60 + size * 0.75
    prompt(d, x, baseline, size)
    small = ImageFont.truetype(REGULAR, s(26))
    lines = ["EQ · crossovers · matrix mixer · crossfeed",
             "live meters · spectrum analyser · presets"]
    y = baseline + 62
    for ln in lines:
        d.text((s(x + 4), s(y)), ln, font=small, fill=DIM, anchor="ls")
        y += 40
    im.resize((W, H), Image.LANCZOS).save(out / "feature-1024x500.png", optimize=True)


LAUNCHER_DENSITIES = {"mdpi": 108, "hdpi": 162, "xhdpi": 216, "xxhdpi": 324, "xxxhdpi": 432}


def launcher(res):
    """Adaptive launcher icon layers (108 dp; drawn at xxxhdpi, 4 px/dp).

    The background is the colour resource ic_launcher_background (BG). The
    window stays inside the 66 dp safe circle that every launcher mask keeps.
    The monochrome layer (Android 13 themed icons) is a window outline plus
    the prompt, in white on transparent.
    """
    W = 432
    box = (112, 140, 320, 292)  # 52 x 38 dp, corners within the 33 dp safe radius
    bar, size = 36, 48
    tw = prompt_width(size)
    baseline = (box[1] + bar + box[3]) / 2 + size * 0.36

    fg = Image.new("RGBA", (s(W), s(W)), (0, 0, 0, 0))
    d = ImageDraw.Draw(fg)
    console(d, box, 20, bar, 6.4)
    prompt(d, (W - tw) / 2, baseline, size)

    mono = Image.new("RGBA", (s(W), s(W)), (0, 0, 0, 0))
    m = ImageDraw.Draw(mono)
    white = (255, 255, 255, 255)
    x0, y0, x1, y1 = box
    m.rounded_rectangle([s(x0), s(y0), s(x1), s(y1)], s(20), outline=white, width=s(10))
    m.rectangle([s(x0), s(y0 + bar - 5), s(x1), s(y0 + bar + 5)], fill=white)
    f = ImageFont.truetype(BOLD, s(size))
    adv = f.getlength("M") / SS
    m.text((s((W - tw) / 2), s(baseline)), ">DSPi", font=f, fill=white, anchor="ls")
    cx = (W - tw) / 2 + adv * 5
    m.rectangle([s(cx + adv * 0.06), s(baseline - size * 0.12), s(cx + adv * 0.94), s(baseline)], fill=white)

    for dpi, px in LAUNCHER_DENSITIES.items():
        d_ = res / f"mipmap-{dpi}"
        d_.mkdir(parents=True, exist_ok=True)
        fg.resize((px, px), Image.LANCZOS).save(d_ / "ic_launcher_foreground.png", optimize=True)
        mono.resize((px, px), Image.LANCZOS).save(d_ / "ic_launcher_monochrome.png", optimize=True)


# -- job ICON1 -- The icon of every client: the Play icon's drawing, unchanged, in two
# shapes. square: full bleed, as Play gets it before its mask (iOS, which masks it
# itself, and the web's apple-touch-icon). rounded: the desktop's, the macOS app icon
# grid: an 824 px rounded square, corner radius 185.4 px, centred on a transparent
# 1024 px canvas (Windows, Linux and the web's favicon take the same).
ICON_PX = 1024
TILE, TILE_RADIUS = 824, 185.4


class SvgDraw:
    """Draws on a PIL canvas and records the same calls as SVG, text as glyph outlines."""

    def __init__(self, im):
        self.d = ImageDraw.Draw(im)
        self.parts = []

    @staticmethod
    def _rgb(c):
        return "#%02x%02x%02x" % c

    # PIL's boxes include their last pixel row and column
    def rectangle(self, xy, fill):
        self.d.rectangle(xy, fill=fill)
        x0, y0, x1, y1 = xy
        self.parts.append(f'<rect x="{x0}" y="{y0}" width="{x1 - x0 + 1}" height="{y1 - y0 + 1}" fill="{self._rgb(fill)}"/>')

    def rounded_rectangle(self, xy, r, fill):
        self.d.rounded_rectangle(xy, r, fill=fill)
        x0, y0, x1, y1 = xy
        self.parts.append(f'<rect x="{x0}" y="{y0}" width="{x1 - x0 + 1}" height="{y1 - y0 + 1}" '
                          f'rx="{r}" fill="{self._rgb(fill)}"/>')

    def ellipse(self, xy, fill):
        self.d.ellipse(xy, fill=fill)
        x0, y0, x1, y1 = xy
        self.parts.append(f'<ellipse cx="{(x0 + x1 + 1) / 2:g}" cy="{(y0 + y1 + 1) / 2:g}" '
                          f'rx="{(x1 - x0 + 1) / 2:g}" ry="{(y1 - y0 + 1) / 2:g}" fill="{self._rgb(fill)}"/>')

    def text(self, xy, text, font, fill, anchor):
        assert anchor == "ls"
        self.d.text(xy, text, font=font, fill=fill, anchor=anchor)
        from fontTools.pens.svgPathPen import SVGPathPen
        from fontTools.ttLib import TTFont
        tt = TTFont(font.path)
        glyphs, cmap = tt.getGlyphSet(), tt.getBestCmap()
        scale = font.size / tt["head"].unitsPerEm
        x, y = xy
        for ch in text:
            name = cmap[ord(ch)]
            pen = SVGPathPen(glyphs)
            glyphs[name].draw(pen)
            self.parts.append(f'<path transform="translate({x:g} {y:g}) scale({scale:g} {-scale:g})" '
                              f'd="{pen.getCommands()}" fill="{self._rgb(fill)}"/>')
            x += glyphs[name].width * scale


def icons(out):
    """dspi-icon-{square,rounded}-1024.png and .svg; assets/icon/render.sh makes the
    clients' files from them."""
    out.mkdir(parents=True, exist_ok=True)
    W = 512
    head = "<!-- The DSPi Console icon (job ICON1), drawn by apps/android/store/play/tools/make_graphics.py; " \
           "glyphs: Noto Sans Mono Bold (SIL OFL 1.1). Do not edit: rerun assets/icon/render.sh. -->"
    # the drawing in SVG, in the 4x canvas units icon-512.png is drawn in
    rec = SvgDraw(Image.new("RGB", (s(W), s(W)), BG))
    draw_icon(rec)
    body = f'<rect width="{s(W)}" height="{s(W)}" fill="{SvgDraw._rgb(BG)}"/>' + "".join(rec.parts)

    k = SS * ICON_PX / W
    im = Image.new("RGB", (s(W, k), s(W, k)), BG)
    draw_icon(ImageDraw.Draw(im), k)
    im.resize((ICON_PX, ICON_PX), Image.LANCZOS).save(out / "dspi-icon-square-1024.png", optimize=True)
    (out / "dspi-icon-square.svg").write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{ICON_PX}" height="{ICON_PX}" viewBox="0 0 {s(W)} {s(W)}">\n'
        f'{head}\n{body}\n</svg>\n')

    k = SS * TILE / W
    tile = Image.new("RGB", (s(W, k), s(W, k)), BG)
    draw_icon(ImageDraw.Draw(tile), k)
    mask = Image.new("L", tile.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, tile.width - 1, tile.height - 1], s(TILE_RADIUS), fill=255)
    big = Image.new("RGBA", (ICON_PX * SS, ICON_PX * SS), BG + (0,))
    margin = (ICON_PX - TILE) // 2
    big.paste(tile, (margin * SS, margin * SS), mask)
    # resample with premultiplied alpha, so the transparent margin bleeds no colour
    big.convert("RGBa").resize((ICON_PX, ICON_PX), Image.LANCZOS).convert("RGBA") \
        .save(out / "dspi-icon-rounded-1024.png", optimize=True)
    (out / "dspi-icon-rounded.svg").write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{ICON_PX}" height="{ICON_PX}" viewBox="0 0 {ICON_PX} {ICON_PX}">\n'
        f'{head}\n<clipPath id="tile"><rect x="{margin}" y="{margin}" width="{TILE}" height="{TILE}" rx="{TILE_RADIUS:g}"/>'
        f'</clipPath>\n<g clip-path="url(#tile)"><g transform="translate({margin} {margin}) scale({TILE / s(W):g})">'
        f'{body}</g></g>\n</svg>\n')


if __name__ == "__main__":
    if sys.argv[1:2] == ["--icons"]:
        icons(Path(sys.argv[2]))
        sys.exit(0)
    out = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    out.mkdir(parents=True, exist_ok=True)
    icon(out)
    feature(out)
    if len(sys.argv) > 2:  # app/src/main/res: the launcher icon layers
        launcher(Path(sys.argv[2]))
