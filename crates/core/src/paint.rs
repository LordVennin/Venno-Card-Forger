//! The brush engine: round brush and eraser on a decoded image.
//!
//! A stroke is a series of overlapping dabs. Opacity applies to the stroke as
//! a whole, as in Photoshop: each pixel remembers the strongest dab that has
//! touched it during the stroke, and its color is always recomputed from the
//! pixel as it was before the stroke began. Going over the same spot twice in
//! one stroke therefore does not build up.

use crate::raster::{Image, Region};
use serde::Deserialize;
use std::collections::BTreeMap;

/// Undo data is kept per block of this many pixels, so a long thin stroke stores only what it crossed.
pub const PATCH: i32 = 256;

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Brush {
    /// Diameter in pixels.
    pub size: f32,
    /// 0..1. At 1 the brush has a crisp, anti-aliased edge; lower values fade from that share of the radius outward.
    pub hardness: f32,
    /// 0..1, for the whole stroke.
    pub opacity: f32,
    /// Straight sRGB, 0..255. Ignored when erasing.
    pub color: [u8; 3],
    /// Erase (reduce alpha) instead of painting.
    pub erase: bool,
    /// With `erase` on a layer that is erased through its mask: bring back what was erased.
    #[serde(default)]
    pub restore: bool,
    /// Distance between dabs as a fraction of the diameter.
    pub spacing: f32,
}

impl Default for Brush {
    fn default() -> Self {
        Brush { size: 40.0, hardness: 0.8, opacity: 1.0, color: [0, 0, 0], erase: false, restore: false, spacing: 0.1 }
    }
}

pub struct Stroke {
    brush: Brush,
    /// The `PATCH`-sized blocks of the image the stroke has touched, each with the image's
    /// pixels there as they were when the stroke first reached it, and the strongest dab
    /// coverage each of its pixels has received so far (0..255). Kept only for blocks the
    /// stroke touches: copying a card-sized image at every pen-down took tens of milliseconds.
    patches: BTreeMap<(i32, i32), Block>,
    /// Everything this stroke has touched, in image pixels.
    dirty: Option<Region>,
    last: Option<(f32, f32)>,
    /// Distance travelled since the last dab.
    carry: f32,
}

struct Block {
    /// The part of the image this block covers (a whole block, cut at the image's edge).
    region: Region,
    before: Vec<u8>,
    coverage: Vec<u8>,
}

fn union(a: Option<Region>, b: Region) -> Region {
    match a {
        None => b,
        Some(a) => {
            let x = a.x.min(b.x);
            let y = a.y.min(b.y);
            Region::new(x, y, (a.x + a.w).max(b.x + b.w) - x, (a.y + a.h).max(b.y + b.h) - y)
        }
    }
}

impl Stroke {
    pub fn new(image: &Image, brush: Brush) -> Stroke {
        let _ = image;
        Stroke { brush, patches: BTreeMap::new(), dirty: None, last: None, carry: 0.0 }
    }

    /// Everything the stroke has changed so far, in image pixels.
    pub fn dirty(&self) -> Option<Region> {
        self.dirty
    }

    /// The blocks of the image the stroke has touched, each cut to the image.
    pub fn patches(&self, image: &Image) -> Vec<Region> {
        let _ = image;
        self.patches.values().map(|block| block.region).collect()
    }

    /// The blocks of the image the stroke has touched, each with its pixels as they were before the stroke, for undo.
    pub fn before(&self) -> Vec<(Region, Vec<u8>)> {
        self.patches.values().map(|block| (block.region, block.before.clone())).collect()
    }

    /// Extend the stroke to (x, y) in image pixels. `pressure` (0..1) scales
    /// the brush diameter. Returns the area changed by this call.
    pub fn line_to(&mut self, image: &mut Image, x: f32, y: f32, pressure: f32) -> Option<Region> {
        let radius = (self.brush.size * pressure.clamp(0.0, 1.0) / 2.0).max(0.5);
        let step = (self.brush.size * self.brush.spacing).max(1.0);
        let mut changed = None;
        match self.last {
            None => changed = self.dab(image, x, y, radius).map(|r| union(changed, r)),
            Some((lx, ly)) => {
                let (dx, dy) = (x - lx, y - ly);
                let length = dx.hypot(dy);
                let mut travelled = step - self.carry;
                while travelled <= length {
                    let t = travelled / length;
                    if let Some(r) = self.dab(image, lx + dx * t, ly + dy * t, radius) {
                        changed = Some(union(changed, r));
                    }
                    travelled += step;
                }
                self.carry = length - (travelled - step);
            }
        }
        self.last = Some((x, y));
        if let Some(r) = changed {
            self.dirty = Some(union(self.dirty, r));
        }
        changed
    }

    fn dab(&mut self, image: &mut Image, cx: f32, cy: f32, radius: f32) -> Option<Region> {
        let area = Region::new((cx - radius - 1.0).floor() as i32, (cy - radius - 1.0).floor() as i32, (radius * 2.0 + 3.0).ceil() as i32, (radius * 2.0 + 3.0).ceil() as i32)
            .intersect(&Region::new(0, 0, image.w, image.h))?;
        let whole = Region::new(0, 0, image.w, image.h);
        let inner = radius * self.brush.hardness.clamp(0.0, 1.0);
        let fade = radius - inner;
        let opacity = self.brush.opacity.clamp(0.0, 1.0);
        let color = self.brush.color.map(|c| c as f32);
        let stride = image.w as usize;
        for py in area.y.div_euclid(PATCH)..=(area.y + area.h - 1).div_euclid(PATCH) {
            for px in area.x.div_euclid(PATCH)..=(area.x + area.w - 1).div_euclid(PATCH) {
                let block = self.patches.entry((px, py)).or_insert_with(|| {
                    // First touch: remember this block as it is, before anything is painted on it.
                    let region = Region::new(px * PATCH, py * PATCH, PATCH, PATCH).intersect(&whole).expect("a dab is inside the image");
                    Block { region, before: crop(&image.data, image.w, &region), coverage: vec![0; region.area()] }
                });
                let Some(part) = area.intersect(&block.region) else { continue };
                for y in part.y..part.y + part.h {
                    let dy = y as f32 + 0.5 - cy;
                    for x in part.x..part.x + part.w {
                        let dx = x as f32 + 0.5 - cx;
                        let d = (dx * dx + dy * dy).sqrt();
                        // Anti-aliased disc, optionally with a smooth falloff outside the hard core.
                        let mut c = (radius - d + 0.5).clamp(0.0, 1.0);
                        if fade > 0.0 && d > inner {
                            let t = ((d - inner) / fade).min(1.0);
                            c = c.min(1.0 - t * t * (3.0 - 2.0 * t));
                        }
                        let c8 = (c * 255.0 + 0.5) as u8;
                        let b = block.region.index(x, y);
                        if c8 <= block.coverage[b] {
                            continue;
                        }
                        block.coverage[b] = c8;
                        let a = c8 as f32 / 255.0 * opacity;
                        let before = &block.before[b * 4..b * 4 + 4];
                        let i = (y as usize * stride + x as usize) * 4;
                        let px = &mut image.data[i..i + 4];
                        if self.brush.erase {
                            px[3] = (before[3] as f32 * (1.0 - a) + 0.5) as u8;
                        } else {
                            // Straight-alpha source-over of the brush color onto the original pixel.
                            let keep = before[3] as f32 / 255.0 * (1.0 - a);
                            let out = a + keep;
                            if out > 0.0 {
                                for k in 0..3 {
                                    px[k] = ((color[k] * a + before[k] as f32 * keep) / out + 0.5) as u8;
                                }
                            }
                            px[3] = (out * 255.0 + 0.5) as u8;
                        }
                    }
                }
            }
        }
        Some(area)
    }
}

/// Copy a rectangle of pixels out of an image.
pub fn crop(data: &[u8], stride: i32, region: &Region) -> Vec<u8> {
    let mut out = Vec::with_capacity(region.area() * 4);
    for y in region.y..region.y + region.h {
        let start = (y as usize * stride as usize + region.x as usize) * 4;
        out.extend_from_slice(&data[start..start + region.w as usize * 4]);
    }
    out
}

/// Write a rectangle of pixels, as produced by `crop`, back into an image.
pub fn paste(image: &mut Image, region: &Region, pixels: &[u8]) {
    let width = region.w as usize * 4;
    for (row, y) in (region.y..region.y + region.h).enumerate() {
        let start = (y as usize * image.w as usize + region.x as usize) * 4;
        image.data[start..start + width].copy_from_slice(&pixels[row * width..(row + 1) * width]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank(w: i32, h: i32, rgba: [u8; 4]) -> Image {
        Image { w, h, data: rgba.repeat((w * h) as usize) }
    }
    fn px(image: &Image, x: i32, y: i32) -> [u8; 4] {
        let i = ((y * image.w + x) * 4) as usize;
        image.data[i..i + 4].try_into().unwrap()
    }
    fn hard(size: f32) -> Brush {
        Brush { size, hardness: 1.0, color: [255, 0, 0], ..Brush::default() }
    }

    #[test]
    fn a_single_dab_is_a_disc_with_an_anti_aliased_edge() {
        let mut image = blank(40, 40, [0, 0, 0, 0]);
        let mut stroke = Stroke::new(&image, hard(20.0));
        // Centered on the middle of pixel (20,20), so distances along the row are whole numbers.
        let dirty = stroke.line_to(&mut image, 20.5, 20.5, 1.0).unwrap();
        assert_eq!(px(&image, 20, 20), [255, 0, 0, 255]);
        assert_eq!(px(&image, 29, 20), [255, 0, 0, 255]); // 9 px from the center, inside the 10 px radius
        assert_eq!(px(&image, 30, 20)[3], 128); // exactly on the edge: half covered
        assert_eq!(px(&image, 31, 20)[3], 0);
        assert_eq!(px(&image, 10, 20)[3], 128); // symmetric
        assert!(dirty.w <= 24 && dirty.x >= 8);
    }

    #[test]
    fn a_line_is_continuous_and_reports_what_it_touched() {
        let mut image = blank(100, 40, [0, 0, 0, 0]);
        let mut stroke = Stroke::new(&image, hard(10.0));
        stroke.line_to(&mut image, 10.0, 20.0, 1.0);
        let dirty = stroke.line_to(&mut image, 90.0, 20.0, 1.0).unwrap();
        for x in 10..90 {
            assert_eq!(px(&image, x, 20), [255, 0, 0, 255], "gap at x={x}");
        }
        assert_eq!(px(&image, 50, 30)[3], 0);
        assert!(dirty.x <= 12 && dirty.x + dirty.w >= 94);
        assert_eq!(stroke.dirty().unwrap().y, 14);
    }

    #[test]
    fn opacity_applies_to_the_whole_stroke_and_does_not_build_up() {
        let mut image = blank(60, 20, [255, 255, 255, 255]);
        let mut stroke = Stroke::new(&image, Brush { opacity: 0.5, color: [0, 0, 0], ..hard(10.0) });
        // Back and forth over the same pixels.
        for x in [10.0, 50.0, 10.0, 50.0] {
            stroke.line_to(&mut image, x, 10.0, 1.0);
        }
        assert_eq!(px(&image, 30, 10), [128, 128, 128, 255]);
    }

    #[test]
    fn painting_over_transparency_uses_straight_alpha() {
        let mut image = blank(20, 20, [0, 0, 255, 128]);
        let mut stroke = Stroke::new(&image, Brush { opacity: 0.5, ..hard(10.0) });
        stroke.line_to(&mut image, 10.0, 10.0, 1.0);
        let [r, _, b, a] = px(&image, 10, 10);
        // alpha = .5 + .502·.5 ≈ .751; red share .5/.751, blue share .251/.751
        assert_eq!(a, 192);
        assert!(r.abs_diff(170) <= 1 && b.abs_diff(85) <= 1, "{r} {b}");
    }

    #[test]
    fn the_eraser_reduces_alpha_and_keeps_color() {
        let mut image = blank(40, 20, [10, 20, 30, 200]);
        let mut stroke = Stroke::new(&image, Brush { erase: true, opacity: 0.5, ..hard(10.0) });
        stroke.line_to(&mut image, 10.0, 10.0, 1.0);
        assert_eq!(px(&image, 10, 10), [10, 20, 30, 100]);
        assert_eq!(px(&image, 30, 10), [10, 20, 30, 200]);
        let mut full = Stroke::new(&image, Brush { erase: true, ..hard(10.0) });
        full.line_to(&mut image, 30.0, 10.0, 1.0);
        assert_eq!(px(&image, 30, 10)[3], 0);
    }

    #[test]
    fn a_soft_brush_fades_toward_its_edge_and_pressure_shrinks_it() {
        let mut image = blank(60, 60, [0, 0, 0, 0]);
        let mut stroke = Stroke::new(&image, Brush { size: 40.0, hardness: 0.0, color: [255, 0, 0], ..Brush::default() });
        stroke.line_to(&mut image, 30.0, 30.0, 1.0);
        let a: Vec<u8> = [30, 35, 40, 45, 49].iter().map(|&x| px(&image, x, 30)[3]).collect();
        assert!(a.windows(2).all(|w| w[0] > w[1]), "{a:?}");
        assert!(a[0] > 250 && a[4] < 20);

        let mut small = blank(60, 60, [0, 0, 0, 0]);
        Stroke::new(&small, hard(40.0)).line_to(&mut small, 30.0, 30.0, 0.5);
        assert_eq!(px(&small, 38, 30)[3], 255); // inside the halved 10 px radius
        assert_eq!(px(&small, 45, 30)[3], 0); // outside it, though inside the full radius
    }

    #[test]
    fn strokes_clip_to_the_image_and_can_be_undone_from_a_crop() {
        let mut image = blank(30, 30, [9, 9, 9, 255]);
        let original = image.data.clone();
        let mut stroke = Stroke::new(&image, hard(20.0));
        assert!(stroke.line_to(&mut image, 100.0, 100.0, 1.0).is_none()); // entirely outside
        stroke.line_to(&mut image, 100.0, 100.0, 1.0);
        stroke.line_to(&mut image, 0.0, 0.0, 1.0); // comes in from outside
        let dirty = stroke.dirty().unwrap();
        assert!(dirty.x >= 0 && dirty.y >= 0 && dirty.x + dirty.w <= 30 && dirty.y + dirty.h <= 30);
        assert_ne!(image.data, original);
        // The touched blocks, each kept as it was, put everything back; and they are the blocks reported.
        let kept = stroke.before();
        assert_eq!(kept.iter().map(|(region, _)| *region).collect::<Vec<_>>(), stroke.patches(&image));
        for (patch, before) in kept {
            paste(&mut image, &patch, &before);
        }
        assert_eq!(image.data, original);
    }
}
