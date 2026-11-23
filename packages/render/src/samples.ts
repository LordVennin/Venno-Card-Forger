/**
 * Hand-written sample documents. They exercise every feature the renderer
 * supports and are what the tests, the app's start screen and the Node/app
 * parity check all render.
 */
import {
  emptyDoc, group, pixel, rectPath, shape, type AssetBytes, type BlendMode, type Color, type Doc, type Effect, type Layer,
} from "@cardsmith/doc";
import type { CanvasKit } from "canvaskit-wasm";
import { encodePng } from "./render.js";

export type Sample = { name: string; doc: Doc; assets: AssetBytes };

function image(ck: CanvasKit, w: number, h: number, f: (x: number, y: number) => [number, number, number, number]): Uint8Array {
  const data = new Uint8Array(w * h * 4);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) data.set(f(x, y), (y * w + x) * 4);
  return encodePng(ck, w, h, data);
}

const roundRect = (x: number, y: number, w: number, h: number, r: number) =>
  `M${x + r} ${y}H${x + w - r}A${r} ${r} 0 0 1 ${x + w} ${y + r}V${y + h - r}A${r} ${r} 0 0 1 ${x + w - r} ${y + h}` +
  `H${x + r}A${r} ${r} 0 0 1 ${x} ${y + h - r}V${y + r}A${r} ${r} 0 0 1 ${x + r} ${y}Z`;
const circle = (cx: number, cy: number, r: number) => `M${cx - r} ${cy}A${r} ${r} 0 1 0 ${cx + r} ${cy}A${r} ${r} 0 1 0 ${cx - r} ${cy}Z`;

const BLENDS: BlendMode[] = ["normal", "multiply", "screen", "overlay", "softLight", "linearLight", "luminosity"];

function compositing(ck: CanvasKit): Sample {
  const W = 420, H = 300;
  const doc = emptyDoc(W, H);
  const assets: AssetBytes = {
    // A colorful backdrop so blend modes have something to work on.
    backdrop: image(ck, W, H, (x, y) => [Math.round((x / W) * 255), Math.round((y / H) * 255), 160, 255]),
    // Checkerboard with transparent squares.
    checker: image(ck, 64, 64, (x, y) => ((x >> 3) + (y >> 3)) % 2 ? [250, 210, 60, 255] : [0, 0, 0, 0]),
    // Horizontal ramp, used as a raster mask.
    ramp: image(ck, 100, 60, (x) => { const v = Math.round((x / 99) * 255); return [v, v, v, 255]; }),
  };
  for (const id of Object.keys(assets)) doc.assets[id] = { kind: "image", path: `assets/${id}.png` };

  const swatches: Layer[] = BLENDS.map((blend, i) => ({
    ...shape({ id: `blend-${blend}`, d: roundRect(12 + i * 58, 14, 50, 70, 8), color: [0.85, 0.45, 0.2, 1] }),
    blend,
  }));
  const orange: Color = [0.95, 0.5, 0.1, 1];

  doc.root.children = [
    pixel({ id: "backdrop", asset: "backdrop" }),
    group({ id: "swatches", name: "Blend modes" }, swatches),

    // Clipping: two layers clipped to a circle, one with a blend mode.
    shape({ id: "clip-base", d: circle(60, 150, 42), color: [0.2, 0.2, 0.25, 0.85] }),
    { ...pixel({ id: "clip-checker", asset: "checker", transform: [1.5, 0, 12, 0, 1.5, 102] }), clip: true },
    { ...shape({ id: "clip-tint", d: rectPath(18, 150, 84, 42), color: [0.3, 0.7, 1, 1] }), clip: true, blend: "overlay" },

    // Masks: raster ramp, vector circle, and both with the vector one inverted.
    { ...shape({ id: "mask-raster", d: rectPath(120, 110, 100, 60), color: orange }), mask: { asset: "ramp", offset: { x: 120, y: 110 } } },
    { ...shape({ id: "mask-vector", d: rectPath(230, 110, 80, 60), color: orange }), vectorMask: { path: { d: circle(270, 140, 36) } } },
    {
      ...shape({ id: "mask-both", d: rectPath(320, 110, 90, 60), color: orange }),
      mask: { asset: "ramp", offset: { x: 315, y: 110 }, inverted: true },
      vectorMask: { path: { d: circle(365, 140, 16) }, inverted: true },
    },

    // Groups: the same multiply child, pass-through, isolated, and pass-through at 50%.
    ...([["pass", true, 1], ["isolated", false, 1], ["faded", true, 0.5]] as const).map(([name, passThrough, opacity], i) =>
      group({ id: `group-${name}`, passThrough, opacity }, [
        shape({ id: `group-${name}-a`, d: roundRect(120 + i * 100, 190, 60, 60, 10), color: [0.9, 0.9, 0.3, 1] }),
        { ...shape({ id: `group-${name}-b`, d: circle(170 + i * 100, 240, 30), color: [0.4, 0.8, 0.9, 1] }), blend: "multiply" as const },
      ]),
    ),

    { ...shape({ id: "stroked-shape", d: roundRect(20, 215, 80, 60, 14), color: [1, 1, 1, 0.6], stroke: { color: [0.1, 0.1, 0.3, 1], width: 5, align: "inside" } }), fillOpacity: 0.7 },
  ];
  return { name: "compositing", doc, assets };
}

function effects(): Sample {
  const W = 420, H = 300;
  const doc = emptyDoc(W, H);
  const paint = { enabled: true, blend: "normal", opacity: 1 } as const;
  const gold: Color = [0.84, 0.67, 0.32, 1];
  const ink: Color = [0.05, 0.05, 0.1, 1];
  const cell = (i: number) => ({ x: 20 + (i % 4) * 100, y: 20 + Math.floor(i / 4) * 95 });
  const tile = (i: number, id: string, fx: Effect[], extra: Partial<Layer> = {}): Layer => {
    const { x, y } = cell(i);
    return { ...shape({ id, d: roundRect(x, y, 78, 68, 16), color: gold }), effects: fx, ...extra } as Layer;
  };
  const bevel: Effect = {
    type: "bevel", enabled: true, style: "inner", technique: "chiselHard", direction: "up", depth: 1, size: 8, soften: 0, angle: 120, altitude: 30,
    highlight: { color: [1, 1, 1, 1], blend: "screen", opacity: 0.75 },
    shadow: { color: [0, 0, 0, 1], blend: "multiply", opacity: 0.75 },
  };
  const rainbow = { stops: [{ offset: 0, color: [0.9, 0.2, 0.2, 1] as Color }, { offset: 0.5, color: [0.95, 0.9, 0.3, 1] as Color }, { offset: 1, color: [0.2, 0.4, 0.9, 1] as Color }] };

  doc.root.children = [
    shape({ id: "paper", d: rectPath(0, 0, W, H), color: [0.93, 0.92, 0.88, 1] }),
    tile(0, "stroke-outside", [{ ...paint, type: "stroke", color: ink, size: 5, position: "outside" }]),
    tile(1, "stroke-inside-center", [
      { ...paint, type: "stroke", color: ink, size: 4, position: "inside" },
      { ...paint, type: "stroke", color: [1, 1, 1, 1], size: 10, position: "center" },
    ]),
    tile(2, "drop-shadow", [{ ...paint, type: "dropShadow", color: [0, 0, 0, 1], blend: "multiply", opacity: 0.7, angle: 120, distance: 7, spread: 0.2, size: 10 }]),
    tile(3, "outer-glow", [{ ...paint, type: "outerGlow", color: [0.1, 0.3, 0.9, 1], opacity: 0.9, spread: 0.1, size: 12 }]),
    tile(4, "inner-shadow", [{ ...paint, type: "innerShadow", color: [0, 0, 0, 1], blend: "multiply", opacity: 0.7, angle: 120, distance: 5, choke: 0, size: 8 }]),
    tile(5, "bevel-inner", [bevel]),
    tile(6, "bevel-outer", [{ ...bevel, style: "outer", size: 6, soften: 2 }]),
    tile(7, "color-overlay", [{ ...paint, type: "colorOverlay", color: [0.3, 0.6, 0.4, 1], blend: "multiply" }]),
    tile(8, "gradient-linear", [{ ...paint, type: "gradientOverlay", gradient: rainbow, style: "linear", angle: 45, scale: 1 }]),
    tile(9, "gradient-radial", [{ ...paint, type: "gradientOverlay", gradient: rainbow, style: "radial", angle: 0, scale: 1, blend: "luminosity" }]),
    tile(10, "fill-zero", [
      { ...paint, type: "stroke", color: ink, size: 3, position: "inside" },
      { ...paint, type: "dropShadow", color: [0, 0, 0, 1], opacity: 0.6, angle: 120, distance: 5, spread: 0, size: 6 },
    ], { fillOpacity: 0 }),
    tile(11, "everything", [
      { ...paint, type: "dropShadow", color: [0, 0, 0, 1], blend: "multiply", opacity: 0.6, angle: 120, distance: 6, spread: 0, size: 8 },
      { ...paint, type: "gradientOverlay", gradient: rainbow, style: "linear", angle: 90, scale: 1, opacity: 0.6 },
      bevel,
      { ...paint, type: "stroke", color: ink, size: 3, position: "outside" },
    ], { opacity: 0.9 }),
  ];
  return { name: "effects", doc, assets: {} };
}

export function buildSamples(ck: CanvasKit): Sample[] {
  return [compositing(ck), effects()];
}
