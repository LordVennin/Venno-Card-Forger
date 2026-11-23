/** Photoshop layer effects -> our `Effect[]`. Only the seven types found by the survey. */
import type { BlendMode, Color, Effect, GradientStop } from "@cardsmith/doc";
import type { BlendMode as PsdBlend, Color as PsdColor, LayerEffectsInfo, UnitsValue } from "ag-psd";

export type Warn = (message: string) => void;

const BLEND: Partial<Record<PsdBlend, BlendMode>> = {
  normal: "normal",
  multiply: "multiply",
  screen: "screen",
  overlay: "overlay",
  "soft light": "softLight",
  "linear light": "linearLight",
  luminosity: "luminosity",
};

/** Map a Photoshop blend mode, warning about (and replacing) any the renderer does not have. */
export function blendMode(mode: string | undefined, where: string, warn: Warn): BlendMode {
  const mapped = BLEND[(mode ?? "normal") as PsdBlend];
  if (!mapped) warn(`${where}: blend mode "${mode}" is not supported; using normal`);
  return mapped ?? "normal";
}

export function color(c: PsdColor | undefined, alpha = 1): Color {
  const rgb = c as { r?: number; g?: number; b?: number; k?: number } | undefined;
  if (rgb && typeof rgb.r === "number") return [rgb.r / 255, rgb.g! / 255, rgb.b! / 255, alpha];
  // Grayscale colors carry a single `k`.
  if (rgb && typeof rgb.k === "number") return [rgb.k / 255, rgb.k / 255, rgb.k / 255, alpha];
  return [0, 0, 0, alpha];
}

const px = (v: UnitsValue | undefined) => v?.value ?? 0;
/** Spread and choke are percentages, whatever unit the file labels them with. */
const percent = (v: UnitsValue | undefined) => Math.min(1, Math.max(0, px(v) / 100));

type Global = { angle: number; altitude: number };

/**
 * Merge Photoshop's separate color and opacity stops into RGBA stops. Midpoints
 * other than 50% get an extra stop at the midpoint, which is what they mean.
 */
function gradientStops(g: { smoothness?: number; colorStops?: { color: PsdColor; location: number; midpoint: number }[]; opacityStops?: { opacity: number; location: number; midpoint: number }[] }): GradientStop[] {
  // Photoshop's "smoothness" eases each segment in and out instead of ramping linearly.
  const smoothness = Math.min(1, Math.max(0, g.smoothness ?? 0));
  const ease = (f: number) => f + (f * f * (3 - 2 * f) - f) * smoothness;
  const colors = [...(g.colorStops ?? [])].sort((a, b) => a.location - b.location);
  const opacities = [...(g.opacityStops ?? [])].sort((a, b) => a.location - b.location);
  const lerp = (a: number, b: number, t: number) => a + (b - a) * t;
  /** Value of a stop list at `t`, honouring each segment's midpoint. */
  const sample = <T>(stops: { location: number; midpoint: number }[], values: T[], t: number, mix: (a: T, b: T, f: number) => T): T => {
    if (t <= stops[0]!.location) return values[0]!;
    for (let i = 0; i + 1 < stops.length; i++) {
      const [a, b] = [stops[i]!, stops[i + 1]!];
      if (t > b.location) continue;
      const span = b.location - a.location;
      let f = span > 0 ? (t - a.location) / span : 1;
      const mid = b.midpoint > 0 && b.midpoint < 1 ? b.midpoint : 0.5;
      // The midpoint is where the blend is half-way; remap linearly on each side of it.
      f = f < mid ? (f / mid) * 0.5 : 0.5 + ((f - mid) / (1 - mid)) * 0.5;
      return mix(values[i]!, values[i + 1]!, ease(f));
    }
    return values[values.length - 1]!;
  };
  const rgb = colors.map((s) => color(s.color));
  const alpha = opacities.map((s) => s.opacity);
  const at = (t: number): Color => {
    const c = colors.length ? sample(colors, rgb, t, (a, b, f) => [lerp(a[0], b[0], f), lerp(a[1], b[1], f), lerp(a[2], b[2], f), 1] as Color) : ([0, 0, 0, 1] as Color);
    const a = opacities.length ? sample(opacities, alpha, t, lerp) : 1;
    return [c[0], c[1], c[2], a];
  };
  const locations = new Set<number>();
  for (const list of [colors, opacities]) {
    list.forEach((s, i) => {
      locations.add(s.location);
      const prev = list[i - 1];
      if (prev && Math.abs(s.midpoint - 0.5) > 1e-3) locations.add(prev.location + (s.location - prev.location) * s.midpoint);
    });
  }
  const sorted = [...locations].sort((a, b) => a - b);
  if (smoothness > 0) {
    // Our gradients are piecewise linear, so an eased segment is written as several short ones.
    const STEPS = 8;
    for (let i = sorted.length - 1; i > 0; i--) {
      const [a, b] = [sorted[i - 1]!, sorted[i]!];
      for (let k = STEPS - 1; k > 0; k--) sorted.splice(i, 0, a + ((b - a) * k) / STEPS);
    }
  }
  return sorted.map((offset) => ({ offset, color: at(offset) }));
}

/**
 * Convert a layer's effects. Effects Photoshop marks as not present are
 * dropped; disabled ones are kept, switched off, because templates turn them
 * on at render time.
 */
export function convertEffects(fx: LayerEffectsInfo, global: Global, where: string, warn: Warn): { effects: Effect[]; enabled: boolean } {
  const out: Effect[] = [];
  const list = <T extends { present?: boolean }>(v: T | T[] | undefined): T[] => (v === undefined ? [] : Array.isArray(v) ? v : [v]).filter((e) => e.present !== false);
  const paint = (e: { enabled?: boolean; blendMode?: string; opacity?: number }, name: string) => ({
    enabled: e.enabled !== false,
    blend: blendMode(e.blendMode, `${where} ${name}`, warn),
    opacity: e.opacity ?? 1,
  });
  const angle = (e: { useGlobalLight?: boolean; angle?: number }) => (e.useGlobalLight ? global.angle : (e.angle ?? global.angle));
  const contour = (e: { contour?: { name?: string } }, name: string) => {
    const kind = e.contour?.name ?? "Linear";
    if (kind !== "Linear" && kind !== "Gaussian") warn(`${where} ${name}: contour "${kind}" is not supported; using linear`);
    return kind === "Gaussian" ? ({ contour: "gaussian" } as const) : {};
  };

  for (const e of list(fx.dropShadow)) {
    out.push({ type: "dropShadow", ...paint(e, "drop shadow"), color: color(e.color), angle: angle(e), distance: px(e.distance), spread: percent(e.choke), size: px(e.size), ...contour(e, "drop shadow") });
    if (e.layerConceals === false) warn(`${where} drop shadow: "layer knocks out drop shadow" is off, which is not supported`);
  }
  for (const e of list(fx.outerGlow)) {
    if (e.gradient) warn(`${where} outer glow: gradient glows are not supported; using its color`);
    out.push({ type: "outerGlow", ...paint(e, "outer glow"), color: color(e.color), spread: percent(e.choke), size: px(e.size) });
  }
  for (const e of list(fx.gradientOverlay)) {
    const g = e.gradient;
    if (!g || g.type !== "solid") {
      warn(`${where} gradient overlay: only solid gradients are supported; skipped`);
      continue;
    }
    if (e.type !== "linear" && e.type !== "radial") warn(`${where} gradient overlay: style "${e.type}" is not supported; using linear`);
    const offset = e.offset && (e.offset.x || e.offset.y) ? { offset: { x: e.offset.x, y: e.offset.y } } : {};
    out.push({
      type: "gradientOverlay", ...paint(e, "gradient overlay"), gradient: { stops: gradientStops(g) },
      style: e.type === "radial" ? "radial" : "linear", angle: e.angle ?? 90, scale: e.scale ?? 1, ...(e.reverse ? { reverse: true } : {}), ...offset,
    });
  }
  for (const e of list(fx.solidFill)) out.push({ type: "colorOverlay", ...paint(e, "color overlay"), color: color(e.color) });
  for (const e of list(fx.innerShadow)) {
    out.push({ type: "innerShadow", ...paint(e, "inner shadow"), color: color(e.color), angle: angle(e), distance: px(e.distance), choke: percent(e.choke), size: px(e.size) });
    contour(e, "inner shadow");
  }
  for (const e of list(fx.bevel)) {
    const style = e.style === "outer bevel" ? "outer" : "inner";
    if (e.style !== "inner bevel" && e.style !== "outer bevel") warn(`${where} bevel: style "${e.style}" is not supported; using inner bevel`);
    if (e.technique !== "chisel hard") warn(`${where} bevel: technique "${e.technique}" is not supported; using chisel hard`);
    out.push({
      type: "bevel", enabled: e.enabled !== false, style, technique: "chiselHard", direction: e.direction === "down" ? "down" : "up",
      depth: e.strength ?? 1, size: px(e.size), soften: px(e.soften), angle: angle(e), altitude: e.useGlobalLight ? global.altitude : (e.altitude ?? global.altitude),
      highlight: { color: color(e.highlightColor), blend: blendMode(e.highlightBlendMode, `${where} bevel highlight`, warn), opacity: e.highlightOpacity ?? 1 },
      shadow: { color: color(e.shadowColor), blend: blendMode(e.shadowBlendMode, `${where} bevel shadow`, warn), opacity: e.shadowOpacity ?? 1 },
    });
  }
  for (const e of list(fx.stroke)) {
    if (e.fillType && e.fillType !== "color") warn(`${where} stroke: ${e.fillType} strokes are not supported; using its color`);
    out.push({ type: "stroke", ...paint(e, "stroke"), color: color(e.color), size: px(e.size), position: e.position ?? "outside" });
  }
  for (const [key, name] of [["innerGlow", "inner glow"], ["satin", "satin"], ["patternOverlay", "pattern overlay"]] as const) {
    if (list(fx[key] as { present?: boolean } | undefined).length) warn(`${where}: ${name} is not supported; skipped`);
  }
  return { effects: out, enabled: fx.disabled !== true };
}
