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
//!
//! Anything expensive is computed in fixed 256 px tiles on the document grid
//! and cached: path coverage, and the coverage of every layer effect. A tile
//! is always computed the same way whatever region asked for it, so drawing
//! part of a document gives exactly the pixels a full render has there, and
//! redrawing after a small change is mostly compositing cached tiles.

use crate::doc::{BlendMode, Color, Doc, Effect, Fill, FillRule, GroupLayer, Layer, Outside, Path, ShapeLayer, StrokeAlign, TextLayer};
use crate::effects::{effect_extent, ordered, EffectContext, Shape};
use crate::field::blur;
use crate::raster::{composite, cross_fade, multiply_alpha, place_image, resample, stamp, Cover, Image, Ink, Raster, Region};
use crate::serialize::AssetBytes;
use crate::text::{layout_text, Fonts, TextLayout};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub struct RenderResult {
    pub width: u32,
    pub height: u32,
    /// Straight-alpha RGBA, 8 bits per channel, row-major.
    pub pixels: Vec<u8>,
    /// Things the document asked for that this build does not draw.
    pub warnings: Vec<String>,
}

/// Side of a cache tile in pixels.
const TILE: i32 = 256;

/// One tile of 8-bit coverage. Uniform tiles cost no memory.
enum Tile {
    Empty,
    Full,
    Mixed(Box<[u8]>),
}

impl Tile {
    fn from(data: &[u8]) -> Tile {
        if data.iter().all(|&v| v == 0) {
            Tile::Empty
        } else if data.iter().all(|&v| v == 255) {
            Tile::Full
        } else {
            Tile::Mixed(data.into())
        }
    }
}

fn tile_region(tx: i32, ty: i32) -> Region {
    Region::new(tx * TILE, ty * TILE, TILE, TILE)
}

/// The tiles a region overlaps.
fn tiles_over(region: &Region) -> impl Iterator<Item = (i32, i32)> {
    let (x0, x1) = (region.x.div_euclid(TILE), (region.x + region.w - 1).div_euclid(TILE));
    let (y0, y1) = (region.y.div_euclid(TILE), (region.y + region.h - 1).div_euclid(TILE));
    let empty = region.area() == 0;
    (y0..=y1).flat_map(move |ty| (x0..=x1).map(move |tx| (tx, ty))).filter(move |_| !empty)
}

/// Anti-aliased coverage of one path, rasterized tile by tile.
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
            Tile::from(mask.data())
        })
    }

    /// Coverage (0..255) over `region`, row-major.
    fn over(&mut self, region: &Region) -> Vec<u8> {
        let mut out = vec![0u8; region.area()];
        for (tx, ty) in tiles_over(region) {
            let bounds = tile_region(tx, ty);
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
                        let (from, to) = (bounds.index(area.x, y), region.index(area.x, y));
                        out[to..to + area.w as usize].copy_from_slice(&data[from..from + area.w as usize]);
                    }
                }
            }
        }
        out
    }
}

/// One effect's contribution inside one tile: "paint this ink through this coverage".
struct StampTile {
    coverage: Tile,
    ink: Ink,
    blend: BlendMode,
    opacity: f32,
    /// Painted over the layer's own pixels (true) or under them (false).
    above: bool,
}

/// A text layer's layout and the coverage of each of its colors, valid while
/// the layer's text and the loaded fonts stay the same.
struct TextCache {
    fingerprint: u64,
    layout: TextLayout,
    coverage: Vec<Coverage>,
}

/// A layer's effect tiles, valid while the layer's fingerprint stays the same.
struct EffectCache {
    fingerprint: u64,
    tiles: HashMap<(i32, i32), Rc<Vec<StampTile>>>,
}

/// Renders documents, keeping decoded images, path coverage and effect tiles
/// between calls.
///
/// One renderer serves one document. After the document changes, call
/// [`Renderer::sync`] so stale effect tiles are dropped; [`Renderer::render`]
/// and [`Renderer::render_region`] do it themselves.
#[derive(Default)]
pub struct Renderer {
    images: HashMap<String, Option<Rc<Image>>>,
    /// Bumped whenever an image's pixels change. Part of layer fingerprints.
    versions: HashMap<String, u64>,
    paths: HashMap<String, Option<Rc<tiny_skia::Path>>>,
    /// Keyed by what is being filled: see `fill_coverage` and `stroke_coverage`.
    coverage: HashMap<String, Coverage>,
    effects: HashMap<String, EffectCache>,
    /// Fingerprint of everything that decides each effect-bearing layer's effect tiles.
    fingerprints: HashMap<String, u64>,
    /// Layers being painted on right now: their effects are recomputed, not cached.
    live: HashSet<String>,
    /// The fonts text layers are drawn with. Supplied by the user, not stored in documents.
    fonts: Fonts,
    texts: HashMap<String, TextCache>,
}

struct RenderPass<'a> {
    assets: &'a AssetBytes,
    cache: &'a mut Renderer,
    /// The whole document. Layers are cut off at its edge.
    doc: Region,
    warnings: Vec<String>,
    /// Draw only part of the paint order, split at one layer. See [`Split`].
    split: Option<(Split<'a>, bool)>,
}

/// A cut through the paint order at one layer, for drawing "everything under
/// it" and "it and everything over it" separately. Only valid when every group
/// around the layer is a plain pass-through group, so that the paint order is
/// a simple sequence (see [`can_split_at`]).
#[derive(Clone, Copy)]
pub struct Split<'a> {
    pub layer: &'a str,
    /// Ids of the groups around the layer, outermost first.
    pub ancestors: &'a [String],
    /// True: draw what comes before the layer. False: draw the layer and what comes after.
    pub below: bool,
}

/// Whether the document's paint order can be cut at this layer: the layer is
/// not clipped, and every group around it is a visible pass-through group at
/// full opacity with no mask, no effects and no clipping. Returns the ids of
/// those groups, outermost first.
pub fn can_split_at(doc: &Doc, id: &str) -> Option<Vec<String>> {
    let path = crate::tree::locate(doc, id)?;
    let mut siblings = &doc.root.children[..];
    let mut ancestors = Vec::new();
    for (depth, &i) in path.iter().enumerate() {
        let layer = &siblings[i];
        let base = layer.base();
        if base.clip {
            return None;
        }
        if depth + 1 == path.len() {
            return Some(ancestors);
        }
        let Layer::Group(group) = layer else { return None };
        let clipped_to_it = siblings.get(i + 1).is_some_and(|next| next.base().clip);
        if !group.pass_through || !base.visible || base.opacity < 1.0 || base.has_active_mask() || !base.active_effects().is_empty() || clipped_to_it {
            return None;
        }
        ancestors.push(base.id.clone());
        siblings = &group.children;
    }
    None
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

fn union(a: Option<Region>, b: Option<Region>) -> Option<Region> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let (x, y) = (a.x.min(b.x), a.y.min(b.y));
            Some(Region::new(x, y, (a.x + a.w).max(b.x + b.w) - x, (a.y + a.h).max(b.y + b.h) - y))
        }
        (a, b) => a.or(b),
    }
}

fn grow(region: Region, by: i32) -> Region {
    Region::new(region.x - by, region.y - by, region.w + by * 2, region.h + by * 2)
}

impl<'a> RenderPass<'a> {
    fn warn(&mut self, message: String) {
        if !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }

    fn image(&mut self, id: &str, used_by: &str) -> Option<Rc<Image>> {
        let image = self.cache.decoded(id, self.assets);
        if image.is_none() {
            self.warn(if self.assets.contains_key(id) { format!("Asset \"{id}\" is not a readable image.") } else { format!("Asset \"{id}\" has no data.") });
            self.warn(format!("Layer \"{used_by}\" was skipped: its image is unavailable."));
        }
        image
    }

    fn path(&mut self, source: &Path) -> Option<Rc<tiny_skia::Path>> {
        match self.cache.parsed(source) {
            Ok(path) => path,
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
            // When drawing one side of a split, walk only the groups that lead to the
            // layer, and switch drawing on or off as the layer is reached.
            if let Some((split, reached)) = self.split.as_mut() {
                let id = base.id();
                if !*reached && id == split.layer {
                    *reached = true;
                    if split.below {
                        return;
                    }
                } else if !*reached && split.ancestors.iter().any(|a| a == id) {
                    if let Layer::Group(group) = base {
                        self.draw_children(target, &group.children);
                    }
                    i = end;
                    continue;
                } else if *reached == split.below {
                    // Below the layer but already past it, or above it but not there yet: nothing to draw.
                    if split.below {
                        return;
                    }
                    i = end;
                    continue;
                }
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
        let has_effects = !base.active_effects().is_empty();

        if let Layer::Group(group) = layer {
            if group.pass_through && !has_effects && clipped.is_empty() {
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

        let mut content = self.masked_content(layer, &target.region);
        if let Some(content) = content.as_mut() {
            for child in clipped {
                // Rendered on its own first, so its masks, effects and children are all honoured.
                // Clipped layers blend into the base without changing its alpha.
                let mut own = Raster::new(content.region);
                self.draw_layer(&mut own, child, &[], 1.0, BlendMode::Normal);
                composite(content, &own, child.base().opacity as f32, child.base().blend, true);
            }
        }
        let fill = base.fill_opacity.unwrap_or(1.0) as f32;

        if !has_effects {
            if let Some(content) = &content {
                composite(target, content, opacity * fill, blend, false);
            }
            return;
        }

        // Effects come from cached tiles. They are found even when the layer has no pixels
        // of its own in the target: a shadow can fall into it from next door.
        let tiles: Vec<((i32, i32), Rc<Vec<StampTile>>)> = tiles_over(&target.region).map(|t| (t, self.effect_tile(layer, t))).collect();
        if content.is_none() && tiles.iter().all(|(_, stamps)| stamps.is_empty()) {
            return;
        }
        let paint = |work: &mut Raster| {
            let stamps = |work: &mut Raster, above: bool| {
                for ((tx, ty), list) in &tiles {
                    for s in list.iter().filter(|s| s.above == above) {
                        let cover = match &s.coverage {
                            Tile::Empty => continue,
                            Tile::Full => Cover::Full,
                            Tile::Mixed(map) => Cover::Map(map),
                        };
                        stamp(work, &tile_region(*tx, *ty), &cover, &s.ink, s.opacity, s.blend);
                    }
                }
            };
            stamps(work, false);
            if let Some(content) = &content {
                composite(work, content, fill, blend, false);
            }
            stamps(work, true);
        };
        if opacity < 1.0 {
            let mut work = target.clone();
            paint(&mut work);
            cross_fade(target, &work, opacity, None);
        } else {
            paint(target);
        }
    }

    /// The effect stamps of one layer inside one tile, from the cache when possible.
    fn effect_tile(&mut self, layer: &Layer, tile: (i32, i32)) -> Rc<Vec<StampTile>> {
        let id = layer.id();
        let live = self.cache.live.contains(id);
        let fingerprint = self.cache.fingerprints.get(id).copied();
        if !live {
            if let (Some(cache), Some(fp)) = (self.cache.effects.get(id), fingerprint) {
                if cache.fingerprint == fp {
                    if let Some(stamps) = cache.tiles.get(&tile) {
                        return stamps.clone();
                    }
                }
            }
        }
        let stamps = Rc::new(self.compute_effect_tile(layer, tile));
        if let (false, Some(fp)) = (live, fingerprint) {
            let cache = self.cache.effects.entry(id.to_string()).or_insert_with(|| EffectCache { fingerprint: fp, tiles: HashMap::new() });
            if cache.fingerprint != fp {
                *cache = EffectCache { fingerprint: fp, tiles: HashMap::new() };
            }
            cache.tiles.insert(tile, stamps.clone());
        }
        stamps
    }

    fn compute_effect_tile(&mut self, layer: &Layer, (tx, ty): (i32, i32)) -> Vec<StampTile> {
        let effects = layer.base().active_effects();
        // Effects look at the layer's alpha around each pixel, so the tile is
        // computed from a window that is wider by their reach.
        let margin = effects.iter().map(|e| effect_extent(e)).max().unwrap_or(0);
        let tile = tile_region(tx, ty);
        let window = grow(tile, margin);
        let Some(inside) = window.intersect(&self.doc) else { return Vec::new() };
        let Some(content) = self.masked_content(layer, &inside) else { return Vec::new() };

        // Alpha over the whole window. Beyond the document edge it is zero, so a
        // layer that touches the edge has an edge there, as in Photoshop.
        let mut alpha = vec![0f32; window.area()];
        let (mut any, mut all) = (false, inside == window && content.region == window);
        for y in content.region.y..content.region.y + content.region.h {
            let mut src = content.region.index(content.region.x, y) * 4 + 3;
            let mut dst = window.index(content.region.x, y);
            for _ in 0..content.region.w {
                let a = content.data[src];
                any |= a != 0;
                all &= a == 255;
                alpha[dst] = a as f32 / 255.0;
                src += 4;
                dst += 1;
            }
        }
        if !any {
            return Vec::new();
        }
        let bounds = self.layer_bounds(layer).unwrap_or(self.doc);
        let shape = Shape { region: window, alpha, bounds };
        let fx = EffectContext::new(&shape);
        let (below, above) = ordered(&effects);
        let mut out = Vec::new();
        for (list, is_above) in [(below, false), (above, true)] {
            for effect in list {
                // Deep inside the layer only the overlays paint anything: skip the distance field.
                if all && !matches!(effect, Effect::ColorOverlay { .. } | Effect::GradientOverlay { .. }) {
                    continue;
                }
                for s in fx.stamps(effect) {
                    let mut bytes = Vec::with_capacity(tile.area());
                    for y in tile.y..tile.y + tile.h {
                        let start = window.index(tile.x, y);
                        bytes.extend(s.coverage[start..start + TILE as usize].iter().map(|&c| (c * 255.0 + 0.5) as u8));
                    }
                    let coverage = Tile::from(&bytes);
                    if !matches!(coverage, Tile::Empty) {
                        out.push(StampTile { coverage, ink: s.ink, blend: s.blend, opacity: s.opacity, above: is_above });
                    }
                }
            }
        }
        out
    }

    /// The layer's own pixels within `region`, with its masks applied. `None` when there are none.
    fn masked_content(&mut self, layer: &Layer, region: &Region) -> Option<Raster> {
        let mut content = self.content(layer, region)?;
        if let Some(mask) = self.mask_coverage(layer, &content.region) {
            multiply_alpha(&mut content, &mask);
        }
        Some(content)
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
            Layer::Text(text) => return self.text(text, region),
            Layer::Pixel(l) => (&l.asset, &l.transform),
            Layer::Fill(l) => (&l.asset, &l.transform),
            Layer::Art(l) => (l.asset.as_ref()?, &l.transform),
        };
        let image = self.image(asset, &layer.base().name)?;
        if let Some((tx, ty)) = whole_pixel_offset(transform) {
            place_image(&image, tx, ty, region)
        } else {
            resample(&image, transform, region)
        }
    }

    /// A text layer's layout, from the cache when its text and the fonts are unchanged.
    fn text_cache(&mut self, layer: &TextLayer) -> &mut TextCache {
        let mut hash = Fnv::new();
        hash.json(&(&layer.bounds, &layer.origin, &layer.content, &layer.style, &layer.fit, &layer.vertical_align));
        hash.write(&self.cache.fonts.version().to_le_bytes());
        let fingerprint = hash.0;
        let stale = self.cache.texts.get(&layer.base.id).is_none_or(|c| c.fingerprint != fingerprint);
        if stale {
            let layout = layout_text(layer, &self.cache.fonts);
            let coverage = layout.parts.iter().map(|(_, path)| Coverage::new(Rc::new(path.clone()), tiny_skia::FillRule::Winding)).collect();
            self.cache.texts.insert(layer.base.id.clone(), TextCache { fingerprint, layout, coverage });
        }
        let missing = self.cache.texts[&layer.base.id].layout.missing.clone();
        for font in missing {
            self.warn(format!("Text layer \"{}\" needs the font \"{font}\", which is not loaded.", layer.base.name));
        }
        self.cache.texts.get_mut(&layer.base.id).expect("just inserted")
    }

    /// Where a text layer's ink is, padded for anti-aliasing.
    fn text_bounds(&mut self, layer: &TextLayer) -> Option<Region> {
        let ink = self.text_cache(layer).layout.ink?;
        let (x0, y0) = (ink[0].floor() as i32 - 1, ink[1].floor() as i32 - 1);
        Some(Region::new(x0, y0, ink[2].ceil() as i32 + 1 - x0, ink[3].ceil() as i32 + 1 - y0))
    }

    fn text(&mut self, layer: &TextLayer, region: &Region) -> Option<Raster> {
        let area = region.intersect(&self.text_bounds(layer)?)?;
        let cache = self.text_cache(layer);
        let mut out = Raster::new(area);
        let mut first = true;
        for ((color, _), coverage) in cache.layout.parts.iter().zip(cache.coverage.iter_mut()) {
            let cov = coverage.over(&area);
            if first {
                ink(&mut out, &cov, color);
                first = false;
            } else {
                // Later colors go over earlier ones: a symbol's glyph over its disc.
                let mut over = Raster::new(area);
                ink(&mut over, &cov, color);
                composite(&mut out, &over, 1.0, BlendMode::Normal, false);
            }
        }
        Some(out)
    }

    fn shape(&mut self, layer: &ShapeLayer, region: &Region) -> Option<Raster> {
        let area = region.intersect(&self.shape_bounds(layer, 1.0)?)?;
        let stroke = layer.stroke.as_ref().filter(|s| s.width > 0.0);
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

    /// Where a shape layer can have pixels: its path's bounds, widened for a
    /// centered stroke. `pad` adds room for anti-aliasing; pass 0 for the
    /// shape's nominal bounds.
    fn shape_bounds(&mut self, layer: &ShapeLayer, pad: f32) -> Option<Region> {
        let path = self.path(&layer.path)?;
        let grow = pad
            + match &layer.stroke {
                Some(s) if s.width > 0.0 && s.align == StrokeAlign::Center => s.width as f32 / 2.0,
                _ => 0.0,
            };
        let b = path.bounds();
        let (x0, y0) = ((b.left() - grow).floor() as i32, (b.top() - grow).floor() as i32);
        Some(Region::new(x0, y0, (b.right() + grow).ceil() as i32 - x0, (b.bottom() + grow).ceil() as i32 - y0))
    }

    /// A layer's bounds from its geometry alone (no rendering), cut to the
    /// document. Shapes report their path's bounds; an anti-aliased edge can
    /// spill one pixel past them.
    fn layer_bounds(&mut self, layer: &Layer) -> Option<Region> {
        let own = match layer {
            Layer::Group(group) => group.children.iter().filter(|l| l.base().visible).fold(None, |acc, child| union(acc, self.layer_bounds(child))),
            Layer::Shape(shape) => self.shape_bounds(shape, 0.0),
            Layer::Text(text) => self.text_bounds(text),
            Layer::Pixel(_) | Layer::Fill(_) | Layer::Art(_) => {
                let (asset, m) = match layer {
                    Layer::Pixel(l) => (&l.asset, &l.transform),
                    Layer::Fill(l) => (&l.asset, &l.transform),
                    Layer::Art(l) => (l.asset.as_ref()?, &l.transform),
                    _ => unreachable!(),
                };
                let image = self.cache.decoded(asset, self.assets)?;
                let (w, h) = (image.w as f64, image.h as f64);
                let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].map(|(u, v)| (m[0] * u + m[1] * v + m[2], m[3] * u + m[4] * v + m[5]));
                let (x0, y0) = (corners.iter().map(|c| c.0).fold(f64::INFINITY, f64::min).floor() as i32, corners.iter().map(|c| c.1).fold(f64::INFINITY, f64::min).floor() as i32);
                let (x1, y1) = (corners.iter().map(|c| c.0).fold(f64::NEG_INFINITY, f64::max).ceil() as i32, corners.iter().map(|c| c.1).fold(f64::NEG_INFINITY, f64::max).ceil() as i32);
                Some(Region::new(x0, y0, x1 - x0, y1 - y0))
            }
        }?;
        let mut bounds = own.intersect(&self.doc)?;
        // A vector mask that is not inverted can only shrink the layer.
        if let Some(mask) = layer.base().vector_mask.as_ref().filter(|m| m.enabled != Some(false) && m.inverted != Some(true)) {
            let b = self.path(&mask.path)?.bounds();
            let (x0, y0) = (b.left().floor() as i32 - 1, b.top().floor() as i32 - 1);
            bounds = bounds.intersect(&Region::new(x0, y0, b.right().ceil() as i32 + 1 - x0, b.bottom().ceil() as i32 + 1 - y0))?;
        }
        Some(bounds)
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
        let wide = grow(*region, margin);
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

/// `Some((tx, ty))` if the transform only moves the image by whole pixels.
pub fn whole_pixel_offset(m: &[f64; 6]) -> Option<(i32, i32)> {
    let [a, c, tx, b, d, ty] = *m;
    (a == 1.0 && d == 1.0 && b == 0.0 && c == 0.0 && tx.fract() == 0.0 && ty.fract() == 0.0).then_some((tx as i32, ty as i32))
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

/// FNV-1a, fed in pieces.
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= byte as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn json<T: serde::Serialize>(&mut self, value: &T) {
        self.write(&serde_json::to_vec(value).unwrap_or_default());
    }
}

impl Renderer {
    pub fn new() -> Renderer {
        Renderer::default()
    }

    fn decoded(&mut self, id: &str, assets: &AssetBytes) -> Option<Rc<Image>> {
        if !self.images.contains_key(id) {
            let decoded = assets.get(id).and_then(|b| decode_image(b)).map(Rc::new);
            self.images.insert(id.to_string(), decoded);
        }
        self.images[id].clone()
    }

    fn parsed(&mut self, source: &Path) -> Result<Option<Rc<tiny_skia::Path>>, ()> {
        if let Some(cached) = self.paths.get(&source.d) {
            return Ok(cached.clone());
        }
        // A malformed path is not cached, so its warning is reported on every render.
        let path = build_path(source)?.map(Rc::new);
        self.paths.insert(source.d.clone(), path.clone());
        Ok(path)
    }

    /// Bring the caches in step with the document. Call after any change to
    /// the document or to an image; effect tiles of layers that changed (or
    /// that contain something that changed) are dropped.
    pub fn sync(&mut self, doc: &Doc) {
        let mut fingerprints = HashMap::new();
        self.fingerprint_group(&doc.root, &mut fingerprints);
        self.effects.retain(|id, cache| fingerprints.get(id) == Some(&cache.fingerprint));
        self.fingerprints = fingerprints;
        // Layouts of text layers that no longer exist.
        let mut alive = HashSet::new();
        crate::tree::walk(&doc.root, &mut |layer| {
            if matches!(layer, Layer::Text(_)) {
                alive.insert(layer.id());
            }
        });
        self.texts.retain(|id, _| alive.contains(id.as_str()));
    }

    /// The fonts text is drawn with. After adding one, call [`Renderer::sync`].
    pub fn fonts_mut(&mut self) -> &mut Fonts {
        &mut self.fonts
    }

    pub fn fonts(&self) -> &Fonts {
        &self.fonts
    }

    /// The ink bounds of every visible-or-not text layer, by layer id, as
    /// (left, top, right, bottom) in document pixels. For checking layout
    /// against a reference.
    pub fn text_ink_bounds(&mut self, doc: &Doc) -> Vec<(String, Option<[f64; 4]>)> {
        let mut out = Vec::new();
        crate::tree::walk(&doc.root, &mut |layer| {
            if let Layer::Text(text) = layer {
                out.push((text.base.id.clone(), layout_text(text, &self.fonts).ink));
            }
        });
        out
    }

    /// Hash everything that decides what a layer looks like, and record the
    /// effect fingerprint of each layer that has effects. Returns the hash of
    /// the group's children, in order.
    fn fingerprint_group(&self, group: &GroupLayer, out: &mut HashMap<String, u64>) -> u64 {
        let mut all = Fnv::new();
        for layer in &group.children {
            // What the layer is, apart from how it is composited.
            let mut content = Fnv::new();
            match layer {
                Layer::Group(g) => {
                    content.write(&[g.pass_through as u8]);
                    content.write(&self.fingerprint_group(g, out).to_le_bytes());
                }
                Layer::Pixel(l) => {
                    content.json(&(&l.asset, &l.transform));
                    content.write(&self.versions.get(&l.asset).copied().unwrap_or(0).to_le_bytes());
                }
                Layer::Fill(l) => {
                    content.json(&(&l.asset, &l.transform));
                    content.write(&self.versions.get(&l.asset).copied().unwrap_or(0).to_le_bytes());
                }
                Layer::Art(l) => {
                    content.json(&(&l.asset, &l.transform));
                    if let Some(asset) = &l.asset {
                        content.write(&self.versions.get(asset).copied().unwrap_or(0).to_le_bytes());
                    }
                }
                Layer::Shape(l) => content.json(&(&l.path, &l.fill, &l.stroke)),
                Layer::Text(l) => {
                    content.json(&(&l.bounds, &l.origin, &l.content, &l.style, &l.fit, &l.vertical_align));
                    content.write(&self.fonts.version().to_le_bytes());
                }
            }
            let base = layer.base();
            content.json(&(&base.mask, &base.vector_mask));
            if let Some(mask) = &base.mask {
                content.write(&self.versions.get(&mask.asset).copied().unwrap_or(0).to_le_bytes());
            }
            // Effect tiles depend on the layer's masked content and its effects, and nothing else.
            if !base.active_effects().is_empty() {
                let mut fx = Fnv::new();
                fx.write(&content.0.to_le_bytes());
                fx.json(&base.effects);
                out.insert(base.id.clone(), fx.0);
            }
            // To its parent, everything about the layer matters.
            all.write(&content.0.to_le_bytes());
            all.json(&(&base.effects, base.effects_enabled, base.visible, base.opacity, base.fill_opacity, base.blend, base.clip));
        }
        all.0
    }

    /// Mark layers whose effects must be recomputed on every draw, because they
    /// are being painted on. Pass an empty list when the stroke ends.
    pub fn set_live(&mut self, layers: &[String]) {
        self.live = layers.iter().cloned().collect();
    }

    /// Draw part of a document whose caches are already in step (see [`Renderer::sync`]).
    pub fn draw_region(&mut self, doc: &Doc, assets: &AssetBytes, region: Region) -> (Raster, Vec<String>) {
        let doc_region = Region::new(0, 0, doc.width as i32, doc.height as i32);
        let mut target = Raster::new(region.intersect(&doc_region).unwrap_or(Region::new(region.x, region.y, 0, 0)));
        let warnings = self.draw_onto(doc, assets, &mut target, None);
        (target, warnings)
    }

    /// Draw onto an existing raster: the whole document, or one side of a
    /// [`Split`]. Drawing the part below a layer onto a clear raster, then the
    /// rest onto the result, gives the same pixels as drawing everything.
    pub fn draw_onto(&mut self, doc: &Doc, assets: &AssetBytes, target: &mut Raster, split: Option<Split>) -> Vec<String> {
        let doc_region = Region::new(0, 0, doc.width as i32, doc.height as i32);
        let mut pass = RenderPass { assets, cache: self, doc: doc_region, warnings: Vec::new(), split: split.map(|s| (s, false)) };
        // The root is always a plain container, whatever its own properties say.
        pass.draw_children(target, &doc.root.children);
        pass.warnings
    }

    /// Render the part of the document inside `region`. The result is exactly
    /// what a full render would show there.
    pub fn render_region(&mut self, doc: &Doc, assets: &AssetBytes, region: Region) -> (Raster, Vec<String>) {
        self.sync(doc);
        self.draw_region(doc, assets, region)
    }

    pub fn render(&mut self, doc: &Doc, assets: &AssetBytes) -> RenderResult {
        let (target, warnings) = self.render_region(doc, assets, Region::new(0, 0, doc.width as i32, doc.height as i32));
        RenderResult { width: doc.width, height: doc.height, pixels: target.data, warnings }
    }

    /// The area a layer can affect: its bounds, widened by the reach of its
    /// own effects and those of every group around it. `None` if the layer is
    /// unknown or its extent cannot be told without rendering (then assume the
    /// whole document).
    pub fn layer_reach(&mut self, doc: &Doc, assets: &AssetBytes, id: &str) -> Option<Region> {
        let path = crate::tree::locate(doc, id)?;
        let doc_region = Region::new(0, 0, doc.width as i32, doc.height as i32);
        let mut margin = 0;
        let mut layer = &doc.root.children[path[0]];
        for &i in &path[1..] {
            margin = margin.max(layer.base().active_effects().iter().map(|e| effect_extent(e)).max().unwrap_or(0));
            layer = &layer.children()[i];
        }
        margin = margin.max(layer.base().active_effects().iter().map(|e| effect_extent(e)).max().unwrap_or(0));
        let mut pass = RenderPass { assets, cache: self, doc: doc_region, warnings: Vec::new(), split: None };
        // One more pixel for anti-aliased edges.
        grow(pass.layer_bounds(layer)?, margin + 1).intersect(&doc_region)
    }

    /// Put a decoded image in the cache under an asset id, replacing what the
    /// asset bytes would decode to. This is how painted layers are stored.
    pub fn set_image(&mut self, id: &str, image: Image) {
        self.images.insert(id.to_string(), Some(Rc::new(image)));
        *self.versions.entry(id.to_string()).or_insert(0) += 1;
    }

    /// The decoded image for an asset, for editing in place. Decodes it first
    /// if needed. Counts as a change to the image.
    pub fn image_mut(&mut self, id: &str, assets: &AssetBytes) -> Option<&mut Image> {
        self.decoded(id, assets)?;
        *self.versions.entry(id.to_string()).or_insert(0) += 1;
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
    let mut h = Fnv::new();
    h.write(pixels);
    h.0
}
