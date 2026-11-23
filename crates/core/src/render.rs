//! The renderer: `(document, assets) -> pixels` (rule 1), with no platform
//! dependencies (rule 2).
//!
//! Compositing model (Photoshop's, for the features in docs/SURVEY.md):
//!   - A group's children paint bottom to top (`children[0]` first).
//!   - A pass-through group paints its children straight onto what is below
//!     it. Any other layer is built on its own first.
//!   - Clipped layers blend into their base and never change its alpha.
//!   - Masks cut the layer before effects see it.
//!   - Effects blend with the backdrop on their own: shadows and glows under
//!     the layer, overlays, inner shadow, bevel and stroke over it. Fill
//!     opacity fades the layer's pixels only. Opacity fades the lot.

use crate::doc::{BlendMode, Color, Doc, Effect, Fill, FillRule, Layer, Outside, Path, ShapeLayer, StrokeAlign};
use crate::effects::{effect_extent, effect_reach, ordered, EffectContext, Shape, Stamp};
use crate::field::blur;
use crate::raster::{composite, cross_fade, multiply_alpha, place_image, resample, stamp, Image, Raster, Region};
use crate::serialize::AssetBytes;
use std::collections::HashMap;
use std::rc::Rc;

pub struct RenderResult {
    pub width: u32,
    pub height: u32,
    /// Straight-alpha RGBA, 8 bits per channel, row-major.
    pub pixels: Vec<u8>,
    /// Things the document asked for that this build does not draw.
    pub warnings: Vec<String>,
}

/// Renders documents, keeping decoded images and parsed paths between calls.
///
/// One renderer serves one document session. Painted layers live here as
/// decoded images (see [`Renderer::image_mut`]).
#[derive(Default)]
pub struct Renderer {
    images: HashMap<String, Option<Rc<Image>>>,
    paths: HashMap<String, Option<Rc<tiny_skia::Path>>>,
    /// Keyed by what is being filled: see `RenderPass::fill_key` and `stroke_key`.
    coverage: HashMap<String, Coverage>,
}

struct RenderPass<'a> {
    assets: &'a AssetBytes,
    cache: &'a mut Renderer,
    /// The whole document. Layers are cut off at its edge.
    doc: Region,
    warnings: Vec<String>,
}

/// Decode PNG or JPEG bytes to straight RGBA.
pub fn decode_image(bytes: &[u8]) -> Option<Image> {
    let rgba = image::load_from_memory(bytes).ok()?.into_rgba8();
    Some(Image { w: rgba.width() as i32, h: rgba.height() as i32, data: rgba.into_raw() })
}

/// Encode straight-alpha RGBA as PNG.
pub fn encode_png(width: u32, height: u32, pixels: &[u8]) -> crate::Result<Vec<u8>> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(pixels, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|e| crate::err!("PNG encode failed: {e}"))?;
    Ok(out)
}

/// Parse an SVG path string. `Err` means it is malformed; `Ok(None)` means it is valid but draws nothing.
fn build_path(source: &Path) -> Result<Option<tiny_skia::Path>, ()> {
    use kurbo::PathEl;
    let bez = kurbo::BezPath::from_svg(&source.d).map_err(|_| ())?;
    let mut pb = tiny_skia::PathBuilder::new();
    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => pb.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => pb.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(a, p) => pb.quad_to(a.x as f32, a.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(a, b, p) => pb.cubic_to(a.x as f32, a.y as f32, b.x as f32, b.y as f32, p.x as f32, p.y as f32),
            PathEl::ClosePath => pb.close(),
        }
    }
    Ok(pb.finish())
}

fn fill_rule(path: &Path) -> tiny_skia::FillRule {
    match path.fill_rule {
        Some(FillRule::Evenodd) => tiny_skia::FillRule::EvenOdd,
        _ => tiny_skia::FillRule::Winding,
    }
}

/// Side of a coverage tile in pixels.
const TILE: i32 = 256;

enum Tile {
    Empty,
    Full,
    Mixed(Box<[u8]>),
}

/// Anti-aliased coverage of one path, rasterized in fixed tiles on the document grid.
///
/// A tile is always rasterized the same way whatever region asked for it, so a
/// partial render is bit-identical to the same part of a full render. Tiles
/// that are entirely inside or outside the path cost no memory.
struct Coverage {
    path: Rc<tiny_skia::Path>,
    rule: tiny_skia::FillRule,
    tiles: HashMap<(i32, i32), Tile>,
}

impl Coverage {
    fn new(path: Rc<tiny_skia::Path>, rule: tiny_skia::FillRule) -> Coverage {
        Coverage { path, rule, tiles: HashMap::new() }
    }

    fn tile(&mut self, tx: i32, ty: i32) -> &Tile {
        let (path, rule) = (&self.path, self.rule);
        self.tiles.entry((tx, ty)).or_insert_with(|| {
            let b = path.bounds();
            let (x, y) = ((tx * TILE) as f32, (ty * TILE) as f32);
            if b.right() <= x || b.bottom() <= y || b.left() >= x + TILE as f32 || b.top() >= y + TILE as f32 {
                return Tile::Empty;
            }
            let Some(mut mask) = tiny_skia::Mask::new(TILE as u32, TILE as u32) else { return Tile::Empty };
            mask.fill_path(path, rule, true, tiny_skia::Transform::from_translate(-x, -y));
            let data = mask.data();
            if data.iter().all(|&v| v == 0) {
                Tile::Empty
            } else if data.iter().all(|&v| v == 255) {
                Tile::Full
            } else {
                Tile::Mixed(data.into())
            }
        })
    }

    /// Coverage (0..255) over `region`, row-major.
    fn over(&mut self, region: &Region) -> Vec<u8> {
        let mut out = vec![0u8; region.area()];
        if region.area() == 0 {
            return out;
        }
        let width = region.w as usize;
        for ty in region.y.div_euclid(TILE)..=(region.y + region.h - 1).div_euclid(TILE) {
            for tx in region.x.div_euclid(TILE)..=(region.x + region.w - 1).div_euclid(TILE) {
                let bounds = Region::new(tx * TILE, ty * TILE, TILE, TILE);
                let Some(area) = region.intersect(&bounds) else { continue };
                match self.tile(tx, ty) {
                    Tile::Empty => {}
                    Tile::Full => {
                        for y in area.y..area.y + area.h {
                            let start = region.index(area.x, y);
                            out[start..start + area.w as usize].fill(255);
                        }
                    }
                    Tile::Mixed(data) => {
                        for y in area.y..area.y + area.h {
                            let from = bounds.index(area.x, y);
                            let to = (y - region.y) as usize * width + (area.x - region.x) as usize;
                            out[to..to + area.w as usize].copy_from_slice(&data[from..from + area.w as usize]);
                        }
                    }
                }
            }
        }
        out
    }
}

impl<'a> RenderPass<'a> {
    fn warn(&mut self, message: String) {
        if !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }

    fn image(&mut self, id: &str, used_by: &str) -> Option<Rc<Image>> {
        if !self.cache.images.contains_key(id) {
            let bytes = self.assets.get(id);
            let decoded = bytes.and_then(|b| decode_image(b)).map(Rc::new);
            self.cache.images.insert(id.to_string(), decoded);
        }
        let image = self.cache.images[id].clone();
        if image.is_none() {
            self.warn(if self.assets.contains_key(id) { format!("Asset \"{id}\" is not a readable image.") } else { format!("Asset \"{id}\" has no data.") });
            self.warn(format!("Layer \"{used_by}\" was skipped: its image is unavailable."));
        }
        image
    }

    fn path(&mut self, source: &Path) -> Option<Rc<tiny_skia::Path>> {
        if let Some(cached) = self.cache.paths.get(&source.d) {
            return cached.clone();
        }
        match build_path(source) {
            Ok(path) => {
                let path = path.map(Rc::new);
                self.cache.paths.insert(source.d.clone(), path.clone());
                path
            }
            // Not cached, so the warning is reported on every render.
            Err(()) => {
                let head: String = source.d.chars().take(40).collect();
                self.warn(format!("A vector path could not be parsed: \"{head}\""));
                None
            }
        }
    }

    /// Coverage of a path's fill over `region`. `None` if the path is unusable or empty.
    fn fill_coverage(&mut self, source: &Path, region: &Region) -> Option<Vec<u8>> {
        let key = format!("{}|{}", if source.fill_rule == Some(FillRule::Evenodd) { "eo" } else { "nz" }, source.d);
        if !self.cache.coverage.contains_key(&key) {
            let path = self.path(source)?;
            self.cache.coverage.insert(key.clone(), Coverage::new(path, fill_rule(source)));
        }
        Some(self.cache.coverage.get_mut(&key)?.over(region))
    }

    /// Coverage of a path stroked `width` pixels wide (centered on the path) over `region`.
    fn stroke_coverage(&mut self, source: &Path, width: f32, region: &Region) -> Option<Vec<u8>> {
        let key = format!("s{}|{}", width.to_bits(), source.d);
        if !self.cache.coverage.contains_key(&key) {
            let path = self.path(source)?;
            let outline = path.stroke(&tiny_skia::Stroke { width, ..Default::default() }, 1.0)?;
            self.cache.coverage.insert(key.clone(), Coverage::new(Rc::new(outline), tiny_skia::FillRule::Winding));
        }
        Some(self.cache.coverage.get_mut(&key)?.over(region))
    }

    // ---- tree ---------------------------------------------------------------

    fn draw_children(&mut self, target: &mut Raster, layers: &[Layer]) {
        let mut i = 0;
        while i < layers.len() {
            let base = &layers[i];
            let mut end = i + 1;
            while end < layers.len() && layers[end].base().clip {
                end += 1;
            }
            // A hidden base hides everything clipped to it.
            if base.base().visible {
                let clipped: Vec<&Layer> = layers[i + 1..end].iter().filter(|l| l.base().visible).collect();
                self.draw_layer(target, base, &clipped, base.base().opacity as f32, base.base().blend);
            }
            i = end;
        }
    }

    /// Draw one layer with the layers clipped to it. Opacity and blend mode are
    /// passed in so a clipped layer can be drawn "on its own" first.
    fn draw_layer(&mut self, target: &mut Raster, layer: &Layer, clipped: &[&Layer], opacity: f32, blend: BlendMode) {
        let base = layer.base();
        let effects = base.active_effects();

        if let Layer::Group(group) = layer {
            if group.pass_through && effects.is_empty() && clipped.is_empty() {
                if opacity >= 1.0 && !base.has_active_mask() {
                    return self.draw_children(target, &group.children);
                }
                if opacity <= 0.0 {
                    return;
                }
                let mut work = target.clone();
                self.draw_children(&mut work, &group.children);
                let mask = self.mask_coverage(layer, &target.region);
                return cross_fade(target, &work, opacity, mask.as_deref());
            }
        }
        if opacity <= 0.0 {
            return;
        }

        // Effects look at the layer's alpha around each pixel, so when only part of the
        // document is being drawn, the layer is built over a wider region. A gradient
        // overlay is positioned by the layer's full bounds, so it needs all of it.
        let content_region = if effects.is_empty() {
            target.region
        } else if effects.iter().any(|e| matches!(e, Effect::GradientOverlay { .. })) {
            self.doc
        } else {
            let margin = effects.iter().map(|e| effect_extent(e)).max().unwrap_or(0);
            let r = target.region;
            Region::new(r.x - margin, r.y - margin, r.w + margin * 2, r.h + margin * 2).intersect(&self.doc).unwrap_or(r)
        };
        let Some(mut content) = self.content(layer, &content_region) else { return };
        if let Some(mask) = self.mask_coverage(layer, &content.region) {
            multiply_alpha(&mut content, &mask);
        }

        // The layer's silhouette is fixed here: clipped layers cannot change alpha.
        // Always keep a margin, so the region contains the layer's edge even for inside-only effects.
        let shape = if effects.is_empty() { None } else { shape_of(&content, effects.iter().map(|e| effect_reach(e)).max().unwrap_or(0).max(2)) };
        if !effects.is_empty() && shape.is_none() {
            return; // nothing visible, so nothing to cast effects
        }

        for child in clipped {
            // Rendered on its own first, so its masks, effects and children are all honoured.
            let mut own = Raster::new(content.region);
            self.draw_layer(&mut own, child, &[], 1.0, BlendMode::Normal);
            composite(&mut content, &own, child.base().opacity as f32, child.base().blend, true);
        }
        let fill = base.fill_opacity.unwrap_or(1.0) as f32;

        let Some(shape) = shape else {
            return composite(target, &content, opacity * fill, blend, false);
        };

        let fx = EffectContext::new(&shape);
        let (below, above) = ordered(&effects);
        let paint = |work: &mut Raster| {
            for effect in &below {
                for s in fx.stamps(effect) {
                    apply_stamp(work, &shape, &s);
                }
            }
            composite(work, &content, fill, blend, false);
            for effect in &above {
                for s in fx.stamps(effect) {
                    apply_stamp(work, &shape, &s);
                }
            }
        };
        if opacity < 1.0 {
            let mut work = target.clone();
            paint(&mut work);
            cross_fade(target, &work, opacity, None);
        } else {
            paint(target);
        }
    }

    /// The layer's own pixels within `region`, before masks. `None` when there are none.
    fn content(&mut self, layer: &Layer, region: &Region) -> Option<Raster> {
        let (asset, transform) = match layer {
            Layer::Group(group) => {
                let mut out = Raster::new(*region);
                self.draw_children(&mut out, &group.children);
                return Some(out);
            }
            Layer::Shape(shape) => return self.shape(shape, region),
            Layer::Text(text) => {
                self.warn(format!("Text layer \"{}\" was skipped: text rendering arrives in Phase 3.", text.base.name));
                return None;
            }
            Layer::Pixel(l) => (&l.asset, &l.transform),
            Layer::Fill(l) => (&l.asset, &l.transform),
            Layer::Art(l) => (l.asset.as_ref()?, &l.transform),
        };
        let image = self.image(asset, &layer.base().name)?;
        let [a, c, tx, b, d, ty] = *transform;
        let unscaled = a == 1.0 && d == 1.0 && b == 0.0 && c == 0.0 && tx.fract() == 0.0 && ty.fract() == 0.0;
        if unscaled {
            place_image(&image, tx as i32, ty as i32, region)
        } else {
            resample(&image, transform, region)
        }
    }

    fn shape(&mut self, layer: &ShapeLayer, region: &Region) -> Option<Raster> {
        let path = self.path(&layer.path)?;
        let stroke = layer.stroke.as_ref().filter(|s| s.width > 0.0);
        let grow = match stroke {
            Some(s) if s.align == StrokeAlign::Center => s.width as f32 / 2.0 + 1.0,
            _ => 1.0,
        };
        let b = path.bounds();
        let (x0, y0) = ((b.left() - grow).floor() as i32, (b.top() - grow).floor() as i32);
        let area = region.intersect(&Region::new(x0, y0, (b.right() + grow).ceil() as i32 - x0, (b.bottom() + grow).ceil() as i32 - y0))?;
        let mut out = Raster::new(area);
        let Fill::Solid { color } = &layer.fill;
        let fill = self.fill_coverage(&layer.path, &area)?;
        ink(&mut out, &fill, color);
        if let Some(stroke) = stroke {
            let inside = stroke.align == StrokeAlign::Inside;
            // An inside stroke is a double-width stroke with the outer half clipped away.
            let width = stroke.width as f32 * if inside { 2.0 } else { 1.0 };
            if let Some(mut edge) = self.stroke_coverage(&layer.path, width, &area) {
                if inside {
                    edge.iter_mut().zip(&fill).for_each(|(e, &f)| *e = ((*e as u32 * f as u32 + 127) / 255) as u8);
                }
                let mut over = Raster::new(area);
                ink(&mut over, &edge, &stroke.color);
                composite(&mut out, &over, 1.0, BlendMode::Normal, false);
            }
        }
        Some(out)
    }

    /// Combined raster and vector mask coverage (0..255) over `region`, or
    /// `None` if the layer has no active mask.
    fn mask_coverage(&mut self, layer: &Layer, region: &Region) -> Option<Vec<u8>> {
        let base = layer.base();
        let mut out: Option<Vec<u8>> = None;

        if let Some(vector) = base.vector_mask.as_ref().filter(|v| v.enabled != Some(false)) {
            let mut cov = self.fill_coverage(&vector.path, region).unwrap_or_else(|| vec![0; region.area()]);
            if vector.inverted == Some(true) {
                cov.iter_mut().for_each(|c| *c = 255 - *c);
            }
            out = Some(cov);
        }

        let Some(mask) = base.mask.as_ref().filter(|m| m.active()) else { return out };
        let Some(image) = self.image(&mask.asset, &base.name) else { return out };
        let density = mask.density.unwrap_or(1.0) as f32;
        let n = region.area();
        let outside = if mask.outside == Some(Outside::Reveal) { 255.0 } else { 0.0 };
        let feather = mask.feather.unwrap_or(0.0) as f32;
        // A feathered mask is blurred, so it is built over a wider region and cropped afterwards.
        let margin = if feather > 0.0 { (feather * 2.0).ceil() as i32 + 2 } else { 0 };
        let wide = Region::new(region.x - margin, region.y - margin, region.w + margin * 2, region.h + margin * 2);
        let mut gray = vec![outside; wide.area()];
        let (ox, oy) = (mask.offset.x.round() as i32, mask.offset.y.round() as i32);
        if let Some(area) = wide.intersect(&Region::new(ox, oy, image.w, image.h)) {
            for y in area.y..area.y + area.h {
                let mut si = ((y - oy) as usize * image.w as usize + (area.x - ox) as usize) * 4;
                let mut di = wide.index(area.x, y);
                for _ in 0..area.w {
                    // Luma, so a mask that is not strictly gray still behaves sensibly.
                    let px = &image.data[si..si + 3];
                    gray[di] = ((54 * px[0] as u32 + 183 * px[1] as u32 + 19 * px[2] as u32) >> 8) as f32;
                    si += 4;
                    di += 1;
                }
            }
        }
        if feather > 0.0 {
            // Blur the difference from the outside value, so the outside extends past the region
            // and the region's own border is not darkened. `sign` keeps the difference non-negative.
            let sign = if outside > 0.0 { -1.0 } else { 1.0 };
            let delta: Vec<f32> = gray.iter().map(|&v| sign * (v - outside) / 255.0).collect();
            let soft = blur(&delta, wide.w as usize, wide.h as usize, feather / 2.0);
            let mut cropped = Vec::with_capacity(n);
            for y in region.y..region.y + region.h {
                let start = wide.index(region.x, y);
                cropped.extend(soft[start..start + region.w as usize].iter().map(|&v| outside + sign * v * 255.0));
            }
            gray = cropped;
        }
        let mut result = out.unwrap_or_else(|| vec![255; n]);
        for (r, &g) in result.iter_mut().zip(&gray) {
            let mut m = g;
            if mask.inverted == Some(true) {
                m = 255.0 - m;
            }
            if density < 1.0 {
                m = 255.0 - density * (255.0 - m);
            }
            *r = (*r as f32 * m / 255.0 + 0.5) as u8;
        }
        Some(result)
    }
}

/// Fill a raster with one color through a 0..255 coverage map.
fn ink(raster: &mut Raster, coverage: &[u8], color: &Color) {
    let rgb = [(color[0] * 255.0 + 0.5) as u8, (color[1] * 255.0 + 0.5) as u8, (color[2] * 255.0 + 0.5) as u8];
    let a = color[3] as f32;
    for (px, &c) in raster.data.chunks_exact_mut(4).zip(coverage) {
        if c != 0 {
            px[..3].copy_from_slice(&rgb);
            px[3] = (c as f32 * a + 0.5) as u8;
        }
    }
}

fn apply_stamp(target: &mut Raster, shape: &Shape, s: &Stamp) {
    stamp(target, &shape.region, &s.coverage, &s.ink, s.opacity, s.blend);
}

/// A layer's alpha, cropped to its visible pixels plus `reach` on every side.
fn shape_of(content: &Raster, reach: i32) -> Option<Shape> {
    let (w, h) = (content.region.w as usize, content.region.h as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    let mut any = false;
    for y in 0..h {
        let row = &content.data[y * w * 4..(y + 1) * w * 4];
        let mut first = None;
        let mut last = 0;
        for (x, px) in row.chunks_exact(4).enumerate() {
            if px[3] != 0 {
                first.get_or_insert(x);
                last = x;
            }
        }
        if let Some(first) = first {
            any = true;
            x0 = x0.min(first);
            x1 = x1.max(last);
            y0 = y0.min(y);
            y1 = y;
        }
    }
    if !any {
        return None;
    }
    // The region may reach past the document edge, where alpha is zero. So a
    // layer that touches the edge still has an edge there, as in Photoshop.
    let reach_u = reach as usize;
    let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
    let (rw, rh) = (bw + reach_u * 2, bh + reach_u * 2);
    let mut alpha = vec![0f32; rw * rh];
    for y in 0..bh {
        let src = ((y0 + y) * w + x0) * 4 + 3;
        let dst = (y + reach_u) * rw + reach_u;
        for i in 0..bw {
            alpha[dst + i] = content.data[src + i * 4] as f32 / 255.0;
        }
    }
    let bounds = Region::new(content.region.x + x0 as i32, content.region.y + y0 as i32, bw as i32, bh as i32);
    Some(Shape { region: Region::new(bounds.x - reach, bounds.y - reach, rw as i32, rh as i32), alpha, bounds })
}

impl Renderer {
    pub fn new() -> Renderer {
        Renderer::default()
    }

    /// Render the part of the document inside `region`. The result is exactly
    /// what a full render would show there.
    pub fn render_region(&mut self, doc: &Doc, assets: &AssetBytes, region: Region) -> (Raster, Vec<String>) {
        let doc_region = Region::new(0, 0, doc.width as i32, doc.height as i32);
        let mut target = Raster::new(region.intersect(&doc_region).unwrap_or(Region::new(region.x, region.y, 0, 0)));
        let mut pass = RenderPass { assets, cache: self, doc: doc_region, warnings: Vec::new() };
        // The root is always a plain container, whatever its own properties say.
        pass.draw_children(&mut target, &doc.root.children);
        (target, pass.warnings)
    }

    pub fn render(&mut self, doc: &Doc, assets: &AssetBytes) -> RenderResult {
        let (target, warnings) = self.render_region(doc, assets, Region::new(0, 0, doc.width as i32, doc.height as i32));
        RenderResult { width: doc.width, height: doc.height, pixels: target.data, warnings }
    }

    /// Put a decoded image in the cache under an asset id, replacing what the
    /// asset bytes would decode to. This is how painted layers are stored.
    pub fn set_image(&mut self, id: &str, image: Image) {
        self.images.insert(id.to_string(), Some(Rc::new(image)));
    }

    /// The decoded image for an asset, for editing in place. Decodes it first if needed.
    pub fn image_mut(&mut self, id: &str, assets: &AssetBytes) -> Option<&mut Image> {
        if !self.images.contains_key(id) {
            let decoded = assets.get(id).and_then(|b| decode_image(b)).map(Rc::new);
            self.images.insert(id.to_string(), decoded);
        }
        self.images.get_mut(id)?.as_mut().map(Rc::make_mut)
    }

    pub fn image(&self, id: &str) -> Option<&Image> {
        self.images.get(id)?.as_deref()
    }
}

/// Render a whole document once. To render repeatedly, keep a [`Renderer`].
pub fn render(doc: &Doc, assets: &AssetBytes) -> RenderResult {
    Renderer::new().render(doc, assets)
}

/// FNV-1a hash of a pixel buffer. For comparing renders across builds.
pub fn hash(pixels: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in pixels {
        h ^= byte as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
