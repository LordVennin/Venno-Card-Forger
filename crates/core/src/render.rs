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

use crate::doc::{BlendMode, Color, Doc, Fill, FillRule, Layer, Outside, Path, ShapeLayer, StrokeAlign};
use crate::effects::{effect_reach, ordered, EffectContext, Shape, Stamp};
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

struct RenderPass<'a> {
    assets: &'a AssetBytes,
    images: HashMap<String, Option<Rc<Image>>>,
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

/// Anti-aliased coverage (0..255) of a path over `region`.
fn coverage(path: &tiny_skia::Path, rule: tiny_skia::FillRule, region: &Region) -> Vec<u8> {
    let Some(mut mask) = tiny_skia::Mask::new(region.w as u32, region.h as u32) else {
        return vec![0; region.area()];
    };
    mask.fill_path(path, rule, true, tiny_skia::Transform::from_translate(-region.x as f32, -region.y as f32));
    mask.data().to_vec()
}

impl<'a> RenderPass<'a> {
    fn warn(&mut self, message: String) {
        if !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }

    fn image(&mut self, id: &str, used_by: &str) -> Option<Rc<Image>> {
        if !self.images.contains_key(id) {
            let bytes = self.assets.get(id);
            let decoded = bytes.and_then(|b| decode_image(b)).map(Rc::new);
            if decoded.is_none() {
                self.warn(if bytes.is_some() { format!("Asset \"{id}\" is not a readable image.") } else { format!("Asset \"{id}\" has no data.") });
            }
            self.images.insert(id.to_string(), decoded);
        }
        let image = self.images[id].clone();
        if image.is_none() {
            self.warn(format!("Layer \"{used_by}\" was skipped: its image is unavailable."));
        }
        image
    }

    fn path(&mut self, source: &Path) -> Option<tiny_skia::Path> {
        match build_path(source) {
            Ok(path) => path,
            Err(()) => {
                let head: String = source.d.chars().take(40).collect();
                self.warn(format!("A vector path could not be parsed: \"{head}\""));
                None
            }
        }
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

        let Some(mut content) = self.content(layer, &target.region) else { return };
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
        let rule = fill_rule(&layer.path);
        let mut out = Raster::new(area);
        let Fill::Solid { color } = &layer.fill;
        let fill = coverage(&path, rule, &area);
        ink(&mut out, &fill, color);
        if let Some(stroke) = stroke {
            let inside = stroke.align == StrokeAlign::Inside;
            // An inside stroke is a double-width stroke with the outer half clipped away.
            let pen = tiny_skia::Stroke { width: stroke.width as f32 * if inside { 2.0 } else { 1.0 }, ..Default::default() };
            if let Some(outline) = path.stroke(&pen, 1.0) {
                let mut edge = coverage(&outline, tiny_skia::FillRule::Winding, &area);
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
            let mut cov = match self.path(&vector.path) {
                Some(path) => coverage(&path, fill_rule(&vector.path), region),
                None => vec![0; region.area()],
            };
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
        let mut gray = vec![outside; n];
        let (ox, oy) = (mask.offset.x.round() as i32, mask.offset.y.round() as i32);
        if let Some(area) = region.intersect(&Region::new(ox, oy, image.w, image.h)) {
            for y in area.y..area.y + area.h {
                let mut si = ((y - oy) as usize * image.w as usize + (area.x - ox) as usize) * 4;
                let mut di = region.index(area.x, y);
                for _ in 0..area.w {
                    // Luma, so a mask that is not strictly gray still behaves sensibly.
                    let px = &image.data[si..si + 3];
                    gray[di] = ((54 * px[0] as u32 + 183 * px[1] as u32 + 19 * px[2] as u32) >> 8) as f32;
                    si += 4;
                    di += 1;
                }
            }
        }
        let feather = mask.feather.unwrap_or(0.0) as f32;
        if feather > 0.0 {
            // Blur the difference from the outside value, so the outside extends past the region
            // and the region's own border is not darkened. `sign` keeps the difference non-negative.
            let sign = if outside > 0.0 { -1.0 } else { 1.0 };
            let delta: Vec<f32> = gray.iter().map(|&v| sign * (v - outside) / 255.0).collect();
            let soft = blur(&delta, region.w as usize, region.h as usize, feather / 2.0);
            gray = soft.iter().map(|&v| outside + sign * v * 255.0).collect();
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

/// Render a whole document.
pub fn render(doc: &Doc, assets: &AssetBytes) -> RenderResult {
    let mut pass = RenderPass { assets, images: HashMap::new(), warnings: Vec::new() };
    let mut target = Raster::new(Region::new(0, 0, doc.width as i32, doc.height as i32));
    // The root is always a plain container, whatever its own properties say.
    pass.draw_children(&mut target, &doc.root.children);
    RenderResult { width: doc.width, height: doc.height, pixels: target.data, warnings: pass.warnings }
}
