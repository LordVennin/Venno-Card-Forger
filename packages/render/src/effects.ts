/**
 * Layer effects as coverage maps.
 *
 * Each effect turns the layer's alpha into one or more "paint this color
 * through this coverage" instructions. The math is plain arrays (see field.ts),
 * so results do not depend on the Skia backend. The renderer does the painting.
 *
 * Only the seven effect types found by the survey exist here.
 */
import type { BlendMode, Color, Effect, Gradient } from "@cardsmith/doc";
import { blur, cover, signedDistance } from "./field.js";

/** A layer's alpha over a rectangular region of the document. */
export type Shape = {
  /** Region origin in document pixels. May be negative: the region extends past the layer's bounds. */
  x: number;
  y: number;
  w: number;
  h: number;
  alpha: Float32Array;
  /** Tight bounds of the non-transparent pixels, in document pixels. */
  bounds: { x: number; y: number; w: number; h: number };
};

export type Stamp = {
  coverage: Float32Array;
  color: Color;
  /** When set, the gradient replaces `color`. Points are in document pixels. */
  gradient?: { gradient: Gradient; style: "linear" | "radial"; start: [number, number]; end: [number, number] };
  blend: BlendMode;
  opacity: number;
};

/**
 * Blur sigma for a Photoshop "size" in pixels. Photoshop's falloff reaches zero
 * at about `size`; a Gaussian of size/2 is the usual match. Calibrated against
 * golden references in Phase 2.
 */
const SIGMA_PER_SIZE = 0.5;

/** How far outside the layer's alpha an effect can paint, in pixels. */
export function effectReach(effect: Effect): number {
  switch (effect.type) {
    case "dropShadow":
      return Math.ceil(effect.distance + effect.size * 1.6) + 2;
    case "outerGlow":
      return Math.ceil(effect.size * 1.6) + 2;
    case "stroke":
      return effect.position === "inside" ? 0 : Math.ceil(effect.size) + 2;
    case "bevel":
      return effect.style === "outer" ? Math.ceil(effect.size + effect.soften * 1.6) + 2 : 0;
    default:
      return 0;
  }
}

/** Unit vector pointing from the surface toward the light, in y-down pixel space. */
function lightDir(angle: number): [number, number] {
  const r = (angle * Math.PI) / 180;
  return [Math.cos(r), -Math.sin(r)];
}

/** Sample `src` shifted by a whole-pixel offset; out-of-range reads give `fill`. */
function shifted(src: Float32Array, w: number, h: number, dx: number, dy: number, fill: number): Float32Array {
  if (dx === 0 && dy === 0) return src;
  const out = new Float32Array(src.length).fill(fill);
  for (let y = 0; y < h; y++) {
    const sy = y - dy;
    if (sy < 0 || sy >= h) continue;
    for (let x = 0; x < w; x++) {
      const sx = x - dx;
      if (sx >= 0 && sx < w) out[y * w + x] = src[sy * w + sx]!;
    }
  }
  return out;
}

export class EffectContext {
  private sdCache?: Float32Array;
  constructor(readonly shape: Shape) {}

  /** Signed distance to the layer's edge. Computed on first use. */
  get sd(): Float32Array {
    return (this.sdCache ??= signedDistance(this.shape.alpha, this.shape.w, this.shape.h));
  }

  private map(f: (i: number) => number): Float32Array {
    const out = new Float32Array(this.shape.alpha.length);
    for (let i = 0; i < out.length; i++) out[i] = f(i);
    return out;
  }

  /** Shadow or glow cast outward, hidden where the layer itself is. */
  private outer(color: Color, blend: BlendMode, opacity: number, size: number, spread: number, dx: number, dy: number, gaussian = false): Stamp {
    const { w, h, alpha } = this.shape;
    const grow = spread * size;
    const matte = grow > 0 ? this.map((i) => cover(this.sd[i]! - grow)) : alpha;
    const soft = blur(shifted(matte, w, h, dx, dy, 0), w, h, (size - grow) * SIGMA_PER_SIZE);
    // The Gaussian contour remaps the falloff through an S-curve: fainter at the rim, denser near the layer.
    const curve = gaussian ? (v: number) => v * v * (3 - 2 * v) : (v: number) => v;
    return { coverage: this.map((i) => curve(soft[i]!) * (1 - alpha[i]!)), color, blend, opacity };
  }

  stamps(effect: Effect): Stamp[] {
    const { w, h, alpha, bounds } = this.shape;
    switch (effect.type) {
      case "colorOverlay":
        return [{ coverage: alpha, color: effect.color, blend: effect.blend, opacity: effect.opacity }];

      case "gradientOverlay": {
        const [lx, ly] = lightDir(effect.angle);
        const cx = bounds.x + bounds.w / 2;
        const cy = bounds.y + bounds.h / 2;
        // The gradient line crosses the bounds' center and spans their projection onto its direction.
        const half = ((Math.abs(bounds.w * lx) + Math.abs(bounds.h * ly)) / 2) * effect.scale;
        let start: [number, number] = [cx - lx * half, cy - ly * half];
        let end: [number, number] = [cx + lx * half, cy + ly * half];
        if (effect.style === "radial") [start, end] = [[cx, cy], [cx + lx * half, cy + ly * half]];
        const stops = effect.reverse
          ? effect.gradient.stops.map((s) => ({ offset: 1 - s.offset, color: s.color })).reverse()
          : effect.gradient.stops;
        return [{
          coverage: alpha,
          color: [0, 0, 0, 1],
          gradient: { gradient: { stops }, style: effect.style, start, end },
          blend: effect.blend,
          opacity: effect.opacity,
        }];
      }

      case "stroke": {
        const s = effect.size;
        const [outer, inner] = effect.position === "outside" ? [s, 0] : effect.position === "inside" ? [0, s] : [s / 2, s / 2];
        const coverage = this.map((i) => Math.max(0, cover(this.sd[i]! - outer) - cover(this.sd[i]! + inner)));
        return [{ coverage, color: effect.color, blend: effect.blend, opacity: effect.opacity }];
      }

      case "dropShadow": {
        const [lx, ly] = lightDir(effect.angle);
        const dx = Math.round(-lx * effect.distance);
        const dy = Math.round(-ly * effect.distance);
        return [this.outer(effect.color, effect.blend, effect.opacity, effect.size, effect.spread, dx, dy, effect.contour === "gaussian")];
      }

      case "outerGlow":
        return [this.outer(effect.color, effect.blend, effect.opacity, effect.size, effect.spread, 0, 0)];

      case "innerShadow": {
        const [lx, ly] = lightDir(effect.angle);
        const dx = Math.round(-lx * effect.distance);
        const dy = Math.round(-ly * effect.distance);
        const shrink = effect.choke * effect.size;
        // The shadow is cast by everything that is NOT the layer, so it is the inverse matte, grown by the choke.
        const matte = shrink > 0 ? this.map((i) => cover(-this.sd[i]! - shrink)) : this.map((i) => 1 - alpha[i]!);
        const soft = blur(shifted(matte, w, h, dx, dy, 1), w, h, (effect.size - shrink) * SIGMA_PER_SIZE);
        return [{ coverage: this.map((i) => soft[i]! * alpha[i]!), color: effect.color, blend: effect.blend, opacity: effect.opacity }];
      }

      case "bevel": {
        const size = Math.max(effect.size, 1e-3);
        const inner = effect.style === "inner";
        const sd = this.sd;
        // Chisel hard: the surface is a straight ramp of width `size` rising into the
        // shape, so its slope direction is the distance field's gradient. `band` is
        // the ramp's footprint, anti-aliased at both ends.
        const band = this.map((i) => (inner ? cover(sd[i]!) - cover(sd[i]! + size) : cover(sd[i]! - size) - cover(sd[i]!)));
        const [lx, ly] = lightDir(effect.angle);
        const alt = (effect.altitude * Math.PI) / 180;
        const light = [Math.cos(alt) * lx, Math.cos(alt) * ly, Math.sin(alt)] as const;
        const flat = light[2];
        const slope = effect.depth * (effect.direction === "down" ? -1 : 1);
        let hi = new Float32Array(alpha.length);
        let lo = new Float32Array(alpha.length);
        for (let y = 0; y < h; y++) {
          for (let x = 0; x < w; x++) {
            const i = y * w + x;
            if (band[i]! <= 0) continue;
            // Height falls as distance grows, so the normal leans along +grad(sd).
            const nx = ((sd[y * w + Math.min(x + 1, w - 1)]! - sd[y * w + Math.max(x - 1, 0)]!) / 2) * slope;
            const ny = ((sd[Math.min(y + 1, h - 1) * w + x]! - sd[Math.max(y - 1, 0) * w + x]!) / 2) * slope;
            const lit = (nx * light[0] + ny * light[1] + light[2]) / Math.hypot(nx, ny, 1);
            if (lit > flat) hi[i] = band[i]! * Math.min(1, (lit - flat) / Math.max(1 - flat, 1e-3));
            else lo[i] = band[i]! * Math.min(1, (flat - lit) / Math.max(flat, 1e-3));
          }
        }
        hi = blur(hi, w, h, effect.soften * SIGMA_PER_SIZE) as Float32Array<ArrayBuffer>;
        lo = blur(lo, w, h, effect.soften * SIGMA_PER_SIZE) as Float32Array<ArrayBuffer>;
        const within = (i: number) => (inner ? alpha[i]! : 1 - alpha[i]!);
        return [
          { coverage: this.map((i) => lo[i]! * within(i)), color: effect.shadow.color, blend: effect.shadow.blend, opacity: effect.shadow.opacity },
          { coverage: this.map((i) => hi[i]! * within(i)), color: effect.highlight.color, blend: effect.highlight.blend, opacity: effect.highlight.opacity },
        ];
      }
    }
  }
}

/**
 * Photoshop's stacking, bottom to top. Effects in `BELOW` are painted under
 * the layer's own pixels and the rest over them.
 */
export const BELOW: readonly Effect["type"][] = ["dropShadow", "outerGlow"];
export const ABOVE: readonly Effect["type"][] = ["gradientOverlay", "colorOverlay", "innerShadow", "bevel", "stroke"];

/** Enabled effects of the given types in paint order. Within a type, the first listed ends up on top. */
export function ordered(effects: Effect[], types: readonly Effect["type"][]): Effect[] {
  return types.flatMap((type) => effects.filter((e) => e.type === type && e.enabled).reverse());
}
