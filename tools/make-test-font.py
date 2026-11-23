"""Builds crates/core/tests/fixtures/blocks.ttf: a tiny font of rectangles with
round metrics, so the text tests can check layout with exact numbers and need
no third-party font. Our own work; run with `python3 tools/make-test-font.py`
(needs fontTools).

1000 units per em. Every glyph is a rectangle standing on the baseline:
  a  400 wide, 500 tall, advance 500
  b  800 wide, 500 tall, advance 1000
  d  400 wide, 700 tall, advance 500   (the tall letter that sets the first baseline of box text)
  space  advance 250
Rectangles start 50 units right of the pen. There is no kerning.
"""
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

def rect(w, h):
    pen = TTGlyphPen(None)
    if w:
        pen.moveTo((50, 0)); pen.lineTo((50, h)); pen.lineTo((50 + w, h)); pen.lineTo((50 + w, 0)); pen.closePath()
    return pen.glyph()

glyphs = {".notdef": (0, 0, 500), "space": (0, 0, 250), "a": (400, 500, 500), "b": (800, 500, 1000), "d": (400, 700, 500)}
fb = FontBuilder(1000, isTTF=True)
fb.setupGlyphOrder(list(glyphs))
fb.setupCharacterMap({32: "space", 97: "a", 98: "b", 100: "d"})
fb.setupGlyf({name: rect(w, h) for name, (w, h, _) in glyphs.items()})
fb.setupHorizontalMetrics({name: (adv, 50 if w else 0) for name, (w, _, adv) in glyphs.items()})
fb.setupHorizontalHeader(ascent=800, descent=-200)
fb.setupNameTable({"familyName": "Blocks", "styleName": "Regular", "psName": "Blocks-Regular"})
fb.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
fb.setupPost()
fb.save("crates/core/tests/fixtures/blocks.ttf")
