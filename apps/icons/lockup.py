# Draws the wordmark and lockups from Space Mono Bold outlines, so the SVGs
# need no font. Needs fonttools, uharfbuzz and skia-pathops. Run from
# apps/icons with the font beside it:
#   curl -LO https://raw.githubusercontent.com/google/fonts/main/ofl/spacemono/SpaceMono-Bold.ttf
import math, uharfbuzz as hb, pathops
from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen

FONT = 'SpaceMono-Bold.ttf'
NAME = 'telepr0mpt'
ZERO = NAME.index('0')
TRACK = 0.02    # of the em, added between letters
X_HEIGHT = 42   # px in a 128 px lockup
M_OPEN = 60     # units added to each of the m's counters

ICON = '''<defs><linearGradient id="edge" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#3a3d45"/><stop offset="1" stop-color="#1a1c20"/></linearGradient></defs>
  <rect x="8" y="8" width="112" height="112" rx="26" fill="url(#edge)"/>
  <rect x="10" y="10" width="108" height="108" rx="24" fill="#000"/>
  <rect x="40" y="38" width="54" height="9" rx="4.5" fill="#f2f4f7" fill-opacity="0.28"/>
  <path d="M24 55.5 L36 64 L24 72.5 Z" fill="#ffb800"/>
  <rect x="44" y="59.5" width="22" height="9" rx="4.5" fill="#ffb800"/>
  <rect x="70" y="59.5" width="30" height="9" rx="4.5" fill="#f2f4f7"/>
  <rect x="40" y="81" width="44" height="9" rx="4.5" fill="#f2f4f7" fill-opacity="0.55"/>'''


def widen(name, pen):
    """A pen that draws `name`, with the m's two counters opened up.

    Space Mono squeezes its m into one cell, 81 units a counter against
    the n's 200-odd; set large, that knots up "mpt". Stretching only the
    counters keeps the stems' weight."""
    if name != 'm':
        return pen, 0
    def x(v):
        return v + sum(M_OPEN * min(max((v - a) / (b - a), 0), 1) for a, b in ((162, 243), (369, 450)))
    class Warp:
        def transformPoint(self, p):
            return (x(p[0]), p[1])
    return TransformPen(pen, Warp()), 2 * M_OPEN


def outlines():
    """The name's letters and its zero, as paths in font units."""
    f = TTFont(FONT)
    glyphs = f.getGlyphSet()
    font = hb.Font(hb.Face(hb.Blob.from_file_path(FONT)))
    buf = hb.Buffer()
    buf.add_str(NAME)
    buf.guess_segment_properties()
    hb.shape(font, buf, {})
    upm = f['head'].unitsPerEm
    x, rest, zero = 0, pathops.Path(), pathops.Path()
    for info, pos in zip(buf.glyph_infos, buf.glyph_positions):
        into = zero if info.cluster == ZERO else pathops.Path()
        name = f.getGlyphName(info.codepoint)
        pen, extra = widen(name, TransformPen(into.getPen(), (1, 0, 0, 1, x + pos.x_offset, pos.y_offset)))
        glyphs[name].draw(pen)
        if info.cluster != ZERO:
            rest = pathops.op(rest, into, pathops.PathOp.UNION)
        x += pos.x_advance + extra + TRACK * upm
    return rest, zero, X_HEIGHT / f['OS/2'].sxHeight


def svgd(path, scale, ox, oy):
    pen = SVGPathPen(None, ntos=lambda v: ('%.2f' % v).rstrip('0').rstrip('.'))
    path.draw(TransformPen(pen, (scale, 0, 0, -scale, ox, oy)))
    return pen.getCommands()


def lockup(ink, zero_fill, icon=True):
    rest, zero, s = outlines()
    x0, y0, x1, y1 = pathops.op(rest, zero, pathops.PathOp.UNION).bounds
    left = 146 if icon else 0
    base = 64 + y1 * s / 2 if icon else y1 * s
    ox = left - x0 * s
    w = math.ceil(ox + x1 * s + (8 if icon else 0))
    h = 128 if icon else math.ceil(base - y0 * s)
    body = ([ICON] if icon else []) + [
        f'<path fill="{ink}" d="{svgd(rest, s, ox, base)}"/>',
        f'<path fill="{zero_fill}" d="{svgd(zero, s, ox, base)}"/>',
    ]
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">\n  '
            + '\n  '.join(body) + '\n</svg>\n')


if __name__ == '__main__':
    open('teleprompt-lockup-dark.svg', 'w').write(lockup('#f2f4f7', '#ffb800'))
    open('teleprompt-lockup-light.svg', 'w').write(lockup('#111318', '#e5a000'))
    open('teleprompt-wordmark.svg', 'w').write(lockup('#111318', '#e5a000', icon=False))
