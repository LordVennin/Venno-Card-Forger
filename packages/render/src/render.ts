/**
 * The render core: `(document, assets) -> pixels` (rule 1), headless (rule 2).
 *
 * Everything is drawn on CPU raster surfaces, in Node and in the app alike, so
 * the same document gives the same bytes everywhere.
 *
 * Compositing model (Photoshop's, for the features in docs/SURVEY.md):
 *   - A group's children paint bottom to top (children[0] first).
 *   - A pass-through group paints its children straight onto what is below it.
 *     Any other layer is built on its own transparent surface first.
 *   - Clipped layers blend into their base and never change its alpha.
 *   - Masks cut the layer before effects see it.
 *   - Effects blend with the backdrop on their own: shadows and glows under
 *     the layer, overlays, inner shadow, bevel and stroke over it. Fill opacity
 *     fades the layer's pixels only. Opacity fades the lot.
 */
import type { AssetBytes, AssetId, BlendMode, Color, Doc, Effect, Layer, Mat3, Path as DocPath } from "@cardsmith/doc";
import type { Canvas, CanvasKit, Image, Paint, Path, Surface } from "canvaskit-wasm";
import { Blending } from "./blend.js";
import { ABOVE, BELOW, EffectContext, effectReach, ordered, type Shape, type Stamp } from "./effects.js";

export type RenderResult = {
  width: number;
  height: number;
  /** Straight-alpha RGBA, 8 bits per channel, row-major. */
  pixels: Uint8Array;
  /** Things the document asked for that this build does not draw. */
  warnings: string[];
};

const activeEffects = (layer: Layer): Effect[] =>
  layer.effectsEnabled === false ? [] : (layer.effects ?? []).filter((e) => e.enabled);

const hasActiveMask = (layer: Layer): boolean =>
  (!!layer.mask && layer.mask.enabled !== false && (layer.mask.density ?? 1) > 0) ||
  (!!layer.vectorMask && layer.vectorMask.enabled !== false);

class RenderPass {
  private images = new Map<AssetId, Image | null>();
  private pool: Surface[] = [];
  private live = new Set<Surface>();
  readonly warnings = new Set<string>();

  constructor(
    private ck: CanvasKit,
    private blending: Blending,
    private doc: Doc,
    private assets: AssetBytes,
  ) {}

  // ---- surfaces -----------------------------------------------------------

  private acquire(): Surface {
    const surface = this.pool.pop() ?? this.ck.MakeSurface(this.doc.width, this.doc.height);
    if (!surface) throw new Error(`Could not allocate a ${this.doc.width}×${this.doc.height} surface`);
    surface.getCanvas().clear(this.ck.TRANSPARENT);
    this.live.add(surface);
    return surface;
  }

  private release(surface: Surface): void {
    this.live.delete(surface);
    this.pool.push(surface);
  }

  /** Draw all of `src` onto `dst` with the given paint setup. */
  private blit(dst: Surface, src: Surface, setup?: (paint: Paint) => void): void {
    const image = src.makeImageSnapshot();
    const paint = new this.ck.Paint();
    setup?.(paint);
    dst.getCanvas().drawImage(image, 0, 0, paint);
    paint.delete();
    image.delete();
  }

  private copy(src: Surface): Surface {
    const out = this.acquire();
    this.blit(out, src, (p) => p.setBlendMode(this.ck.BlendMode.Src));
    return out;
  }

  /** dst = dst·(1 − opacity·m) + src·opacity·m, where m is the layer's mask coverage. */
  private crossFade(dst: Surface, src: Surface, layer: Layer): void {
    const { ck } = this;
    if (hasActiveMask(layer)) {
      const mask = this.acquire();
      mask.getCanvas().clear(ck.WHITE);
      this.applyMasks(mask, layer);
      this.applyMasks(src, layer);
      this.blit(dst, mask, (p) => {
        p.setBlendMode(ck.BlendMode.DstOut);
        p.setAlphaf(layer.opacity);
      });
      this.release(mask);
    } else {
      const paint = new ck.Paint();
      paint.setBlendMode(ck.BlendMode.DstOut);
      paint.setColor(ck.Color4f(0, 0, 0, layer.opacity));
      dst.getCanvas().drawPaint(paint);
      paint.delete();
    }
    this.blit(dst, src, (p) => {
      p.setBlendMode(ck.BlendMode.Plus);
      p.setAlphaf(layer.opacity);
    });
  }

  // ---- assets -------------------------------------------------------------

  private image(id: AssetId, usedBy: string): Image | null {
    let image = this.images.get(id);
    if (image === undefined) {
      const bytes = this.assets[id];
      image = bytes ? this.ck.MakeImageFromEncoded(bytes) : null;
      if (!image) this.warnings.add(bytes ? `Asset "${id}" is not a readable image.` : `Asset "${id}" has no data.`);
      this.images.set(id, image);
    }
    if (!image) this.warnings.add(`Layer "${usedBy}" was skipped: its image is unavailable.`);
    return image;
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

  private color([r, g, b, a]: Color, opacity = 1) {
    return this.ck.Color4f(r, g, b, a * opacity);
  }

  // ---- tree ---------------------------------------------------------------

  drawChildren(target: Surface, layers: Layer[]): void {
    for (let i = 0; i < layers.length; ) {
      const base = layers[i]!;
      let end = i + 1;
      while (end < layers.length && layers[end]!.clip) end++;
      // A hidden base hides everything clipped to it.
      if (base.visible) this.drawLayer(target, base, layers.slice(i + 1, end).filter((l) => l.visible));
      i = end;
    }
  }

  private drawLayer(target: Surface, layer: Layer, clipped: Layer[]): void {
    const { ck } = this;
    const effects = activeEffects(layer);

    if (layer.kind === "group" && layer.passThrough && effects.length === 0 && clipped.length === 0) {
      if (layer.opacity >= 1 && !hasActiveMask(layer)) return this.drawChildren(target, layer.children);
      if (layer.opacity <= 0) return;
      const work = this.copy(target);
      this.drawChildren(work, layer.children);
      this.crossFade(target, work, layer);
      return this.release(work);
    }
    if (layer.opacity <= 0) return;

    const content = this.acquire();
    this.drawContent(content, layer);
    this.applyMasks(content, layer);

    // The layer's silhouette is fixed here: clipped layers cannot change alpha.
    // Always keep a margin, so the region contains the layer's edge even for inside-only effects.
    const shape = effects.length ? this.readShape(content, Math.max(2, ...effects.map(effectReach))) : undefined;
    for (const child of clipped) this.drawClipped(content, child);
    const fill = layer.fillOpacity ?? 1;

    if (!effects.length) {
      this.blit(target, content, (p) => {
        p.setAlphaf(layer.opacity * fill);
        this.blending.apply(p, layer.blend);
      });
      return this.release(content);
    }
    if (!shape) return this.release(content); // nothing visible, so nothing to cast effects

    const work = layer.opacity < 1 ? this.copy(target) : target;
    const fx = new EffectContext(shape);
    for (const effect of ordered(effects, BELOW)) for (const stamp of fx.stamps(effect)) this.stamp(work, shape, stamp);
    if (fill > 0) {
      this.blit(work, content, (p) => {
        p.setAlphaf(fill);
        this.blending.apply(p, layer.blend);
      });
    }
    for (const effect of ordered(effects, ABOVE)) for (const stamp of fx.stamps(effect)) this.stamp(work, shape, stamp);
    this.release(content);

    if (work !== target) {
      this.crossFade(target, work, { ...layer, mask: undefined, vectorMask: undefined });
      this.release(work);
    }
  }

  /** Blend a clipped layer into its base's surface without touching the base's alpha. */
  private drawClipped(base: Surface, layer: Layer): void {
    const own = this.acquire();
    // Rendered on its own first, so its masks, effects and children are all honoured.
    this.drawLayer(own, { ...layer, opacity: 1, blend: "normal" }, []);
    this.blit(base, own, (p) => {
      p.setAlphaf(layer.opacity);
      this.blending.apply(p, layer.blend, true);
    });
    this.release(own);
  }

  private drawContent(surface: Surface, layer: Layer): void {
    const { ck } = this;
    const canvas = surface.getCanvas();
    switch (layer.kind) {
      case "group":
        return this.drawChildren(surface, layer.children);

      case "art":
        if (layer.asset === undefined) return;
      // falls through
      case "pixel":
      case "fill": {
        const image = this.image(layer.asset!, layer.name);
        if (!image) return;
        canvas.save();
        canvas.concat(matrix(layer.transform));
        canvas.drawImageOptions(image, 0, 0, ck.FilterMode.Linear, ck.MipmapMode.None, null);
        canvas.restore();
        return;
      }

      case "shape": {
        const path = this.path(layer.path);
        if (!path) return;
        const paint = new ck.Paint();
        paint.setAntiAlias(true);
        paint.setColor(this.color(layer.fill.color));
        canvas.drawPath(path, paint);
        if (layer.stroke && layer.stroke.width > 0) {
          const inside = layer.stroke.align === "inside";
          paint.setStyle(ck.PaintStyle.Stroke);
          paint.setColor(this.color(layer.stroke.color));
          // An inside stroke is a double-width stroke with the outer half clipped away.
          paint.setStrokeWidth(layer.stroke.width * (inside ? 2 : 1));
          canvas.save();
          if (inside) canvas.clipPath(path, ck.ClipOp.Intersect, true);
          canvas.drawPath(path, paint);
          canvas.restore();
        }
        paint.delete();
        path.delete();
        return;
      }

      case "text":
        this.warnings.add(`Text layer "${layer.name}" was skipped: text rendering arrives in Phase 3.`);
        return;
    }
  }

  private applyMasks(surface: Surface, layer: Layer): void {
    const { ck } = this;
    const canvas = surface.getCanvas();

    const vector = layer.vectorMask;
    if (vector && vector.enabled !== false) {
      const path = this.path(vector.path);
      if (path) {
        canvas.save();
        // Clear what the mask hides: outside the path, or inside it when inverted.
        canvas.clipPath(path, vector.inverted ? ck.ClipOp.Intersect : ck.ClipOp.Difference, true);
        canvas.clear(ck.TRANSPARENT);
        canvas.restore();
        path.delete();
      }
    }

    const mask = layer.mask;
    const density = mask?.density ?? 1;
    if (!mask || mask.enabled === false || density <= 0) return;
    const image = this.image(mask.asset, layer.name);
    if (!image) return;

    // Build the mask as opaque gray over the whole document, then use its luma as coverage.
    const gray = this.acquire();
    const g = gray.getCanvas();
    g.clear(mask.outside === "reveal" ? ck.WHITE : ck.BLACK);
    const paint = new ck.Paint();
    paint.setBlendMode(ck.BlendMode.Src);
    g.drawImage(image, mask.offset.x, mask.offset.y, paint);
    if (mask.inverted) {
      paint.setBlendMode(ck.BlendMode.Difference);
      paint.setColor(ck.WHITE);
      g.drawPaint(paint);
    }
    if (density < 1) {
      paint.setBlendMode(ck.BlendMode.SrcOver);
      paint.setColor(ck.Color4f(1, 1, 1, 1 - density));
      g.drawPaint(paint);
    }
    paint.delete();

    this.blit(surface, gray, (p) => {
      p.setBlendMode(ck.BlendMode.DstIn);
      const luma = ck.ColorFilter.MakeLuma();
      const feather = mask.feather ?? 0;
      if (feather > 0) {
        const soften = ck.ImageFilter.MakeBlur(feather / 2, feather / 2, ck.TileMode.Clamp, null);
        const both = ck.ImageFilter.MakeColorFilter(luma, soften);
        p.setImageFilter(both);
        both.delete();
        soften.delete();
      } else {
        p.setColorFilter(luma);
      }
      luma.delete();
    });
    this.release(gray);
  }

  // ---- effects ------------------------------------------------------------

  /** Read a surface's alpha, cropped to its visible pixels plus `reach` on every side. */
  private readShape(surface: Surface, reach: number): Shape | undefined {
    const { ck } = this;
    const { width: W, height: H } = this.doc;
    const image = surface.makeImageSnapshot();
    const rgba = image.readPixels(0, 0, {
      width: W,
      height: H,
      colorType: ck.ColorType.RGBA_8888,
      alphaType: ck.AlphaType.Premul,
      colorSpace: ck.ColorSpace.SRGB,
    }) as Uint8Array | null;
    image.delete();
    if (!rgba) throw new Error("Could not read back layer pixels");

    let x0 = W, y0 = H, x1 = -1, y1 = -1;
    for (let y = 0; y < H; y++) {
      const row = y * W * 4 + 3;
      for (let x = 0; x < W; x++) {
        if (rgba[row + x * 4] === 0) continue;
        if (x < x0) x0 = x;
        if (x > x1) x1 = x;
        if (y < y0) y0 = y;
        y1 = y;
      }
    }
    if (x1 < 0) return undefined;

    // The region may reach past the document edge, where alpha is zero. So a
    // layer that touches the edge still has an edge there, as in Photoshop.
    const x = x0 - reach, y = y0 - reach;
    const w = x1 - x0 + 1 + reach * 2, h = y1 - y0 + 1 + reach * 2;
    const alpha = new Float32Array(w * h);
    for (let yy = y0; yy <= y1; yy++) {
      const src = (yy * W + x0) * 4 + 3;
      const dst = (yy - y) * w + (x0 - x);
      for (let i = 0; i <= x1 - x0; i++) alpha[dst + i] = rgba[src + i * 4]! / 255;
    }
    return { x, y, w, h, alpha, bounds: { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 } };
  }

  /** Paint a color or gradient through a coverage map. */
  private stamp(target: Surface, shape: Shape, stamp: Stamp): void {
    const { ck } = this;
    if (stamp.opacity <= 0) return;
    const bytes = new Uint8Array(stamp.coverage.length);
    let any = false;
    for (let i = 0; i < bytes.length; i++) {
      const v = Math.round(stamp.coverage[i]! * 255);
      bytes[i] = v;
      if (v) any = true;
    }
    if (!any) return;
    const image = ck.MakeImage(
      { width: shape.w, height: shape.h, colorType: ck.ColorType.Alpha_8, alphaType: ck.AlphaType.Premul, colorSpace: ck.ColorSpace.SRGB },
      bytes,
      shape.w,
    );
    if (!image) throw new Error("Could not build an effect coverage image");

    const paint = new ck.Paint();
    this.blending.apply(paint, stamp.blend);
    if (stamp.gradient) {
      // An alpha-only image takes its color from the paint's shader, in the image's own coordinates.
      const { gradient, style, start, end } = stamp.gradient;
      const local = (p: [number, number]) => [p[0] - shape.x, p[1] - shape.y];
      const colors = gradient.stops.map((s) => this.color(s.color));
      const offsets = gradient.stops.map((s) => s.offset);
      const shader =
        style === "radial"
          ? ck.Shader.MakeRadialGradient(local(start), Math.hypot(end[0] - start[0], end[1] - start[1]), colors, offsets, ck.TileMode.Clamp)
          : ck.Shader.MakeLinearGradient(local(start), local(end), colors, offsets, ck.TileMode.Clamp);
      paint.setShader(shader);
      paint.setAlphaf(stamp.opacity);
      shader.delete();
    } else {
      paint.setColor(this.color(stamp.color, stamp.opacity));
    }
    target.getCanvas().drawImage(image, shape.x, shape.y, paint);
    paint.delete();
    image.delete();
  }

  // ---- lifecycle ----------------------------------------------------------

  dispose(): void {
    for (const image of this.images.values()) image?.delete();
    for (const surface of [...this.pool, ...this.live]) surface.delete();
    this.images.clear();
    this.pool = [];
    this.live.clear();
  }
}

function matrix([a, c, tx, b, d, ty]: Mat3): number[] {
  return [a, c, tx, b, d, ty, 0, 0, 1];
}

/** Reusable renderer. Holds compiled shaders; call `dispose()` when finished. */
export class Renderer {
  private blending: Blending;

  constructor(private ck: CanvasKit) {
    this.blending = new Blending(ck);
  }

  render(doc: Doc, assets: AssetBytes): RenderResult {
    const { ck } = this;
    const pass = new RenderPass(ck, this.blending, doc, assets);
    const surface = ck.MakeSurface(doc.width, doc.height);
    if (!surface) throw new Error(`Could not allocate a ${doc.width}×${doc.height} surface`);
    try {
      surface.getCanvas().clear(ck.TRANSPARENT);
      // The root is always a plain container, whatever its own properties say.
      pass.drawChildren(surface, doc.root.children);
      const image = surface.makeImageSnapshot();
      const pixels = image.readPixels(0, 0, {
        width: doc.width,
        height: doc.height,
        colorType: ck.ColorType.RGBA_8888,
        alphaType: ck.AlphaType.Unpremul,
        colorSpace: ck.ColorSpace.SRGB,
      }) as Uint8Array | null;
      image.delete();
      if (!pixels) throw new Error("Could not read back the rendered image");
      return { width: doc.width, height: doc.height, pixels: Uint8Array.from(pixels), warnings: [...pass.warnings] };
    } finally {
      pass.dispose();
      surface.delete();
    }
  }

  /** Exposed for tests: paint one color over another through the given blend path. */
  blendPixel(dst: Color, src: Color, mode: BlendMode, atop: boolean, forceCustom: boolean): number[] {
    const { ck } = this;
    const surface = ck.MakeSurface(1, 1)!;
    const canvas = surface.getCanvas();
    canvas.clear(ck.Color4f(...dst));
    const paint = new ck.Paint();
    paint.setColor(ck.Color4f(...src));
    this.blending.apply(paint, mode, atop, forceCustom);
    canvas.drawPaint(paint);
    paint.delete();
    const image = surface.makeImageSnapshot();
    const px = image.readPixels(0, 0, { width: 1, height: 1, colorType: ck.ColorType.RGBA_8888, alphaType: ck.AlphaType.Unpremul, colorSpace: ck.ColorSpace.SRGB }) as Uint8Array;
    const out = [...px];
    image.delete();
    surface.delete();
    return out;
  }

  dispose(): void {
    this.blending.dispose();
  }
}

/** One-off render. For repeated renders, keep a `Renderer`. */
export function renderDoc(ck: CanvasKit, doc: Doc, assets: AssetBytes): RenderResult {
  const renderer = new Renderer(ck);
  try {
    return renderer.render(doc, assets);
  } finally {
    renderer.dispose();
  }
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
