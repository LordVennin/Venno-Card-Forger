//! The compositor: blending, masking and resampling over plain byte buffers.
//!
//! Pixels are straight (non-premultiplied) RGBA, 8 bits per channel, as in
//! Photoshop. Everything here is deterministic integer and `f32` math with no
//! platform dependencies, so native and WebAssembly builds agree byte for byte.

use crate::doc::BlendMode;

/// A rectangle in document pixels. Width and height are never negative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Region {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Region {
        Region { x, y, w: w.max(0), h: h.max(0) }
    }

    pub fn area(&self) -> usize {
        self.w as usize * self.h as usize
    }

    pub fn intersect(&self, other: &Region) -> Option<Region> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let w = (self.x + self.w).min(other.x + other.w) - x;
        let h = (self.y + self.h).min(other.y + other.h) - y;
        (w > 0 && h > 0).then_some(Region { x, y, w, h })
    }

    /// Index of document pixel (x, y) in a row-major buffer covering this region.
    #[inline]
    pub fn index(&self, x: i32, y: i32) -> usize {
        (y - self.y) as usize * self.w as usize + (x - self.x) as usize
    }
}

/// A rectangle of straight-alpha RGBA pixels placed in document space.
#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    pub region: Region,
    pub data: Vec<u8>,
}

impl Raster {
    pub fn new(region: Region) -> Raster {
        Raster { region, data: vec![0; region.area() * 4] }
    }

    pub fn pixel(&self, x: i32, y: i32) -> [u8; 4] {
        let i = self.region.index(x, y) * 4;
        [self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]]
    }
}

/// A decoded image: straight RGBA, row-major.
#[derive(Clone)]
pub struct Image {
    pub w: i32,
    pub h: i32,
    pub data: Vec<u8>,
}

#[inline]
fn to_u8(v: f32) -> u8 {
    // `as` saturates, so this also clamps.
    (v + 0.5) as u8
}

fn soft_d(d: f32) -> f32 {
    if d <= 0.25 {
        ((16.0 * d - 12.0) * d + 4.0) * d
    } else {
        d.sqrt()
    }
}

/// Separable blend functions on 0..1 values (the W3C/PDF definitions, which are also Photoshop's).
#[inline]
fn blend_channel(mode: BlendMode, s: f32, d: f32) -> f32 {
    match mode {
        BlendMode::Multiply => s * d,
        BlendMode::Screen => s + d - s * d,
        BlendMode::Overlay => {
            if d <= 0.5 {
                2.0 * s * d
            } else {
                1.0 - 2.0 * (1.0 - s) * (1.0 - d)
            }
        }
        BlendMode::SoftLight => {
            if s <= 0.5 {
                d - (1.0 - 2.0 * s) * d * (1.0 - d)
            } else {
                d + (2.0 * s - 1.0) * (soft_d(d) - d)
            }
        }
        BlendMode::LinearLight => (d + 2.0 * s - 1.0).clamp(0.0, 1.0),
        BlendMode::Normal | BlendMode::Luminosity => s,
    }
}

#[inline]
fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

/// The blended color B(source, backdrop), all 0..1.
#[inline]
fn blend_color(mode: BlendMode, s: [f32; 3], d: [f32; 3]) -> [f32; 3] {
    if mode != BlendMode::Luminosity {
        return [blend_channel(mode, s[0], d[0]), blend_channel(mode, s[1], d[1]), blend_channel(mode, s[2], d[2])];
    }
    // Luminosity: the backdrop's hue and saturation with the source's lightness.
    let shift = lum(s) - lum(d);
    let mut c = [d[0] + shift, d[1] + shift, d[2] + shift];
    let l = lum(c);
    let lo = c[0].min(c[1]).min(c[2]);
    let hi = c[0].max(c[1]).max(c[2]);
    if lo < 0.0 {
        c = c.map(|v| l + (v - l) * l / (l - lo));
    }
    if hi > 1.0 {
        c = c.map(|v| l + (v - l) * (1.0 - l) / (hi - l));
    }
    c
}

/// Composite one source pixel onto `d` (4 bytes). Source color is 0..255,
/// source alpha `sa` is 0..1. With `atop` the destination's alpha is left
/// exactly as it was (clipped layers).
#[inline]
fn mix(d: &mut [u8], s: [f32; 3], sa: f32, mode: BlendMode, atop: bool) {
    let da8 = d[3];
    if da8 == 0 {
        // Nothing to blend with: a clipped layer vanishes, anything else lands as-is.
        if !atop {
            d[0] = to_u8(s[0]);
            d[1] = to_u8(s[1]);
            d[2] = to_u8(s[2]);
            d[3] = to_u8(sa * 255.0);
        }
        return;
    }
    let dc = [d[0] as f32, d[1] as f32, d[2] as f32];
    let b = if mode == BlendMode::Normal {
        s
    } else {
        blend_color(mode, s.map(|v| v / 255.0), dc.map(|v| v / 255.0)).map(|v| v * 255.0)
    };
    if atop || da8 == 255 {
        for c in 0..3 {
            d[c] = to_u8(dc[c] + (b[c] - dc[c]) * sa);
        }
        return;
    }
    // Where the backdrop is translucent, the uncovered share of the source shows unblended.
    let da = da8 as f32 / 255.0;
    let both = sa * da;
    let src_only = sa - both;
    let dst_only = da - both;
    let oa = sa + dst_only;
    for c in 0..3 {
        d[c] = to_u8((src_only * s[c] + both * b[c] + dst_only * dc[c]) / oa);
    }
    d[3] = to_u8(oa * 255.0);
}

/// Composite `src` onto `dst` where they overlap.
pub fn composite(dst: &mut Raster, src: &Raster, opacity: f32, blend: BlendMode, atop: bool) {
    let Some(area) = dst.region.intersect(&src.region) else { return };
    if opacity <= 0.0 {
        return;
    }
    let k = opacity / 255.0;
    let width = area.w as usize * 4;
    for y in area.y..area.y + area.h {
        let di = dst.region.index(area.x, y) * 4;
        let si = src.region.index(area.x, y) * 4;
        let d_row = &mut dst.data[di..di + width];
        let s_row = &src.data[si..si + width];
        for (d, s) in d_row.chunks_exact_mut(4).zip(s_row.chunks_exact(4)) {
            if s[3] != 0 {
                mix(d, [s[0] as f32, s[1] as f32, s[2] as f32], s[3] as f32 * k, blend, atop);
            }
        }
    }
}

/// What a stamp paints with: one color (0..255, alpha 0..1), or a gradient.
#[derive(Clone)]
pub enum Ink {
    Solid([f32; 4]),
    Gradient(std::rc::Rc<GradientInk>),
}

/// How much of each pixel a stamp covers.
pub enum Cover<'a> {
    /// One byte per pixel of the stamp's region, 0..255.
    Map(&'a [u8]),
    /// Every pixel fully.
    Full,
}

/// A gradient sampled into a lookup table. Stops interpolate in straight sRGB,
/// as Photoshop's classic mode does.
pub struct GradientInk {
    lut: Vec<[f32; 4]>,
    start: (f32, f32),
    delta: (f32, f32),
    len2: f32,
    radial: bool,
}

impl GradientInk {
    const STEPS: usize = 1024;

    /// `stops` are (offset, straight RGBA 0..1). `start` and `end` are in document pixels.
    pub fn new(stops: &[(f64, [f64; 4])], radial: bool, start: (f64, f64), end: (f64, f64)) -> GradientInk {
        let mut stops = stops.to_vec();
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut lut = Vec::with_capacity(Self::STEPS + 1);
        let mut s = 0;
        for i in 0..=Self::STEPS {
            let t = i as f64 / Self::STEPS as f64;
            while s + 2 < stops.len() && t > stops[s + 1].0 {
                s += 1;
            }
            let from = stops[s];
            let to = stops[(s + 1).min(stops.len() - 1)];
            let span = to.0 - from.0;
            let f = if span > 0.0 {
                ((t - from.0) / span).clamp(0.0, 1.0)
            } else if t < from.0 {
                0.0
            } else {
                1.0
            };
            let c = |k: usize| from.1[k] + (to.1[k] - from.1[k]) * f;
            lut.push([(c(0) * 255.0) as f32, (c(1) * 255.0) as f32, (c(2) * 255.0) as f32, c(3) as f32]);
        }
        let delta = ((end.0 - start.0) as f32, (end.1 - start.1) as f32);
        let len2 = delta.0 * delta.0 + delta.1 * delta.1;
        GradientInk { lut, start: (start.0 as f32, start.1 as f32), delta, len2: if len2 > 0.0 { len2 } else { 1.0 }, radial }
    }

    #[inline]
    fn at(&self, x: i32, y: i32) -> [f32; 4] {
        let px = x as f32 + 0.5 - self.start.0;
        let py = y as f32 + 0.5 - self.start.1;
        let t = if self.radial { ((px * px + py * py) / self.len2).sqrt() } else { (px * self.delta.0 + py * self.delta.1) / self.len2 };
        self.lut[(t.clamp(0.0, 1.0) * Self::STEPS as f32 + 0.5) as usize]
    }
}

/// Paint `ink` onto `dst` through a coverage map laid over `region`.
pub fn stamp(dst: &mut Raster, region: &Region, coverage: &Cover, ink: &Ink, opacity: f32, blend: BlendMode) {
    let Some(area) = dst.region.intersect(region) else { return };
    if opacity <= 0.0 {
        return;
    }
    for y in area.y..area.y + area.h {
        let mut di = dst.region.index(area.x, y) * 4;
        let mut ci = region.index(area.x, y);
        for x in area.x..area.x + area.w {
            let cov = match coverage {
                Cover::Map(map) => map[ci],
                Cover::Full => 255,
            };
            if cov != 0 {
                let c = match ink {
                    Ink::Solid(c) => *c,
                    Ink::Gradient(g) => g.at(x, y),
                };
                let a = cov as f32 / 255.0 * c[3] * opacity;
                if a > 0.0 {
                    mix(&mut dst.data[di..di + 4], [c[0], c[1], c[2]], a, blend, false);
                }
            }
            di += 4;
            ci += 1;
        }
    }
}

/// `dst = dst·(1 − m) + src·m`, where `m = opacity × coverage`. Both rasters
/// must cover the same region. `coverage` is 0..255 per pixel, or `None` for
/// "everywhere".
pub fn cross_fade(dst: &mut Raster, src: &Raster, opacity: f32, coverage: Option<&[u8]>) {
    debug_assert_eq!(dst.region, src.region);
    for (p, (d, s)) in dst.data.chunks_exact_mut(4).zip(src.data.chunks_exact(4)).enumerate() {
        if d == s {
            continue;
        }
        let m = match coverage {
            Some(c) => opacity * c[p] as f32 / 255.0,
            None => opacity,
        };
        if m <= 0.0 {
            continue;
        }
        // Interpolate premultiplied values, then go back to straight alpha.
        let da = d[3] as f32 * (1.0 - m);
        let sa = s[3] as f32 * m;
        let oa = da + sa;
        if oa <= 0.0 {
            d[3] = 0;
            continue;
        }
        for c in 0..3 {
            d[c] = to_u8((d[c] as f32 * da + s[c] as f32 * sa) / oa);
        }
        d[3] = to_u8(oa);
    }
}

/// Multiply a raster's alpha by a 0..255 coverage map of the same size.
pub fn multiply_alpha(raster: &mut Raster, coverage: &[u8]) {
    for (px, &c) in raster.data.chunks_exact_mut(4).zip(coverage) {
        if c != 255 {
            px[3] = to_u8(px[3] as f32 * c as f32 / 255.0);
        }
    }
}

/// The part of an image that falls inside `region` when its top-left is at (x, y).
pub fn place_image(image: &Image, x: i32, y: i32, region: &Region) -> Option<Raster> {
    let area = region.intersect(&Region::new(x, y, image.w, image.h))?;
    let mut out = Raster::new(area);
    let width = area.w as usize * 4;
    for row in 0..area.h {
        let from = ((area.y - y + row) as usize * image.w as usize + (area.x - x) as usize) * 4;
        let to = row as usize * width;
        out.data[to..to + width].copy_from_slice(&image.data[from..from + width]);
    }
    Some(out)
}

/// Draw an image through an affine transform `[a, c, tx, b, d, ty]` with
/// bilinear sampling. Texels are clamped at the image border, and the border
/// itself is anti-aliased, so an image scaled by a whole number keeps crisp
/// edges.
pub fn resample(image: &Image, m: &[f64; 6], region: &Region) -> Option<Raster> {
    let [a, c, tx, b, d, ty] = *m;
    let det = a * d - b * c;
    if det.abs() < 1e-12 {
        return None;
    }
    let (iw, ih) = (image.w as f64, image.h as f64);
    let corners = [(0.0, 0.0), (iw, 0.0), (0.0, ih), (iw, ih)];
    let xs = corners.map(|(u, v)| a * u + c * v + tx);
    let ys = corners.map(|(u, v)| b * u + d * v + ty);
    let min = |v: [f64; 4]| v.into_iter().fold(f64::INFINITY, f64::min);
    let max = |v: [f64; 4]| v.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let (x0, y0) = (min(xs).floor() as i32, min(ys).floor() as i32);
    let area = region.intersect(&Region::new(x0, y0, max(xs).ceil() as i32 - x0, max(ys).ceil() as i32 - y0))?;
    let mut out = Raster::new(area);
    let (ia, ic, ib, id) = (d / det, -c / det, -b / det, a / det);
    // Output pixels per texel along each image axis, for edge coverage.
    let (scale_u, scale_v) = (a.hypot(b), c.hypot(d));
    let (max_u, max_v) = (image.w - 1, image.h - 1);
    let stride = image.w as usize;
    for y in 0..area.h {
        for x in 0..area.w {
            // Center of this output pixel in image space, where the image spans [0,W]×[0,H].
            let px = (area.x + x) as f64 + 0.5 - tx;
            let py = (area.y + y) as f64 + 0.5 - ty;
            let (u, v) = (ia * px + ic * py, ib * px + id * py);
            let edge = (u.min(iw - u) * scale_u + 0.5).min(1.0) * (v.min(ih - v) * scale_v + 0.5).min(1.0);
            if edge <= 0.0 {
                continue;
            }
            let (u, v) = (u - 0.5, v - 0.5);
            let (uf, vf) = (u.floor(), v.floor());
            let (fu, fv) = ((u - uf) as f32, (v - vf) as f32);
            let u0 = (uf as i32).clamp(0, max_u) as usize;
            let u1 = (uf as i32 + 1).clamp(0, max_u) as usize;
            let v0 = (vf as i32).clamp(0, max_v) as usize;
            let v1 = (vf as i32 + 1).clamp(0, max_v) as usize;
            // Interpolate premultiplied, so transparent texels do not bleed their color.
            let mut acc = [0f32; 4];
            for (uu, vv, weight) in [(u0, v0, (1.0 - fu) * (1.0 - fv)), (u1, v0, fu * (1.0 - fv)), (u0, v1, (1.0 - fu) * fv), (u1, v1, fu * fv)] {
                if weight <= 0.0 {
                    continue;
                }
                let i = (vv * stride + uu) * 4;
                let alpha = image.data[i + 3] as f32 * weight;
                acc[0] += image.data[i] as f32 * alpha;
                acc[1] += image.data[i + 1] as f32 * alpha;
                acc[2] += image.data[i + 2] as f32 * alpha;
                acc[3] += alpha;
            }
            if acc[3] <= 0.0 {
                continue;
            }
            let o = (y as usize * area.w as usize + x as usize) * 4;
            out.data[o] = to_u8(acc[0] / acc[3]);
            out.data[o + 1] = to_u8(acc[1] / acc[3]);
            out.data[o + 2] = to_u8(acc[2] / acc[3]);
            out.data[o + 3] = to_u8(acc[3] * edge as f32);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use BlendMode::*;

    fn filled(x: i32, y: i32, w: i32, h: i32, rgba: [u8; 4]) -> Raster {
        let mut r = Raster::new(Region::new(x, y, w, h));
        r.data.chunks_exact_mut(4).for_each(|p| p.copy_from_slice(&rgba));
        r
    }

    #[test]
    fn composite_only_touches_the_overlap() {
        let mut dst = filled(10, 10, 4, 4, [0, 0, 0, 255]);
        composite(&mut dst, &filled(12, 8, 10, 4, [255, 0, 0, 255]), 1.0, Normal, false);
        assert_eq!(dst.pixel(12, 10), [255, 0, 0, 255]);
        assert_eq!(dst.pixel(13, 11), [255, 0, 0, 255]);
        assert_eq!(dst.pixel(11, 10), [0, 0, 0, 255]);
        assert_eq!(dst.pixel(12, 12), [0, 0, 0, 255]);
        composite(&mut dst, &filled(100, 100, 2, 2, [255, 0, 0, 255]), 1.0, Normal, false);
    }

    #[test]
    fn normal_over_translucent_is_straight_alpha_source_over() {
        let mut dst = filled(0, 0, 1, 1, [0, 0, 255, 128]);
        composite(&mut dst, &filled(0, 0, 1, 1, [255, 0, 0, 128]), 1.0, Normal, false);
        // With a = 128/255: alpha = a + a·(1 − a) ≈ .752; red ≈ .667, blue ≈ .333
        let [r, g, b, a] = dst.pixel(0, 0);
        assert_eq!([r, g, a], [170, 0, 192]);
        assert!((84..=85).contains(&b));
    }

    #[test]
    fn atop_never_changes_destination_alpha() {
        for mode in [Normal, Multiply, Luminosity] {
            let mut dst = filled(0, 0, 2, 1, [200, 100, 50, 77]);
            dst.data[7] = 0;
            composite(&mut dst, &filled(0, 0, 2, 1, [10, 20, 30, 255]), 0.6, mode, true);
            assert_eq!(dst.pixel(0, 0)[3], 77);
            assert_eq!(dst.pixel(1, 0), [200, 100, 50, 0]);
        }
    }

    #[test]
    fn stamp_paints_through_coverage_clipped_to_the_destination() {
        let mut dst = filled(0, 0, 4, 1, [0, 0, 0, 255]);
        let white = Ink::Solid([255.0, 255.0, 255.0, 1.0]);
        stamp(&mut dst, &Region::new(2, 0, 4, 1), &Cover::Map(&[255, 128, 255, 255]), &white, 1.0, Normal);
        assert_eq!(dst.pixel(1, 0), [0, 0, 0, 255]);
        assert_eq!(dst.pixel(2, 0), [255, 255, 255, 255]);
        assert_eq!(dst.pixel(3, 0), [128, 128, 128, 255]);
        stamp(&mut dst, &Region::new(0, 0, 1, 1), &Cover::Full, &white, 0.5, Normal);
        assert_eq!(dst.pixel(0, 0), [128, 128, 128, 255]);
    }

    #[test]
    fn cross_fade_interpolates_premultiplied() {
        let mut dst = filled(0, 0, 1, 1, [255, 0, 0, 255]);
        cross_fade(&mut dst, &filled(0, 0, 1, 1, [0, 0, 255, 0]), 0.5, None);
        // Fading toward transparent keeps the color and halves the alpha.
        assert_eq!(dst.pixel(0, 0), [255, 0, 0, 128]);
    }

    #[test]
    fn cross_fade_respects_coverage() {
        let mut dst = filled(0, 0, 2, 1, [0, 0, 0, 255]);
        let src = filled(0, 0, 2, 1, [255, 255, 255, 255]);
        cross_fade(&mut dst, &src, 1.0, Some(&[255, 0]));
        assert_eq!(dst.pixel(0, 0), [255, 255, 255, 255]);
        assert_eq!(dst.pixel(1, 0), [0, 0, 0, 255]);
    }

    fn image() -> Image {
        Image { w: 2, h: 2, data: vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255] }
    }
    const WHOLE: Region = Region { x: 0, y: 0, w: 16, h: 16 };

    #[test]
    fn place_image_crops_to_the_region() {
        let r = place_image(&image(), -1, 3, &WHOLE).unwrap();
        assert_eq!(r.region, Region::new(0, 3, 1, 2));
        assert_eq!(r.pixel(0, 3), [0, 255, 0, 255]);
        assert!(place_image(&image(), 40, 40, &WHOLE).is_none());
    }

    #[test]
    fn resample_scales_with_crisp_edges() {
        // Scale ×2, then move to (3, 1): the red texel covers (3..4, 1..2).
        let r = resample(&image(), &[2.0, 0.0, 3.0, 0.0, 2.0, 1.0], &WHOLE).unwrap();
        assert_eq!(r.pixel(3, 1), [255, 0, 0, 255]);
        assert_eq!(r.pixel(6, 4), [255, 255, 255, 255]);
        assert_eq!(r.region, Region::new(3, 1, 4, 4));
    }

    #[test]
    fn resample_at_a_half_pixel_offset_blends_and_softens_the_border() {
        let r = resample(&image(), &[1.0, 0.0, 4.5, 0.0, 1.0, 4.0], &WHOLE).unwrap();
        assert_eq!(r.pixel(4, 4), [255, 0, 0, 128]);
        let [red, green, _, alpha] = r.pixel(5, 4);
        assert_eq!(alpha, 255);
        assert!(red.abs_diff(128) <= 1 && green.abs_diff(128) <= 1);
        assert_eq!(r.pixel(6, 4), [0, 255, 0, 128]);
    }

    #[test]
    fn resample_does_not_let_transparent_texels_tint_neighbours() {
        let fringe = Image { w: 2, h: 1, data: vec![255, 0, 0, 255, 0, 255, 0, 0] };
        let r = resample(&fringe, &[4.0, 0.0, 0.0, 0.0, 4.0, 0.0], &WHOLE).unwrap();
        for x in 0..8 {
            let [red, green, _, alpha] = r.pixel(x, 1);
            if alpha > 0 {
                assert_eq!([red, green], [255, 0]);
            }
        }
    }

    #[test]
    fn resample_rotates_and_refuses_degenerate_transforms() {
        // 90° clockwise about the origin, then moved right by 8: image (u,v) -> (8 - v, u).
        let r = resample(&image(), &[0.0, -1.0, 8.0, 1.0, 0.0, 0.0], &WHOLE).unwrap();
        assert_eq!(r.pixel(7, 0), [255, 0, 0, 255]);
        assert_eq!(r.pixel(7, 1), [0, 255, 0, 255]);
        assert_eq!(r.pixel(6, 0), [0, 0, 255, 255]);
        assert!(resample(&image(), &[0.0; 6], &WHOLE).is_none());
    }

    #[test]
    fn helpers() {
        assert_eq!(Region::new(0, 0, 4, 4).intersect(&Region::new(2, 3, 10, 10)), Some(Region::new(2, 3, 2, 1)));
        assert_eq!(Region::new(0, 0, 4, 4).intersect(&Region::new(4, 0, 1, 1)), None);
        let mut r = filled(0, 0, 2, 1, [9, 8, 7, 200]);
        multiply_alpha(&mut r, &[255, 51]);
        assert_eq!(r.pixel(0, 0), [9, 8, 7, 200]);
        assert_eq!(r.pixel(1, 0), [9, 8, 7, 40]);
    }
}
