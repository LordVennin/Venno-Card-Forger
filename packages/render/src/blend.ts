import type { BlendMode } from "@cardsmith/doc";
import type { Blender, CanvasKit, Paint, RuntimeEffect } from "canvaskit-wasm";

/**
 * Blend formulas as an SkSL blender. Used where Skia has no built-in:
 * linear light, and every non-normal mode on a clipped layer ("atop": the
 * result keeps the backdrop's alpha exactly).
 *
 * Colors arrive premultiplied. The formulas are the W3C/PDF separable and
 * non-separable definitions, which are also Photoshop's.
 */
const SKSL = `
uniform int mode;
uniform int atop;

float lum(vec3 c) { return dot(c, vec3(0.3, 0.59, 0.11)); }
vec3 clipColor(vec3 c) {
  float l = lum(c);
  float n = min(c.r, min(c.g, c.b));
  float x = max(c.r, max(c.g, c.b));
  if (n < 0.0) { c = l + (c - l) * l / (l - n); }
  if (x > 1.0) { c = l + (c - l) * (1.0 - l) / (x - l); }
  return c;
}
float softD(float d) { return d <= 0.25 ? ((16.0 * d - 12.0) * d + 4.0) * d : sqrt(d); }
float soft1(float s, float d) {
  return s <= 0.5 ? d - (1.0 - 2.0 * s) * d * (1.0 - d) : d + (2.0 * s - 1.0) * (softD(d) - d);
}
float hard1(float s, float d) { return s <= 0.5 ? 2.0 * s * d : 1.0 - 2.0 * (1.0 - s) * (1.0 - d); }

vec3 blend(vec3 s, vec3 d) {
  if (mode == 1) { return s * d; }
  if (mode == 2) { return s + d - s * d; }
  if (mode == 3) { return vec3(hard1(d.r, s.r), hard1(d.g, s.g), hard1(d.b, s.b)); }
  if (mode == 4) { return vec3(soft1(s.r, d.r), soft1(s.g, d.g), soft1(s.b, d.b)); }
  if (mode == 5) { return clamp(d + 2.0 * s - 1.0, 0.0, 1.0); }
  if (mode == 6) { return clipColor(d + (lum(s) - lum(d))); }
  return s;
}

vec4 main(vec4 src, vec4 dst) {
  vec3 s = src.a > 0.0 ? src.rgb / src.a : vec3(0.0);
  vec3 d = dst.a > 0.0 ? dst.rgb / dst.a : vec3(0.0);
  vec3 b = blend(s, d);
  if (atop == 1) {
    return vec4(dst.a * (src.a * b + (1.0 - src.a) * d), dst.a);
  }
  vec3 rgb = src.a * ((1.0 - dst.a) * s + dst.a * b) + (1.0 - src.a) * dst.rgb;
  return vec4(rgb, src.a + dst.a * (1.0 - src.a));
}
`;

const MODE_INDEX: Record<BlendMode, number> = {
  normal: 0,
  multiply: 1,
  screen: 2,
  overlay: 3,
  softLight: 4,
  linearLight: 5,
  luminosity: 6,
};

/** Owns the compiled blender and hands out configured paints. */
export class Blending {
  private effect: RuntimeEffect;
  private cache = new Map<string, Blender>();

  constructor(private ck: CanvasKit) {
    let error = "";
    const effect = ck.RuntimeEffect.MakeForBlender(SKSL, (e) => (error = e));
    if (!effect) throw new Error(`Blend shader failed to compile: ${error}`);
    this.effect = effect;
  }

  private custom(mode: BlendMode, atop: boolean): Blender {
    const key = `${mode}:${atop}`;
    let blender = this.cache.get(key);
    if (!blender) {
      blender = this.effect.makeBlender([MODE_INDEX[mode], atop ? 1 : 0]);
      this.cache.set(key, blender);
    }
    return blender;
  }

  /**
   * Set `paint` to composite with `mode`. With `atop`, the destination's alpha
   * is left unchanged (clipped layers). `forceCustom` is for tests that compare
   * the SkSL formulas with Skia's built-ins.
   */
  apply(paint: Paint, mode: BlendMode, atop = false, forceCustom = false): void {
    const { BlendMode: B } = this.ck;
    if (!forceCustom) {
      if (atop && mode === "normal") return paint.setBlendMode(B.SrcATop);
      if (!atop && mode !== "linearLight") {
        const native = {
          normal: B.SrcOver,
          multiply: B.Multiply,
          screen: B.Screen,
          overlay: B.Overlay,
          softLight: B.SoftLight,
          luminosity: B.Luminosity,
        }[mode];
        return paint.setBlendMode(native);
      }
    }
    paint.setBlender(this.custom(mode, atop));
  }

  dispose(): void {
    for (const blender of this.cache.values()) blender.delete();
    this.cache.clear();
    this.effect.delete();
  }
}
