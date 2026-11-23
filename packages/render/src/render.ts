/**
 * The render core: `(document, assets) -> pixels` (rule 1), headless (rule 2).
 *
 * CanvasKit is used for what it is good at: turning vector paths into
 * anti-aliased coverage and decoding images. All compositing runs in our own
 * typed-array loops (raster.ts), so the same document gives the same bytes in
 * Node and in the app, at a speed CanvasKit's CPU backend cannot reach.
 *
 * Compositing model (Photoshop's, for the features in docs/SURVEY.md):
 *   - A group's children paint bottom to top (children[0] first).
 *   - A pass-through group paints its children straight onto what is below it.
 *     Any other layer is built on its own first.
 *   - Clipped layers blend into their base and never change its alpha.
 *   - Masks cut the layer before effects see it.
 *   - Effects blend with the backdrop on their own: shadows and glows under
 *     the layer, overlays, inner shadow, bevel and stroke over it. Fill opacity
 *     fades the layer's pixels only. Opacity fades the lot.
 */
import type { AssetBytes, AssetId, Color, Doc, Effect, Gradient, Layer, Mat3, Path as DocPath, ShapeLayer } from "@cardsmith/doc";
import type { CanvasKit, Paint, Path } from "canvaskit-wasm";
import { ABOVE, BELOW, EffectContext, effectReach, ordered, type Shape, type Stamp } from "./effects.js";
import { blur } from "./field.js";
import {
  composite, copyRaster, crossFade, intersect, multiplyAlpha, newRaster, placeImage, resample, stamp,
  type Ink, type Raster, type Region,
} from "./raster.js";

export type RenderResult = {
  width: number;
  height: number;
  /** Straight-alpha RGBA, 8 bits per channel, row-major. */
  pixels: Uint8Array;
  /** Things the document asked for that this build does not draw. */
  warnings: string[];
};

type Decoded = { w: number; h: number; data: Uint8ClampedArray };

const activeEffects = (layer: Layer): Effect[] =>
  layer.effectsEnabled === false ? [] : (layer.effects ?? []).filter((e) => e.enabled);

const rasterMaskOn = (layer: Layer) => !!layer.mask && layer.mask.enabled !== false && (layer.mask.density ?? 1) > 0;
const vectorMaskOn = (layer: Layer) => !!layer.vectorMask && layer.vectorMask.enabled !== false;
const hasActiveMask = (layer: Layer) => rasterMaskOn(layer) || vectorMaskOn(layer);

class RenderPass {
  private images = new Map<AssetId, Decoded | null>();
  readonly warnings = new Set<string>();

  constructor(
    private ck: CanvasKit,
    private assets: AssetBytes,
  ) {}

  // ---- CanvasKit: decoding and path coverage --------------------------------

  private image(id: AssetId, usedBy: string): Decoded | null {
    let decoded = this.images.get(id);
    if (decoded === undefined) {
      const { ck } = this;
      const bytes = this.assets[id];
      const image = bytes ? ck.MakeImageFromEncoded(bytes) : null;
      decoded = null;
      if (image) {
        const w = image.width(), h = image.height();
        const pixels = image.readPixels(0, 0, {
          width: w,
          height: h,
          colorType: ck.ColorType.RGBA_8888,
          alphaType: ck.AlphaType.Unpremul,
          colorSpace: ck.ColorSpace.SRGB,
        }) as Uint8Array | null;
        image.delete();
        if (pixels) decoded = { w, h, data: new Uint8ClampedArray(pixels) };
      }
      if (!decoded) this.warnings.add(bytes ? `Asset "${id}" is not a readable image.` : `Asset "${id}" has no data.`);
      this.images.set(id, decoded);
    }
    if (!decoded) this.warnings.add(`Layer "${usedBy}" was skipped: its image is unavailable.`);
    return decoded;
  }

  private path(source: DocPath): Path | null {
    const path = this.ck.Path.MakeFromSVGString(source.d);
    if (!path) {
      this.warnings.add(`A vector path could not be parsed: "${source.d.slice(0, 40)}"`);
      return null;
    }
    path.setFillType(source.fillRule === "evenodd" ? this.ck.FillType.EvenOdd : this.ck.FillType.Winding);
    return path;
  }

  /** Anti-aliased coverage (0..255) of whatever `draw` paints, over `region`. */
  private coverage(region: Region, draw: (canvas: ReturnType<NonNullable<ReturnType<CanvasKit["MakeSurface"]>>["getCanvas"]>, paint: Paint) => void): Uint8Array {
    const { ck } = this;
    // Skia draws straight into an alpha-only buffer of ours: no readback needed.
    const memory = ck.Malloc(Uint8Array, region.w * region.h);
    const surface = ck.MakeRasterDirectSurface(
      { width: region.w, height: region.h, colorType: ck.ColorType.Alpha_8, alphaType: ck.AlphaType.Premul, colorSpace: ck.ColorSpace.SRGB },
      memory,
      region.w,
    );
    if (!surface) {
      ck.Free(memory);
      throw new Error(`Could not allocate a ${region.w}×${region.h} coverage buffer`);
    }
    const canvas = surface.getCanvas();
    canvas.clear(ck.TRANSPARENT);
    canvas.translate(-region.x, -region.y);
    const paint = new ck.Paint();
    paint.setAntiAlias(true);
    paint.setColor(ck.WHITE);
    draw(canvas, paint);
    surface.flush();
    const out = Uint8Array.from(memory.toTypedArray() as Uint8Array);
    paint.delete();
    surface.delete();
    ck.Free(memory);
    return out;
  }

  // ---- tree -----------------------------------------------------------------

  drawChildren(target: Raster, layers: Layer[]): void {
    for (let i = 0; i < layers.length; ) {
      const base = layers[i]!;
      let end = i + 1;
      while (end < layers.length && layers[end]!.clip) end++;
      // A hidden base hides everything clipped to it.
      if (base.visible) this.drawLayer(target, base, layers.slice(i + 1, end).filter((l) => l.visible));
      i = end;
    }
  }

  private drawLayer(target: Raster, layer: Layer, clipped: Layer[]): void {
    const effects = activeEffects(layer);

    if (layer.kind === "group" && layer.passThrough && effects.length === 0 && clipped.length === 0) {
      if (layer.opacity >= 1 && !hasActiveMask(layer)) return this.drawChildren(target, layer.children);
      if (layer.opacity <= 0) return;
      const work = copyRaster(target);
      this.drawChildren(work, layer.children);
      return crossFade(target, work, layer.opacity, this.maskCoverage(layer, target));
    }
    if (layer.opacity <= 0) return;

    const content = this.content(layer, target);
    if (!content) return;
    const mask = this.maskCoverage(layer, content);
    if (mask) multiplyAlpha(content, mask);

    // The layer's silhouette is fixed here: clipped layers cannot change alpha.
    // Always keep a margin, so the region contains the layer's edge even for inside-only effects.
    const shape = effects.length ? shapeOf(content, Math.max(2, ...effects.map(effectReach))) : undefined;
    if (effects.length && !shape) return; // nothing visible, so nothing to cast effects

    for (const child of clipped) {
      // Rendered on its own first, so its masks, effects and children are all honoured.
      const own = newRaster(content);
      this.drawLayer(own, { ...child, opacity: 1, blend: "normal" }, []);
      composite(content, own, child.opacity, child.blend, true);
    }
    const fill = layer.fillOpacity ?? 1;

    if (!shape) return composite(target, content, layer.opacity * fill, layer.blend);

    const work = layer.opacity < 1 ? copyRaster(target) : target;
    const fx = new EffectContext(shape);
    for (const effect of ordered(effects, BELOW)) for (const s of fx.stamps(effect)) this.stamp(work, shape, s);
    composite(work, content, fill, layer.blend);
    for (const effect of ordered(effects, ABOVE)) for (const s of fx.stamps(effect)) this.stamp(work, shape, s);
    if (work !== target) crossFade(target, work, layer.opacity, null);
  }

  /** The layer's own pixels within `region`, before masks. Null when there are none. */
  private content(layer: Layer, region: Region): Raster | null {
    switch (layer.kind) {
      case "group": {
        const out = newRaster(region);
        this.drawChildren(out, layer.children);
        return out;
      }

      case "art":
        if (layer.asset === undefined) return null;
      // falls through
      case "pixel":
      case "fill": {
        const image = this.image(layer.asset!, layer.name);
        if (!image) return null;
        const [a, c, tx, b, d, ty] = layer.transform;
        const unscaled = a === 1 && d === 1 && b === 0 && c === 0 && Number.isInteger(tx) && Number.isInteger(ty);
        return unscaled ? placeImage(image, tx, ty, region) : resample(image, layer.transform as Mat3, region);
      }

      case "shape":
        return this.shape(layer, region);

      case "text":
        this.warnings.add(`Text layer "${layer.name}" was skipped: text rendering arrives in Phase 3.`);
        return null;
    }
  }

  private shape(layer: ShapeLayer, region: Region): Raster | null {
    const { ck } = this;
    const path = this.path(layer.path);
    if (!path) return null;
    const stroke = layer.stroke && layer.stroke.width > 0 ? layer.stroke : undefined;
    const grow = stroke && stroke.align === "center" ? stroke.width / 2 + 1 : 1;
    const [l, t, r, b] = path.getBounds();
    const area = intersect(region, {
      x: Math.floor(l! - grow),
      y: Math.floor(t! - grow),
      w: Math.ceil(r! + grow) - Math.floor(l! - grow),
      h: Math.ceil(b! + grow) - Math.floor(t! - grow),
    });
    if (!area) {
      path.delete();
      return null;
    }
    const out = newRaster(area);
    this.ink(out, this.coverage(area, (canvas, paint) => canvas.drawPath(path, paint)), layer.fill.color);
    if (stroke) {
      const inside = stroke.align === "inside";
      const edge = this.coverage(area, (canvas, paint) => {
        paint.setStyle(ck.PaintStyle.Stroke);
        // An inside stroke is a double-width stroke with the outer half clipped away.
        paint.setStrokeWidth(stroke.width * (inside ? 2 : 1));
        if (inside) canvas.clipPath(path, ck.ClipOp.Intersect, true);
        canvas.drawPath(path, paint);
      });
      const over = newRaster(area);
      this.ink(over, edge, stroke.color);
      composite(out, over, 1, "normal");
    }
    path.delete();
    return out;
  }

  /** Fill a raster with one color through a 0..255 coverage map. */
  private ink(raster: Raster, coverage: Uint8Array, [r, g, b, a]: Color): void {
    const d = raster.data;
    for (let p = 0, i = 0; p < coverage.length; p++, i += 4) {
      const c = coverage[p]!;
      if (c === 0) continue;
      d[i] = r * 255;
      d[i + 1] = g * 255;
      d[i + 2] = b * 255;
      d[i + 3] = c * a;
    }
  }

  /** Combined raster and vector mask coverage (0..255) over `region`, or null if the layer has no active mask. */
  private maskCoverage(layer: Layer, region: Region): Uint8Array | null {
    let out: Uint8Array | null = null;

    const vector = layer.vectorMask;
    if (vector && vector.enabled !== false) {
      const path = this.path(vector.path);
      if (path) {
        out = this.coverage(region, (canvas, paint) => canvas.drawPath(path, paint));
        if (vector.inverted) for (let i = 0; i < out.length; i++) out[i] = 255 - out[i]!;
        path.delete();
      }
    }

    const mask = layer.mask;
    const density = mask?.density ?? 1;
    if (!mask || mask.enabled === false || density <= 0) return out;
    const image = this.image(mask.asset, layer.name);
    if (!image) return out;

    const n = region.w * region.h;
    const outside = mask.outside === "reveal" ? 255 : 0;
    let gray: Uint8Array | Float32Array = new Uint8Array(n).fill(outside);
    const area = intersect(region, { x: mask.offset.x, y: mask.offset.y, w: image.w, h: image.h });
    if (area) {
      for (let y = 0; y < area.h; y++) {
        let si = ((area.y - mask.offset.y + y) * image.w + (area.x - mask.offset.x)) * 4;
        let di = (area.y - region.y + y) * region.w + (area.x - region.x);
        // Luma, so a mask that is not strictly gray still behaves sensibly.
        for (let x = 0; x < area.w; x++, si += 4, di++) gray[di] = (54 * image.data[si]! + 183 * image.data[si + 1]! + 19 * image.data[si + 2]!) >> 8;
      }
    }
    const feather = mask.feather ?? 0;
    if (feather > 0) {
      // Blur with the outside value extended past the region, so the edge of the region is not darkened.
      const shifted = Float32Array.from(gray, (v) => (v - outside) / 255);
      gray = blur(shifted, region.w, region.h, feather / 2).map((v) => v * 255 + outside);
    }
    const result = out ?? new Uint8Array(n).fill(255);
    for (let i = 0; i < n; i++) {
      let m = gray[i]!;
      if (mask.inverted) m = 255 - m;
      if (density < 1) m = 255 - density * (255 - m);
      result[i] = Math.round((result[i]! * m) / 255);
    }
    return result;
  }

  // ---- effects --------------------------------------------------------------

  private stamp(target: Raster, shape: Shape, s: Stamp): void {
    const ink: Ink = s.gradient
      ? gradientInk(s.gradient)
      : { r: s.color[0] * 255, g: s.color[1] * 255, b: s.color[2] * 255, a: s.color[3] };
    stamp(target, shape, s.coverage, ink, s.opacity, s.blend);
  }
}

/** A layer's alpha, cropped to its visible pixels plus `reach` on every side. */
function shapeOf(content: Raster, reach: number): Shape | undefined {
  const { w: W, h: H, data } = content;
  let x0 = W, y0 = H, x1 = -1, y1 = -1;
  for (let y = 0; y < H; y++) {
    const row = y * W * 4 + 3;
    for (let x = 0; x < W; x++) {
      if (data[row + x * 4] === 0) continue;
      if (x < x0) x0 = x;
      if (x > x1) x1 = x;
      if (y < y0) y0 = y;
      y1 = y;
    }
  }
  if (x1 < 0) return undefined;

  // The region may reach past the document edge, where alpha is zero. So a
  // layer that touches the edge still has an edge there, as in Photoshop.
  const w = x1 - x0 + 1 + reach * 2, h = y1 - y0 + 1 + reach * 2;
  const alpha = new Float32Array(w * h);
  for (let y = y0; y <= y1; y++) {
    const src = (y * W + x0) * 4 + 3;
    const dst = (y - y0 + reach) * w + reach;
    for (let i = 0; i <= x1 - x0; i++) alpha[dst + i] = data[src + i * 4]! / 255;
  }
  return {
    x: content.x + x0 - reach,
    y: content.y + y0 - reach,
    w,
    h,
    alpha,
    bounds: { x: content.x + x0, y: content.y + y0, w: x1 - x0 + 1, h: y1 - y0 + 1 },
  };
}

/** Color lookup for a gradient stamp. Stops interpolate in straight sRGB, as Photoshop's classic mode does. */
function gradientInk(g: { gradient: Gradient; style: "linear" | "radial"; start: [number, number]; end: [number, number] }): Ink {
  const STEPS = 1024;
  const stops = [...g.gradient.stops].sort((p, q) => p.offset - q.offset);
  const lut = new Float64Array((STEPS + 1) * 4);
  for (let i = 0, s = 0; i <= STEPS; i++) {
    const t = i / STEPS;
    while (s < stops.length - 2 && t > stops[s + 1]!.offset) s++;
    const from = stops[s]!, to = stops[Math.min(s + 1, stops.length - 1)]!;
    const span = to.offset - from.offset;
    const f = span > 0 ? Math.min(1, Math.max(0, (t - from.offset) / span)) : t < from.offset ? 0 : 1;
    for (let c = 0; c < 4; c++) lut[i * 4 + c] = (from.color[c]! + (to.color[c]! - from.color[c]!) * f) * (c < 3 ? 255 : 1);
  }
  const [sx, sy] = g.start;
  const dx = g.end[0] - sx, dy = g.end[1] - sy;
  const len2 = dx * dx + dy * dy || 1;
  const radial = g.style === "radial";
  return (x, y, out) => {
    const px = x + 0.5 - sx, py = y + 0.5 - sy;
    const t = radial ? Math.sqrt((px * px + py * py) / len2) : (px * dx + py * dy) / len2;
    const i = Math.round((t < 0 ? 0 : t > 1 ? 1 : t) * STEPS) * 4;
    out[0] = lut[i]!;
    out[1] = lut[i + 1]!;
    out[2] = lut[i + 2]!;
    out[3] = lut[i + 3]!;
  };
}

/** Reusable renderer. Stateless today; kept as a class so caches can live here later. */
export class Renderer {
  constructor(private ck: CanvasKit) {}

  render(doc: Doc, assets: AssetBytes): RenderResult {
    const pass = new RenderPass(this.ck, assets);
    const target = newRaster({ x: 0, y: 0, w: doc.width, h: doc.height });
    // The root is always a plain container, whatever its own properties say.
    pass.drawChildren(target, doc.root.children);
    return {
      width: doc.width,
      height: doc.height,
      pixels: new Uint8Array(target.data.buffer),
      warnings: [...pass.warnings],
    };
  }

  dispose(): void {}
}

/** One-off render. */
export function renderDoc(ck: CanvasKit, doc: Doc, assets: AssetBytes): RenderResult {
  return new Renderer(ck).render(doc, assets);
}

/** Encode straight-alpha RGBA as PNG. */
export function encodePng(ck: CanvasKit, width: number, height: number, pixels: Uint8Array): Uint8Array {
  const image = ck.MakeImage(
    { width, height, colorType: ck.ColorType.RGBA_8888, alphaType: ck.AlphaType.Unpremul, colorSpace: ck.ColorSpace.SRGB },
    pixels,
    width * 4,
  );
  const png = image?.encodeToBytes();
  image?.delete();
  if (!png) throw new Error("PNG encode failed");
  return png;
}
