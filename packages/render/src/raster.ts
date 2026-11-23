/**
 * Our own compositor: blending, masking and resampling over plain typed arrays.
 *
 * CanvasKit's CPU backend blends at roughly 13 megapixels per second, which is
 * minutes for a 3264×4440 card with dozens of layers. These loops are about ten
 * times faster, give identical bytes in Node and in the app, and can be moved
 * to Rust/WASM later without changing the renderer around them.
 *
 * Pixels are straight (non-premultiplied) RGBA, 8 bits per channel, as in
 * Photoshop. No CanvasKit in this file.
 */
import type { BlendMode } from "@cardsmith/doc";

/** A rectangle of pixels placed in document space. */
export type Raster = { x: number; y: number; w: number; h: number; data: Uint8ClampedArray };
export type Region = { x: number; y: number; w: number; h: number };

export function newRaster(region: Region): Raster {
  return { x: region.x, y: region.y, w: region.w, h: region.h, data: new Uint8ClampedArray(region.w * region.h * 4) };
}

export function copyRaster(src: Raster): Raster {
  return { ...src, data: src.data.slice() };
}

export function intersect(a: Region, b: Region): Region | null {
  const x = Math.max(a.x, b.x), y = Math.max(a.y, b.y);
  const w = Math.min(a.x + a.w, b.x + b.w) - x, h = Math.min(a.y + a.h, b.y + b.h) - y;
  return w > 0 && h > 0 ? { x, y, w, h } : null;
}

const MODES: Record<BlendMode, number> = { normal: 0, multiply: 1, screen: 2, overlay: 3, softLight: 4, linearLight: 5, luminosity: 6 };

function softD(d: number): number {
  return d <= 0.25 ? ((16 * d - 12) * d + 4) * d : Math.sqrt(d);
}

/** Separable blend functions on 0..1 values (the W3C/PDF definitions, which are also Photoshop's). */
function blendChannel(mode: number, s: number, d: number): number {
  switch (mode) {
    case 1:
      return s * d;
    case 2:
      return s + d - s * d;
    case 3:
      return d <= 0.5 ? 2 * s * d : 1 - 2 * (1 - s) * (1 - d);
    case 4:
      return s <= 0.5 ? d - (1 - 2 * s) * d * (1 - d) : d + (2 * s - 1) * (softD(d) - d);
    case 5: {
      const v = d + 2 * s - 1;
      return v < 0 ? 0 : v > 1 ? 1 : v;
    }
    default:
      return s;
  }
}

const lum = (r: number, g: number, b: number) => 0.3 * r + 0.59 * g + 0.11 * b;

// Scratch for the blended color, to avoid allocating per pixel.
const B = new Float64Array(3);

function blendColor(mode: number, sr: number, sg: number, sb: number, dr: number, dg: number, db: number): void {
  if (mode === 6) {
    // Luminosity: the backdrop's hue and saturation with the source's lightness.
    const shift = lum(sr, sg, sb) - lum(dr, dg, db);
    let r = dr + shift, g = dg + shift, b = db + shift;
    const l = lum(r, g, b), lo = Math.min(r, g, b), hi = Math.max(r, g, b);
    if (lo < 0) {
      r = l + ((r - l) * l) / (l - lo);
      g = l + ((g - l) * l) / (l - lo);
      b = l + ((b - l) * l) / (l - lo);
    }
    if (hi > 1) {
      r = l + ((r - l) * (1 - l)) / (hi - l);
      g = l + ((g - l) * (1 - l)) / (hi - l);
      b = l + ((b - l) * (1 - l)) / (hi - l);
    }
    B[0] = r;
    B[1] = g;
    B[2] = b;
    return;
  }
  B[0] = blendChannel(mode, sr, dr);
  B[1] = blendChannel(mode, sg, dg);
  B[2] = blendChannel(mode, sb, db);
}

/**
 * Composite one source pixel onto `d[i..i+3]`.
 * Source color is 0..255, source alpha `sa` is 0..1. With `atop` the
 * destination's alpha is left exactly as it was (clipped layers).
 */
function mix(d: Uint8ClampedArray, i: number, sr: number, sg: number, sb: number, sa: number, mode: number, atop: boolean): void {
  const da8 = d[i + 3]!;
  if (da8 === 0) {
    // Nothing to blend with: a clipped layer vanishes, anything else lands as-is.
    if (atop) return;
    d[i] = sr;
    d[i + 1] = sg;
    d[i + 2] = sb;
    d[i + 3] = sa * 255;
    return;
  }
  if (mode === 0) {
    if (atop || da8 === 255) {
      d[i] = d[i]! + (sr - d[i]!) * sa;
      d[i + 1] = d[i + 1]! + (sg - d[i + 1]!) * sa;
      d[i + 2] = d[i + 2]! + (sb - d[i + 2]!) * sa;
      return;
    }
    const da = da8 / 255, keep = da * (1 - sa), oa = sa + keep;
    d[i] = (sr * sa + d[i]! * keep) / oa;
    d[i + 1] = (sg * sa + d[i + 1]! * keep) / oa;
    d[i + 2] = (sb * sa + d[i + 2]! * keep) / oa;
    d[i + 3] = oa * 255;
    return;
  }
  const dr = d[i]!, dg = d[i + 1]!, db = d[i + 2]!;
  blendColor(mode, sr / 255, sg / 255, sb / 255, dr / 255, dg / 255, db / 255);
  if (atop || da8 === 255) {
    d[i] = dr + (B[0]! * 255 - dr) * sa;
    d[i + 1] = dg + (B[1]! * 255 - dg) * sa;
    d[i + 2] = db + (B[2]! * 255 - db) * sa;
    return;
  }
  // Where the backdrop is translucent, the uncovered share of the source shows unblended.
  const da = da8 / 255, both = sa * da, srcOnly = sa - both, dstOnly = da - both, oa = sa + dstOnly;
  d[i] = (srcOnly * sr + both * B[0]! * 255 + dstOnly * dr) / oa;
  d[i + 1] = (srcOnly * sg + both * B[1]! * 255 + dstOnly * dg) / oa;
  d[i + 2] = (srcOnly * sb + both * B[2]! * 255 + dstOnly * db) / oa;
  d[i + 3] = oa * 255;
}

/** Composite `src` onto `dst` where they overlap. */
export function composite(dst: Raster, src: Raster, opacity: number, blend: BlendMode, atop = false): void {
  const area = intersect(dst, src);
  if (!area || opacity <= 0) return;
  const mode = MODES[blend], d = dst.data, s = src.data, k = opacity / 255;
  for (let y = area.y; y < area.y + area.h; y++) {
    let di = ((y - dst.y) * dst.w + (area.x - dst.x)) * 4;
    let si = ((y - src.y) * src.w + (area.x - src.x)) * 4;
    for (let x = 0; x < area.w; x++, di += 4, si += 4) {
      const a = s[si + 3]!;
      if (a !== 0) mix(d, di, s[si]!, s[si + 1]!, s[si + 2]!, a * k, mode, atop);
    }
  }
}

/** One source color, or a function giving the color (0..255, plus alpha 0..1) at a document pixel. */
export type Ink = { r: number; g: number; b: number; a: number } | ((x: number, y: number, out: Float64Array) => void);

/** Paint `ink` onto `dst` through a coverage map (0..1) laid over `region`. */
export function stamp(dst: Raster, region: Region, coverage: Float32Array, ink: Ink, opacity: number, blend: BlendMode): void {
  const area = intersect(dst, region);
  if (!area || opacity <= 0) return;
  const mode = MODES[blend], d = dst.data;
  const varying = typeof ink === "function";
  const c = new Float64Array(4);
  if (!varying) c.set([ink.r, ink.g, ink.b, ink.a]);
  for (let y = area.y; y < area.y + area.h; y++) {
    let di = ((y - dst.y) * dst.w + (area.x - dst.x)) * 4;
    let ci = (y - region.y) * region.w + (area.x - region.x);
    for (let x = 0; x < area.w; x++, di += 4, ci++) {
      const cov = coverage[ci]!;
      if (cov <= 0) continue;
      if (varying) ink(area.x + x, y, c);
      const a = cov * c[3]! * opacity;
      if (a > 0) mix(d, di, c[0]!, c[1]!, c[2]!, a, mode, false);
    }
  }
}

/**
 * dst = dst·(1 − m) + src·m, where m = opacity × coverage. Both rasters must
 * cover the same region. `coverage` is 0..255 per pixel, or null for "everywhere".
 */
export function crossFade(dst: Raster, src: Raster, opacity: number, coverage: Uint8Array | null): void {
  const d = dst.data, s = src.data;
  for (let p = 0, i = 0; i < d.length; p++, i += 4) {
    if (d[i] === s[i] && d[i + 1] === s[i + 1] && d[i + 2] === s[i + 2] && d[i + 3] === s[i + 3]) continue;
    const m = coverage ? (opacity * coverage[p]!) / 255 : opacity;
    if (m <= 0) continue;
    // Interpolate premultiplied values, then go back to straight alpha.
    const da = d[i + 3]! * (1 - m), sa = s[i + 3]! * m, oa = da + sa;
    if (oa <= 0) {
      d[i + 3] = 0;
      continue;
    }
    d[i] = (d[i]! * da + s[i]! * sa) / oa;
    d[i + 1] = (d[i + 1]! * da + s[i + 1]! * sa) / oa;
    d[i + 2] = (d[i + 2]! * da + s[i + 2]! * sa) / oa;
    d[i + 3] = oa;
  }
}

/** Multiply a raster's alpha by a 0..255 coverage map of the same size. */
export function multiplyAlpha(r: Raster, coverage: Uint8Array): void {
  const d = r.data;
  for (let p = 0, i = 3; p < coverage.length; p++, i += 4) {
    const c = coverage[p]!;
    if (c !== 255) d[i] = (d[i]! * c) / 255;
  }
}

/** The part of a decoded image that falls inside `region` when its top-left is at (x, y). */
export function placeImage(image: { w: number; h: number; data: Uint8ClampedArray }, x: number, y: number, region: Region): Raster | null {
  const area = intersect(region, { x, y, w: image.w, h: image.h });
  if (!area) return null;
  const out = newRaster(area);
  for (let row = 0; row < area.h; row++) {
    const from = ((area.y - y + row) * image.w + (area.x - x)) * 4;
    out.data.set(image.data.subarray(from, from + area.w * 4), row * area.w * 4);
  }
  return out;
}

/**
 * Draw an image through an affine transform [a, c, tx, b, d, ty] with bilinear
 * sampling. Texels are clamped at the image border, and the border itself is
 * anti-aliased, so an image scaled by a whole number keeps crisp edges.
 */
export function resample(image: { w: number; h: number; data: Uint8ClampedArray }, m: readonly number[], region: Region): Raster | null {
  const [a, c, tx, b, d, ty] = m as [number, number, number, number, number, number];
  const det = a * d - b * c;
  if (Math.abs(det) < 1e-12) return null;
  const xs = [0, image.w].flatMap((u) => [0, image.h].map((v) => a * u + c * v + tx));
  const ys = [0, image.w].flatMap((u) => [0, image.h].map((v) => b * u + d * v + ty));
  const x0 = Math.floor(Math.min(...xs)), y0 = Math.floor(Math.min(...ys));
  const area = intersect(region, { x: x0, y: y0, w: Math.ceil(Math.max(...xs)) - x0, h: Math.ceil(Math.max(...ys)) - y0 });
  if (!area) return null;
  const out = newRaster(area);
  const ia = d / det, ic = -c / det, ib = -b / det, id = a / det;
  // Output pixels per texel along each image axis, for edge coverage.
  const scaleU = Math.hypot(a, b), scaleV = Math.hypot(c, d);
  const src = image.data, W = image.w, H = image.h;
  const clampU = (u: number) => (u < 0 ? 0 : u > W - 1 ? W - 1 : u);
  const clampV = (v: number) => (v < 0 ? 0 : v > H - 1 ? H - 1 : v);
  for (let y = 0; y < area.h; y++) {
    for (let x = 0; x < area.w; x++) {
      // Center of this output pixel in image space, where the image spans [0,W]×[0,H].
      const px = area.x + x + 0.5 - tx, py = area.y + y + 0.5 - ty;
      const U = ia * px + ic * py, V = ib * px + id * py;
      const edge = Math.min(1, Math.min(U, W - U) * scaleU + 0.5) * Math.min(1, Math.min(V, H - V) * scaleV + 0.5);
      if (edge <= 0) continue;
      const u = U - 0.5, v = V - 0.5;
      const uf = Math.floor(u), vf = Math.floor(v), fu = u - uf, fv = v - vf;
      const u0 = clampU(uf), u1 = clampU(uf + 1), v0 = clampV(vf), v1 = clampV(vf + 1);
      // Interpolate premultiplied, so transparent texels do not bleed their color.
      let r = 0, g = 0, bl = 0, al = 0;
      for (let k = 0; k < 4; k++) {
        const weight = (k & 1 ? fu : 1 - fu) * (k & 2 ? fv : 1 - fv);
        if (weight <= 0) continue;
        const i = ((k & 2 ? v1 : v0) * W + (k & 1 ? u1 : u0)) * 4, alpha = src[i + 3]! * weight;
        r += src[i]! * alpha;
        g += src[i + 1]! * alpha;
        bl += src[i + 2]! * alpha;
        al += alpha;
      }
      if (al <= 0) continue;
      const o = (y * area.w + x) * 4;
      out.data[o] = r / al;
      out.data[o + 1] = g / al;
      out.data[o + 2] = bl / al;
      out.data[o + 3] = al * edge;
    }
  }
  return out;
}
