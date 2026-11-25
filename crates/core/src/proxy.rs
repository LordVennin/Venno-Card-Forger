//! A smaller copy of a document, for drawing the screen quickly.
//!
//! A card is 3264 × 4440 pixels and the screen shows it at a fraction of
//! that. Drawing every edit at card size and then shrinking the result does
//! sixteen times the work a quarter-size picture needs. So while the view is
//! zoomed out, the session draws a copy of the document scaled by 1/2, 1/4 or
//! 1/8 with the ordinary renderer, and brings the full-size picture up to
//! date only when it is wanted (saving, exporting, painting).
//!
//! The copy is the same document with every length scaled: positions, paths,
//! effect sizes, mask offsets. Images are halved to match, so their layers'
//! scale stays as it is. Text is the exception: it is laid out at full size,
//! exactly as the real document lays it out, and its finished outlines are
//! scaled, so a line never breaks differently in the small picture.
//!
//! The small picture is a preview. It is not what is saved or exported, and
//! thin details (a one-pixel stroke, a small shadow) come out softer in it.

use crate::doc::*;
use crate::raster::Image;
use crate::text::{layout_text, Fonts};
use std::collections::HashMap;

/// What making one smaller copy after another can reuse: the document is scaled
/// again after every edit, and nearly all of it is the same as the time before.
#[derive(Default)]
pub struct TextOutlines {
    /// Outlines of text layers already laid out, by layer id: what they were made from, and one path per color.
    texts: HashMap<String, (u64, Vec<(Color, String)>)>,
    /// Paths already scaled, by a hash of the path they were scaled from and its length.
    paths: HashMap<(u64, usize), String>,
}

impl TextOutlines {
    pub fn new() -> TextOutlines {
        TextOutlines::default()
    }
}

fn scale_path(d: &str, s: f64, cache: &mut TextOutlines) -> String {
    let key = (fnv(d.as_bytes()), d.len());
    if let Some(done) = cache.paths.get(&key) {
        return done.clone();
    }
    let scaled = match kurbo::BezPath::from_svg(d) {
        Ok(mut path) => {
            path.apply_affine(kurbo::Affine::scale(s));
            path.to_svg()
        }
        // The renderer will report it; scaled or not, it draws nothing.
        Err(_) => d.to_string(),
    };
    // A dragged shape leaves a path behind for every position it passed through.
    if cache.paths.len() > 2048 {
        cache.paths.clear();
    }
    cache.paths.insert(key, scaled.clone());
    scaled
}

fn outline(path: &tiny_skia::Path, s: f64) -> String {
    use std::fmt::Write;
    use tiny_skia::PathSegment as Seg;
    let mut d = String::new();
    let n = |v: f32| (v as f64 * s * 1000.0).round() / 1000.0;
    for segment in path.segments() {
        let _ = match segment {
            Seg::MoveTo(p) => write!(d, "M{} {}", n(p.x), n(p.y)),
            Seg::LineTo(p) => write!(d, "L{} {}", n(p.x), n(p.y)),
            Seg::QuadTo(a, p) => write!(d, "Q{} {} {} {}", n(a.x), n(a.y), n(p.x), n(p.y)),
            Seg::CubicTo(a, b, p) => write!(d, "C{} {} {} {} {} {}", n(a.x), n(a.y), n(b.x), n(b.y), n(p.x), n(p.y)),
            Seg::Close => write!(d, "Z"),
        };
    }
    d
}

fn scale_base(base: &mut LayerBase, s: f64, cache: &mut TextOutlines) {
    if let Some(mask) = &mut base.mask {
        mask.offset = Vec2 { x: mask.offset.x * s, y: mask.offset.y * s };
        mask.feather = mask.feather.map(|f| f * s);
    }
    if let Some(mask) = &mut base.vector_mask {
        mask.path.d = scale_path(&mask.path.d, s, cache);
    }
    for effect in base.effects.iter_mut().flatten() {
        match effect {
            Effect::Stroke { size, .. } | Effect::OuterGlow { size, .. } => *size *= s,
            Effect::DropShadow { distance, size, .. } | Effect::InnerShadow { distance, size, .. } => {
                *distance *= s;
                *size *= s;
            }
            Effect::Bevel { size, soften, .. } => {
                *size *= s;
                *soften *= s;
            }
            // Overlays are placed relative to the layer's bounds.
            Effect::ColorOverlay { .. } | Effect::GradientOverlay { .. } => {}
        }
    }
}

/// A quick hash, eight bytes at a time: paths run to hundreds of kilobytes.
fn fnv(bytes: &[u8]) -> u64 {
    let mut chunks = bytes.chunks_exact(8);
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for chunk in &mut chunks {
        h = (h ^ u64::from_le_bytes(chunk.try_into().expect("eight bytes"))).wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(29);
    }
    chunks.remainder().iter().fold(h, |h, b| (h ^ *b as u64).wrapping_mul(0x100_0000_01b3))
}

fn scale_group(group: &mut GroupLayer, s: f64, fonts: &Fonts, texts: &mut TextOutlines) {
    scale_base(&mut group.base, s, texts);
    for layer in &mut group.children {
        match layer {
            Layer::Group(inner) => scale_group(inner, s, fonts, texts),
            Layer::Pixel(l) => {
                scale_base(&mut l.base, s, texts);
                l.transform[2] *= s;
                l.transform[5] *= s;
            }
            Layer::Fill(l) => {
                scale_base(&mut l.base, s, texts);
                l.transform[2] *= s;
                l.transform[5] *= s;
            }
            Layer::Art(l) => {
                scale_base(&mut l.base, s, texts);
                l.transform[2] *= s;
                l.transform[5] *= s;
                l.frame_ref = Rect { x: l.frame_ref.x * s, y: l.frame_ref.y * s, w: l.frame_ref.w * s, h: l.frame_ref.h * s };
            }
            Layer::Shape(l) => {
                scale_base(&mut l.base, s, texts);
                l.path.d = scale_path(&l.path.d, s, texts);
                if let Some(stroke) = &mut l.stroke {
                    stroke.width *= s;
                }
            }
            Layer::Text(l) => {
                // Laid out at full size, as the real document does, then its outlines scaled:
                // a group standing in for the layer, holding one shape for each color of ink.
                let made_from = fnv(format!("{}|{}|{s}", serde_json::to_string(&(&l.bounds, &l.origin, &l.content, &l.style, &l.fit, &l.vertical_align)).unwrap_or_default(), fonts.version()).as_bytes());
                if texts.texts.get(&l.base.id).is_none_or(|(from, _)| *from != made_from) {
                    let parts = layout_text(l, fonts).parts.iter().map(|(color, path)| (*color, outline(path, s))).collect();
                    texts.texts.insert(l.base.id.clone(), (made_from, parts));
                }
                let mut base = l.base.clone();
                scale_base(&mut base, s, texts);
                let children = texts.texts[&l.base.id]
                    .1
                    .iter()
                    .enumerate()
                    .map(|(i, (color, d))| Layer::Shape(ShapeLayer { base: LayerBase::new(&format!("{}#{i}", base.id)), path: Path { d: d.clone(), fill_rule: None }, fill: Fill::Solid { color: *color }, stroke: None }))
                    .collect();
                *layer = Layer::Group(GroupLayer { base, children, pass_through: false });
            }
        }
    }
}

/// The document at 1/2^level of its size.
pub fn scale_doc(doc: &Doc, level: u32, fonts: &Fonts, texts: &mut TextOutlines) -> Doc {
    let s = 1.0 / (1u32 << level) as f64;
    let mut small = doc.clone();
    small.width = (doc.width as f64 * s).ceil() as u32;
    small.height = (doc.height as f64 * s).ceil() as u32;
    small.dpi = doc.dpi * s;
    small.card = None;
    scale_group(&mut small.root, s, fonts, texts);
    let alive: std::collections::HashSet<&str> = crate::tree::all_ids(doc).into_iter().collect();
    texts.texts.retain(|id, _| alive.contains(id.as_str()));
    small
}

/// An image at 1/2^level of its size, each pixel the average of the block it stands for.
pub fn downscale(image: &Image, level: u32) -> Image {
    let k = 1i32 << level;
    let (w, h) = ((image.w + k - 1) / k, (image.h + k - 1) / k);
    let mut data = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            // Color is weighted by opacity, so a clear pixel's color does not bleed into its neighbors.
            let (mut r, mut g, mut b, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for sy in (y * k)..((y + 1) * k).min(image.h) {
                let row = (sy * image.w) as usize * 4;
                for sx in (x * k)..((x + 1) * k).min(image.w) {
                    let p = row + sx as usize * 4;
                    let alpha = image.data[p + 3] as u32;
                    r += image.data[p] as u32 * alpha;
                    g += image.data[p + 1] as u32 * alpha;
                    b += image.data[p + 2] as u32 * alpha;
                    a += alpha;
                    n += 1;
                }
            }
            let o = ((y * w + x) * 4) as usize;
            if a > 0 {
                data[o] = ((r + a / 2) / a) as u8;
                data[o + 1] = ((g + a / 2) / a) as u8;
                data[o + 2] = ((b + a / 2) / a) as u8;
                data[o + 3] = ((a + n / 2) / n) as u8;
            }
        }
    }
    Image { w, h, data }
}

/// Every image asset a document's layers and masks use.
pub fn image_assets(doc: &Doc) -> Vec<&str> {
    let mut ids = Vec::new();
    crate::tree::walk(&doc.root, &mut |layer| {
        match layer {
            Layer::Pixel(l) => ids.push(l.asset.as_str()),
            Layer::Fill(l) => ids.push(l.asset.as_str()),
            Layer::Art(l) => ids.extend(l.asset.as_deref()),
            _ => {}
        }
        if let Some(mask) = &layer.base().mask {
            ids.push(mask.asset.as_str());
        }
    });
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Draw a whole document at 1/2^level of its size: (width, height, pixels).
/// The session does this piece by piece; this is the same thing in one go,
/// for looking at what the small picture is like.
pub fn render_small(doc: &Doc, assets: &crate::serialize::AssetBytes, fonts: &Fonts, level: u32) -> (u32, u32, Vec<u8>) {
    let mut full = crate::render::Renderer::new();
    let mut small = crate::render::Renderer::new();
    let scaled = scale_doc(doc, level, fonts, &mut TextOutlines::new());
    for id in image_assets(doc) {
        if let Some(image) = full.decoded_image(id, assets) {
            small.set_image(id, downscale(&image, level));
        }
    }
    let drawn = small.render(&scaled, &Default::default());
    (drawn.width, drawn.height, drawn.pixels)
}
