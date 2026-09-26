# Draws the wordmark and lockups from Atkinson Hyperlegible Next outlines.
# Needs fonttools, uharfbuzz and skia-pathops; run from apps/icons with a
# static bold instance: fonttools varLib.instancer ../fonts/AtkinsonHyperlegibleNext[wght].ttf wght=700 -o atk700.ttf

import math, sys, uharfbuzz as hb, pathops
from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen

FONT = sys.argv[1] if len(sys.argv) > 1 else 'atk700.ttf'
BAND = float(sys.argv[2]) if len(sys.argv) > 2 else 84
TRACK = float(sys.argv[3]) if len(sys.argv) > 3 else -12
f = TTFont(FONT); gs = f.getGlyphSet()
blob = hb.Blob.from_file_path(FONT); face = hb.Face(blob); font = hb.Font(face)

def glyph_path(name, dx=0):
    p = pathops.Path(); pen = TransformPen(p.getPen(), (1,0,0,1,dx,0)); gs[name].draw(pen); return p

def slashed_o():
    # The o's ring and counter apart, so the counter can be cut by a band at
    # the angle of Atkinson's own zero: the edges (202,443)->(419,132).
    rec = pathops.Path(); gs['o'].draw(rec.getPen())
    contours = list(rec.contours)
    outer, counter = contours[0], contours[1]
    d = (217, -311); L = math.hypot(*d); u = (d[0]/L, d[1]/L); n = (-u[1], u[0])
    cx, cy = 288, 248; h = BAND/2; ext = 600
    band = pathops.Path(); pen = band.getPen()
    pts = [(cx+u[0]*s+n[0]*t, cy+u[1]*s+n[1]*t) for s,t in [(-ext,-h),(ext,-h),(ext,h),(-ext,h)]]
    pen.moveTo(pts[0]); [pen.lineTo(q) for q in pts[1:]]; pen.closePath()
    counter_fill = pathops.Path(); counter.draw(counter_fill.getPen())
    # counter contour is wound as a hole; make it a filled shape
    counter_fill = pathops.op(counter_fill, pathops.Path(), pathops.PathOp.UNION, fix_winding=True)
    holes = pathops.op(counter_fill, band, pathops.PathOp.DIFFERENCE)
    ring = pathops.Path(); outer.draw(ring.getPen())
    ring = pathops.op(ring, pathops.Path(), pathops.PathOp.UNION, fix_winding=True)
    return pathops.op(ring, holes, pathops.PathOp.DIFFERENCE)

def svgd(path, scale, ox, oy):
    sp = SVGPathPen(None, ntos=lambda v: ('%.2f' % v).rstrip('0').rstrip('.'))
    path.draw(TransformPen(sp, (scale,0,0,-scale,ox,oy)))
    return sp.getCommands()

def shape(text, variant):
    buf = hb.Buffer(); buf.add_str(text); buf.guess_segment_properties()
    hb.shape(font, buf, {})
    x = 0; rest = pathops.Path(); zero = None
    for i,(info,pos) in enumerate(zip(buf.glyph_infos, buf.glyph_positions)):
        name = f.getGlyphName(info.codepoint)
        if text[info.cluster] == 'o':
            if variant == 'A':
                adv = f['hmtx']['zero'][0]
                z = pathops.Path(); gs['zero'].draw(TransformPen(z.getPen(), (1,0,0,1,x,0))); zero = z
                x += adv + TRACK; continue
            z = slashed_o(); zz = pathops.Path(); z.draw(TransformPen(zz.getPen(), (1,0,0,1,x+pos.x_offset,0))); zero = zz
        else:
            g = pathops.Path(); gs[name].draw(TransformPen(g.getPen(), (1,0,0,1,x+pos.x_offset,pos.y_offset)))
            rest = pathops.op(rest, g, pathops.PathOp.UNION)
        x += pos.x_advance + TRACK
    return rest, zero, x - TRACK

ICON = '''<defs><linearGradient id="edge" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#3a3d45"/><stop offset="1" stop-color="#1a1c20"/></linearGradient></defs>
  <rect x="8" y="8" width="112" height="112" rx="26" fill="url(#edge)"/>
  <rect x="10" y="10" width="108" height="108" rx="24" fill="#000"/>
  <rect x="40" y="38" width="54" height="9" rx="4.5" fill="#f2f4f7" fill-opacity="0.28"/>
  <path d="M24 55.5 L36 64 L24 72.5 Z" fill="#ffb800"/>
  <rect x="44" y="59.5" width="22" height="9" rx="4.5" fill="#ffb800"/>
  <rect x="70" y="59.5" width="30" height="9" rx="4.5" fill="#f2f4f7"/>
  <rect x="40" y="81" width="44" height="9" rx="4.5" fill="#f2f4f7" fill-opacity="0.55"/>'''

def lockup(variant, ink, zero_fill, bg=None, s=0.084, icon=True):
    rest, zero, width = shape('teleprompt', variant)
    x0, y0, x1, y1 = pathops.op(rest, zero, pathops.PathOp.UNION).bounds
    asc = y1  # top of t/l
    left = 8 + 112 + 26 if icon else 0
    base = 64 + asc*s/2 if icon else asc*s
    ox = left - x0*s
    W = math.ceil(ox + x1*s + (8 if icon else 0)); H = 128 if icon else math.ceil(base - y0*s)
    body = []
    if bg: body.append(f'<rect width="{W}" height="{H}" fill="{bg}"/>')
    if icon: body.append(ICON)
    body.append(f'<path fill="{ink}" d="{svgd(rest, s, ox, base)}"/>')
    body.append(f'<path fill="{zero_fill}" d="{svgd(zero, s, ox, base)}"/>')
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">\n  ' + '\n  '.join(body) + '\n</svg>\n'

if __name__ == '__main__':
    open('teleprompt-lockup-dark.svg', 'w').write(lockup('A', '#f2f4f7', '#ffb800'))
    open('teleprompt-lockup-light.svg', 'w').write(lockup('A', '#111318', '#e5a000'))
    open('teleprompt-wordmark.svg', 'w').write(lockup('A', '#111318', '#e5a000', icon=False))
