//! Text layout: from a text layer and loaded fonts to glyph outlines.
//!
//! Shaping (kerning, ligatures) is done by rustybuzz. Line breaking, leading,
//! alignment and fitting are done here, following Photoshop's rules for point
//! text and box text, because the templates were laid out with them.
//!
//! Card symbols need nothing special: a symbol is a few characters of a symbol
//! font, each with its own color, that the font stacks on top of one another.

use crate::doc::{Color, FitMode, Rect, TextAlign, TextLayer, VerticalAlign};
use rustybuzz::ttf_parser::{self, GlyphId};
use std::rc::Rc;

/// One face of a loaded font file.
struct FontFace {
    /// PostScript name, as text layers refer to it.
    name: String,
    /// The name with case, spaces and punctuation removed, for forgiving lookups.
    key: String,
    bytes: Rc<Vec<u8>>,
    index: u32,
}

/// The fonts available for rendering. Fonts are not part of a document: the
/// user supplies them, and a document only names the ones it needs.
#[derive(Default)]
pub struct Fonts {
    faces: Vec<FontFace>,
    /// Changes whenever a font is added, so cached layouts can be told apart.
    version: u64,
}

fn key_of(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The PostScript name stored in a font face.
fn postscript_name(face: &ttf_parser::Face) -> Option<String> {
    face.names().into_iter().filter(|n| n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME).find_map(|n| {
        // Unicode names decode directly; old Mac-encoded ones are plain ASCII in practice.
        n.to_string().or_else(|| n.name.is_ascii().then(|| String::from_utf8_lossy(n.name).into_owned()))
    })
}

impl Fonts {
    pub fn new() -> Fonts {
        Fonts::default()
    }

    /// Load a font file (TTF, OTF or a collection). Returns the PostScript
    /// names it provides, or an error if it is not a usable font.
    pub fn add(&mut self, bytes: Vec<u8>) -> crate::Result<Vec<String>> {
        let bytes = Rc::new(bytes);
        let count = ttf_parser::fonts_in_collection(&bytes).unwrap_or(1);
        let mut names = Vec::new();
        for index in 0..count {
            let Ok(face) = ttf_parser::Face::parse(&bytes, index) else { continue };
            let Some(name) = postscript_name(&face) else { continue };
            // A font loaded twice replaces the earlier copy.
            self.faces.retain(|f| f.name != name);
            self.faces.push(FontFace { key: key_of(&name), name: name.clone(), bytes: bytes.clone(), index });
            names.push(name);
        }
        if names.is_empty() {
            return Err(crate::err!("Not a usable font file"));
        }
        self.version += 1;
        Ok(names)
    }

    /// PostScript names of every loaded face.
    pub fn names(&self) -> Vec<&str> {
        self.faces.iter().map(|f| f.name.as_str()).collect()
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.faces.iter().position(|f| f.name == name).or_else(|| {
            let key = key_of(name);
            self.faces.iter().position(|f| f.key == key)
        })
    }
}

/// The result of laying out one text layer.
pub struct TextLayout {
    /// Glyph outlines in document pixels, one path per color, in paint order.
    pub parts: Vec<(Color, tiny_skia::Path)>,
    /// Bounds of the ink, in document pixels: (left, top, right, bottom).
    pub ink: Option<[f64; 4]>,
    /// Fonts the layer asks for that are not loaded. Their runs are not drawn.
    pub missing: Vec<String>,
    /// The factor the text was shrunk by to fit (1 when it was not).
    pub scale: f64,
    pub lines: usize,
}

/// A shaped glyph, measured at the layer's own sizes (before any fitting).
struct Glyph {
    face: usize,
    id: u16,
    /// Pixels per font unit for this glyph.
    k: f64,
    /// Horizontal stretch.
    sx: f64,
    advance: f64,
    offset: (f64, f64),
    size: f64,
    color: Color,
    /// The glyph is white space: it can end a line and does not count toward its width there.
    space: bool,
    /// Ink bounds in font units (x_min, y_min, x_max, y_max), if the glyph has any.
    bbox: Option<[f64; 4]>,
}

struct Line {
    start: usize,
    end: usize,
    /// Width without trailing white space.
    width: f64,
    /// The largest font size on the line, which sets its leading.
    size: f64,
    /// First line of a paragraph (other than the first paragraph).
    new_paragraph: bool,
}

struct Shaped {
    glyphs: Vec<Glyph>,
    /// Glyph index ranges, one per paragraph.
    paragraphs: Vec<(usize, usize)>,
    missing: Vec<String>,
}

fn shape(layer: &TextLayer, fonts: &Fonts, faces: &[rustybuzz::Face]) -> Shaped {
    let style = &layer.style;
    let scale_x = style.scale_x.unwrap_or(1.0);
    let tracking = style.tracking.unwrap_or(0.0) / 1000.0;
    let mut glyphs = Vec::new();
    let mut paragraphs = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    let mut start = 0;

    for run in &layer.content.runs {
        let font = run.font.as_deref().unwrap_or(&style.font);
        let size = run.size.unwrap_or(style.size);
        let color = run.color.unwrap_or(style.color);
        let tracking = run.tracking.map_or(tracking, |t| t / 1000.0);
        let face_index = fonts.find(font);
        if face_index.is_none() && !missing.iter().any(|m| m == font) {
            missing.push(font.to_string());
        }
        for (i, piece) in run.text.split('\n').enumerate() {
            if i > 0 {
                paragraphs.push((start, glyphs.len()));
                start = glyphs.len();
            }
            let Some(f) = face_index else { continue };
            if piece.is_empty() {
                continue;
            }
            let face = &faces[f];
            let k = size / face.units_per_em() as f64;
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(piece);
            let shaped = rustybuzz::shape(face, &[], buffer);
            for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
                let ch = piece[info.cluster as usize..].chars().next().unwrap_or(' ');
                let id = info.glyph_id as u16;
                let bbox = face.glyph_bounding_box(GlyphId(id)).map(|b| [b.x_min as f64, b.y_min as f64, b.x_max as f64, b.y_max as f64]);
                // A glyph the font places on top of the previous one (as symbol fonts do) gets no tracking.
                let track = if pos.x_advance != 0 { tracking * size } else { 0.0 };
                glyphs.push(Glyph {
                    face: f,
                    id,
                    k,
                    sx: scale_x,
                    advance: pos.x_advance as f64 * k * scale_x + track,
                    offset: (pos.x_offset as f64 * k * scale_x, pos.y_offset as f64 * k),
                    size,
                    color,
                    space: ch.is_whitespace(),
                    bbox,
                });
            }
        }
    }
    paragraphs.push((start, glyphs.len()));
    Shaped { glyphs, paragraphs, missing }
}

/// Break paragraphs into lines no wider than `limit` (None: never wrap).
fn break_lines(shaped: &Shaped, limit: Option<f64>, default_size: f64) -> Vec<Line> {
    let g = &shaped.glyphs;
    let mut lines = Vec::new();
    for (p, &(start, end)) in shaped.paragraphs.iter().enumerate() {
        let mut line_start = start;
        let mut first = true;
        loop {
            // Take glyphs until the line is full, remembering the last place it could break.
            let (mut i, mut width, mut trimmed) = (line_start, 0.0, 0.0);
            let mut last_break: Option<(usize, f64)> = None;
            while i < end {
                let glyph = &g[i];
                if !glyph.space && limit.is_some_and(|l| width + glyph.advance > l + 1e-6) && i > line_start {
                    break;
                }
                width += glyph.advance;
                i += 1;
                if glyph.space {
                    last_break = Some((i, trimmed));
                } else {
                    trimmed = width;
                }
            }
            let (line_end, line_width) = if i >= end {
                (end, trimmed)
            } else {
                // Break after the last space; a word longer than the line is split where it overflows.
                last_break.unwrap_or((i, trimmed))
            };
            let size = g[line_start..line_end].iter().map(|x| x.size).fold(0.0, f64::max);
            lines.push(Line { start: line_start, end: line_end, width: line_width, size: if size > 0.0 { size } else { default_size }, new_paragraph: first && p > 0 });
            first = false;
            line_start = line_end;
            if line_start >= end {
                break;
            }
        }
    }
    lines
}

/// Where every line's baseline starts, at fit scale `s`, before vertical alignment.
struct Placed {
    lines: Vec<Line>,
    /// (x of the line's first glyph, baseline y) per line.
    origins: Vec<(f64, f64)>,
    ink: Option<[f64; 4]>,
}

fn place(layer: &TextLayer, shaped: &Shaped, first_ascent: f64, s: f64) -> Placed {
    let style = &layer.style;
    let b: &Rect = &layer.bounds;
    let wraps = layer.origin.is_none();
    let lines = break_lines(shaped, wraps.then(|| b.w / s), style.size);
    let leading = |line: &Line| line.size * style.leading.unwrap_or(1.2) * s;
    let gap = style.paragraph_spacing.unwrap_or(0.0) * s;

    let mut origins = Vec::with_capacity(lines.len());
    let mut y = match layer.origin {
        Some(o) => o.y,
        // Photoshop puts the first baseline of box text one ascender below the top of the box.
        None => b.y + first_ascent * lines.first().map_or(style.size, |l| l.size) * s,
    };
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            y += leading(line) + if line.new_paragraph { gap } else { 0.0 };
        }
        let width = line.width * s;
        let x = match (layer.origin, style.align) {
            (Some(o), TextAlign::Left) => o.x,
            (Some(o), TextAlign::Center) => o.x - width / 2.0,
            (Some(o), TextAlign::Right) => o.x - width,
            (None, TextAlign::Left) => b.x,
            (None, TextAlign::Center) => b.x + (b.w - width) / 2.0,
            (None, TextAlign::Right) => b.x + b.w - width,
        };
        // Photoshop sets every baseline on a whole pixel.
        origins.push((x, y.round()));
    }

    let mut ink: Option<[f64; 4]> = None;
    for (line, &(x0, y0)) in lines.iter().zip(&origins) {
        let mut pen = x0;
        for glyph in &shaped.glyphs[line.start..line.end] {
            if let Some(bb) = glyph.bbox {
                let k = glyph.k * s;
                let gx = pen + glyph.offset.0 * s;
                let gy = y0 - glyph.offset.1 * s;
                let rect = [gx + bb[0] * k * glyph.sx, gy - bb[3] * k, gx + bb[2] * k * glyph.sx, gy - bb[1] * k];
                ink = Some(match ink {
                    None => rect,
                    Some(a) => [a[0].min(rect[0]), a[1].min(rect[1]), a[2].max(rect[2]), a[3].max(rect[3])],
                });
            }
            pen += glyph.advance * s;
        }
    }
    Placed { lines, origins, ink }
}

/// Feeds a glyph outline into a path, scaled and moved into document pixels.
struct Outline<'a> {
    path: &'a mut tiny_skia::PathBuilder,
    kx: f32,
    ky: f32,
    x: f32,
    y: f32,
}

impl ttf_parser::OutlineBuilder for Outline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to(self.x + x * self.kx, self.y - y * self.ky);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(self.x + x * self.kx, self.y - y * self.ky);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.path.quad_to(self.x + x1 * self.kx, self.y - y1 * self.ky, self.x + x * self.kx, self.y - y * self.ky);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.path.cubic_to(self.x + x1 * self.kx, self.y - y1 * self.ky, self.x + x2 * self.kx, self.y - y2 * self.ky, self.x + x * self.kx, self.y - y * self.ky);
    }
    fn close(&mut self) {
        self.path.close();
    }
}

/// Lay out a text layer: shape it, break and place its lines, fit it to its
/// box if the layer asks for that, and build the glyph outlines.
pub fn layout_text(layer: &TextLayer, fonts: &Fonts) -> TextLayout {
    let faces: Vec<rustybuzz::Face> = fonts.faces.iter().filter_map(|f| rustybuzz::Face::from_slice(&f.bytes, f.index)).collect();
    if faces.len() != fonts.faces.len() {
        // Cannot happen: every stored face parsed when it was added.
        return TextLayout { parts: Vec::new(), ink: None, missing: Vec::new(), scale: 1.0, lines: 0 };
    }
    let shaped = shape(layer, fonts, &faces);
    let b = &layer.bounds;

    // Height of a lowercase "d" in the layer's font, as a share of the font size.
    let first_ascent = fonts
        .find(&layer.style.font)
        .or_else(|| shaped.glyphs.first().map(|g| g.face))
        .and_then(|f| {
            let face = &faces[f];
            let d = face.glyph_index('d')?;
            Some(face.glyph_bounding_box(d)?.y_max as f64 / face.units_per_em() as f64)
        })
        .unwrap_or(0.7);

    let at = |s: f64| place(layer, &shaped, first_ascent, s);
    let height = |p: &Placed| p.ink.map_or(0.0, |i| i[3] - i[1]);
    let width = |p: &Placed| p.ink.map_or(0.0, |i| i[2] - i[0]);

    let mut scale = 1.0;
    let mut placed = at(1.0);
    match layer.fit {
        FitMode::None => {}
        FitMode::ShrinkWidth => {
            let w = width(&placed);
            if b.w > 0.0 && w > b.w {
                scale = b.w / w;
                placed = at(scale);
            }
        }
        FitMode::ShrinkHeight => {
            if b.h > 0.0 && height(&placed) > b.h {
                // The largest scale at which the text is no taller than the box.
                let (mut lo, mut hi) = (0.02, 1.0);
                for _ in 0..24 {
                    let mid = (lo + hi) / 2.0;
                    if height(&at(mid)) > b.h {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                scale = lo;
                placed = at(scale);
            }
        }
    }

    // Box text can be centered vertically in its box.
    let mut shift = 0.0;
    if layer.origin.is_none() && layer.vertical_align == Some(VerticalAlign::Center) {
        if let Some(ink) = placed.ink {
            shift = b.y + (b.h - (ink[3] - ink[1])) / 2.0 - ink[1];
        }
    }

    let mut builders: Vec<(Color, tiny_skia::PathBuilder)> = Vec::new();
    for (line, &(x0, y0)) in placed.lines.iter().zip(&placed.origins) {
        let mut pen = x0;
        for glyph in &shaped.glyphs[line.start..line.end] {
            if glyph.bbox.is_some() {
                let slot = match builders.iter().position(|(c, _)| *c == glyph.color) {
                    Some(i) => i,
                    None => {
                        builders.push((glyph.color, tiny_skia::PathBuilder::new()));
                        builders.len() - 1
                    }
                };
                let k = glyph.k * scale;
                let mut outline = Outline {
                    path: &mut builders[slot].1,
                    kx: (k * glyph.sx) as f32,
                    ky: k as f32,
                    x: (pen + glyph.offset.0 * scale) as f32,
                    y: (y0 + shift - glyph.offset.1 * scale) as f32,
                };
                faces[glyph.face].outline_glyph(GlyphId(glyph.id), &mut outline);
            }
            pen += glyph.advance * scale;
        }
    }
    TextLayout {
        parts: builders.into_iter().filter_map(|(color, path)| Some((color, path.finish()?))).collect(),
        ink: placed.ink.map(|i| [i[0], i[1] + shift, i[2], i[3] + shift]),
        missing: shaped.missing,
        scale,
        lines: placed.lines.len(),
    }
}
