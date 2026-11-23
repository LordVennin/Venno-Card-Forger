//! Layer effects as coverage maps.
//!
//! Each effect turns the layer's alpha into one or more "paint this ink
//! through this coverage" stamps. Only the seven effect types found by the
//! survey exist here.

use crate::doc::{BevelStyle, BlendMode, Color, Contour, Direction, Effect, GradientStyle, StrokePosition, Tone};
use crate::field::{blur, cover, signed_distance};
use crate::raster::{GradientInk, Ink, Region};
use std::borrow::Cow;
use std::cell::OnceCell;

/// A layer's alpha over a rectangular region of the document.
pub struct Shape {
    /// May extend past the document: outside it, alpha is zero.
    pub region: Region,
    pub alpha: Vec<f32>,
    /// Tight bounds of the non-transparent pixels.
    pub bounds: Region,
}

pub struct Stamp<'a> {
    pub coverage: Cow<'a, [f32]>,
    pub ink: Ink,
    pub blend: BlendMode,
    pub opacity: f32,
}

/// Blur sigma for a Photoshop "size" in pixels. Photoshop's falloff reaches
/// zero at about `size`; a Gaussian of size/2 is the usual match. Calibrated
/// against golden references in Phase 2.
const SIGMA_PER_SIZE: f32 = 0.5;

/// How far outside the layer's alpha an effect can paint, in pixels.
pub fn effect_reach(effect: &Effect) -> i32 {
    let reach = match effect {
        Effect::DropShadow { distance, size, .. } => distance + size * 1.6 + 2.0,
        Effect::OuterGlow { size, .. } => size * 1.6 + 2.0,
        Effect::Stroke { size, position, .. } if *position != StrokePosition::Inside => size + 2.0,
        Effect::Bevel { style: BevelStyle::Outer, size, soften, .. } => size + soften * 1.6 + 2.0,
        _ => 0.0,
    };
    reach.ceil() as i32
}

/// How far an effect can be influenced by, or paint away from, a change in the
/// layer's alpha, in any direction. A region re-render must see this much of
/// the layer around the region it redraws.
pub fn effect_extent(effect: &Effect) -> i32 {
    let extent = match effect {
        Effect::DropShadow { distance, size, .. } | Effect::InnerShadow { distance, size, .. } => distance + size * 1.6,
        // Blurred effects have a tail: three box passes reach 1.5 × size.
        Effect::OuterGlow { size, .. } => size * 1.6,
        Effect::Stroke { size, .. } => *size,
        Effect::Bevel { size, soften, .. } => size + soften * 1.6,
        Effect::ColorOverlay { .. } | Effect::GradientOverlay { .. } => 0.0,
    };
    extent.ceil() as i32 + 2
}

/// Unit vector pointing from the surface toward the light, in y-down pixel space.
fn light_dir(angle: f64) -> (f64, f64) {
    let r = angle.to_radians();
    (r.cos(), -r.sin())
}

/// Whole-pixel offset of a shadow cast `distance` away from the light. Kept in
/// `f64`: narrowing first can turn 3.4999999 into 3.5 and move the shadow a pixel.
fn cast_offset(angle: f64, distance: f64) -> (i32, i32) {
    let (lx, ly) = light_dir(angle);
    ((-lx * distance).round() as i32, (-ly * distance).round() as i32)
}

fn solid(color: &Color) -> Ink {
    Ink::Solid([(color[0] * 255.0) as f32, (color[1] * 255.0) as f32, (color[2] * 255.0) as f32, color[3] as f32])
}

/// Sample `src` shifted by a whole-pixel offset; out-of-range reads give `fill`.
fn shifted(src: &[f32], w: usize, h: usize, dx: i32, dy: i32, fill: f32) -> Cow<'_, [f32]> {
    if dx == 0 && dy == 0 {
        return Cow::Borrowed(src);
    }
    let mut out = vec![fill; src.len()];
    for y in 0..h as i32 {
        let sy = y - dy;
        if sy < 0 || sy >= h as i32 {
            continue;
        }
        for x in 0..w as i32 {
            let sx = x - dx;
            if sx >= 0 && sx < w as i32 {
                out[y as usize * w + x as usize] = src[sy as usize * w + sx as usize];
            }
        }
    }
    Cow::Owned(out)
}

pub struct EffectContext<'a> {
    shape: &'a Shape,
    sd: OnceCell<Vec<f32>>,
}

impl<'a> EffectContext<'a> {
    pub fn new(shape: &'a Shape) -> Self {
        EffectContext { shape, sd: OnceCell::new() }
    }

    /// Signed distance to the layer's edge. Computed on first use.
    fn sd(&self) -> &[f32] {
        self.sd.get_or_init(|| signed_distance(&self.shape.alpha, self.shape.region.w as usize, self.shape.region.h as usize))
    }

    fn size(&self) -> (usize, usize) {
        (self.shape.region.w as usize, self.shape.region.h as usize)
    }

    /// Shadow or glow cast outward, hidden where the layer itself is.
    #[allow(clippy::too_many_arguments)]
    fn outer(&self, color: &Color, blend: BlendMode, opacity: f64, size: f64, spread: f64, dx: i32, dy: i32, gaussian: bool) -> Stamp<'a> {
        let (w, h) = self.size();
        let alpha = &self.shape.alpha;
        let grow = (spread * size) as f32;
        let matte: Cow<[f32]> = if grow > 0.0 { Cow::Owned(self.sd().iter().map(|&d| cover(d - grow)).collect()) } else { Cow::Borrowed(alpha) };
        let soft = blur(&shifted(&matte, w, h, dx, dy, 0.0), w, h, (size as f32 - grow) * SIGMA_PER_SIZE);
        // The Gaussian contour remaps the falloff through an S-curve: fainter at the rim, denser near the layer.
        let curve = |v: f32| if gaussian { v * v * (3.0 - 2.0 * v) } else { v };
        let coverage = soft.iter().zip(alpha).map(|(&s, &a)| curve(s) * (1.0 - a)).collect();
        Stamp { coverage: Cow::Owned(coverage), ink: solid(color), blend, opacity: opacity as f32 }
    }

    pub fn stamps(&self, effect: &Effect) -> Vec<Stamp<'a>> {
        let (w, h) = self.size();
        let alpha: &'a [f32] = &self.shape.alpha;
        let bounds = self.shape.bounds;
        match effect {
            Effect::ColorOverlay { blend, opacity, color, .. } => {
                vec![Stamp { coverage: Cow::Borrowed(alpha), ink: solid(color), blend: *blend, opacity: *opacity as f32 }]
            }

            Effect::GradientOverlay { blend, opacity, gradient, style, angle, scale, reverse, .. } => {
                let (lx, ly) = light_dir(*angle);
                let cx = bounds.x as f64 + bounds.w as f64 / 2.0;
                let cy = bounds.y as f64 + bounds.h as f64 / 2.0;
                // The gradient line crosses the bounds' center and spans their projection onto its direction.
                let half = ((bounds.w as f64 * lx).abs() + (bounds.h as f64 * ly).abs()) / 2.0 * scale;
                let radial = *style == GradientStyle::Radial;
                let end = (cx + lx * half, cy + ly * half);
                let start = if radial { (cx, cy) } else { (cx - lx * half, cy - ly * half) };
                let stops: Vec<(f64, [f64; 4])> = gradient
                    .stops
                    .iter()
                    .map(|s| (if *reverse == Some(true) { 1.0 - s.offset } else { s.offset }, s.color))
                    .collect();
                if stops.is_empty() {
                    return Vec::new();
                }
                let ink = Ink::Gradient(GradientInk::new(&stops, radial, start, end));
                vec![Stamp { coverage: Cow::Borrowed(alpha), ink, blend: *blend, opacity: *opacity as f32 }]
            }

            Effect::Stroke { blend, opacity, color, size, position, .. } => {
                let s = *size as f32;
                let (outer, inner) = match position {
                    StrokePosition::Outside => (s, 0.0),
                    StrokePosition::Inside => (0.0, s),
                    StrokePosition::Center => (s / 2.0, s / 2.0),
                };
                let coverage = self.sd().iter().map(|&d| (cover(d - outer) - cover(d + inner)).max(0.0)).collect();
                vec![Stamp { coverage: Cow::Owned(coverage), ink: solid(color), blend: *blend, opacity: *opacity as f32 }]
            }

            Effect::DropShadow { blend, opacity, color, angle, distance, spread, size, contour, .. } => {
                let (dx, dy) = cast_offset(*angle, *distance);
                vec![self.outer(color, *blend, *opacity, *size, *spread, dx, dy, *contour == Some(Contour::Gaussian))]
            }

            Effect::OuterGlow { blend, opacity, color, spread, size, .. } => {
                vec![self.outer(color, *blend, *opacity, *size, *spread, 0, 0, false)]
            }

            Effect::InnerShadow { blend, opacity, color, angle, distance, choke, size, .. } => {
                let (dx, dy) = cast_offset(*angle, *distance);
                let shrink = (choke * size) as f32;
                // The shadow is cast by everything that is NOT the layer, so it is the inverse matte, grown by the choke.
                let matte: Vec<f32> = if shrink > 0.0 { self.sd().iter().map(|&d| cover(-d - shrink)).collect() } else { alpha.iter().map(|&a| 1.0 - a).collect() };
                let soft = blur(&shifted(&matte, w, h, dx, dy, 1.0), w, h, (*size as f32 - shrink) * SIGMA_PER_SIZE);
                let coverage = soft.iter().zip(alpha).map(|(&s, &a)| s * a).collect();
                vec![Stamp { coverage: Cow::Owned(coverage), ink: solid(color), blend: *blend, opacity: *opacity as f32 }]
            }

            Effect::Bevel { style, direction, depth, size, soften, angle, altitude, highlight, shadow, .. } => {
                let size = (*size as f32).max(1e-3);
                let inner = *style == BevelStyle::Inner;
                let sd = self.sd();
                let (lx, ly) = light_dir(*angle);
                let (lx, ly) = (lx as f32, ly as f32);
                let alt = (*altitude as f32).to_radians();
                let light = [alt.cos() * lx, alt.cos() * ly, alt.sin()];
                let flat = light[2];
                let slope = *depth as f32 * if *direction == Direction::Down { -1.0 } else { 1.0 };
                let mut hi = vec![0f32; alpha.len()];
                let mut lo = vec![0f32; alpha.len()];
                for y in 0..h {
                    for x in 0..w {
                        let i = y * w + x;
                        // Chisel hard: the surface is a straight ramp of width `size` rising into the
                        // shape. `band` is the ramp's footprint, anti-aliased at both ends.
                        let band = if inner { cover(sd[i]) - cover(sd[i] + size) } else { cover(sd[i] - size) - cover(sd[i]) };
                        if band <= 0.0 {
                            continue;
                        }
                        // Height falls as distance grows, so the normal leans along +grad(sd).
                        let nx = (sd[y * w + (x + 1).min(w - 1)] - sd[y * w + x.saturating_sub(1)]) / 2.0 * slope;
                        let ny = (sd[(y + 1).min(h - 1) * w + x] - sd[y.saturating_sub(1) * w + x]) / 2.0 * slope;
                        let lit = (nx * light[0] + ny * light[1] + light[2]) / (nx * nx + ny * ny + 1.0).sqrt();
                        if lit > flat {
                            hi[i] = band * ((lit - flat) / (1.0 - flat).max(1e-3)).min(1.0);
                        } else {
                            lo[i] = band * ((flat - lit) / flat.max(1e-3)).min(1.0);
                        }
                    }
                }
                let sigma = *soften as f32 * SIGMA_PER_SIZE;
                let within = |i: usize| if inner { alpha[i] } else { 1.0 - alpha[i] };
                let tone = |map: Vec<f32>, tone: &Tone| -> Stamp<'a> {
                    let soft = blur(&map, w, h, sigma);
                    let coverage = soft.iter().enumerate().map(|(i, &v)| v * within(i)).collect();
                    Stamp { coverage: Cow::Owned(coverage), ink: solid(&tone.color), blend: tone.blend, opacity: tone.opacity as f32 }
                };
                vec![tone(lo, shadow), tone(hi, highlight)]
            }
        }
    }
}

/// Position of an effect in Photoshop's stacking, bottom to top. Negative
/// values are painted under the layer's own pixels, the rest over them.
fn rank(effect: &Effect) -> i32 {
    match effect {
        Effect::DropShadow { .. } => -2,
        Effect::OuterGlow { .. } => -1,
        Effect::GradientOverlay { .. } => 1,
        Effect::ColorOverlay { .. } => 2,
        Effect::InnerShadow { .. } => 3,
        Effect::Bevel { .. } => 4,
        Effect::Stroke { .. } => 5,
    }
}

/// Split enabled effects into those painted below the layer and those painted
/// above, each in paint order. Within a type, the first listed ends up on top.
pub fn ordered<'e>(effects: &[&'e Effect]) -> (Vec<&'e Effect>, Vec<&'e Effect>) {
    let mut all: Vec<&Effect> = effects.iter().rev().copied().collect();
    all.sort_by_key(|e| rank(e)); // stable: keeps the reversed order within a type
    let above = all.split_off(all.iter().take_while(|e| rank(e) < 0).count());
    (all, above)
}
