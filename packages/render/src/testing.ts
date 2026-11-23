/** Helpers for building tiny documents and reading pixels in tests. */
import type { AssetBytes, Color, Doc, Layer } from "@cardsmith/doc";
import { emptyDoc } from "@cardsmith/doc";
import type { CanvasKit } from "canvaskit-wasm";
import { encodePng, type RenderResult } from "./render.js";

export const RED: Color = [1, 0, 0, 1];
export const GREEN: Color = [0, 1, 0, 1];
export const BLUE: Color = [0, 0, 1, 1];
export const WHITE: Color = [1, 1, 1, 1];
export const BLACK: Color = [0, 0, 0, 1];

/** PNG bytes from a function giving straight-alpha RGBA (0..255) per pixel. */
export function png(ck: CanvasKit, w: number, h: number, f: (x: number, y: number) => [number, number, number, number]): Uint8Array {
  const data = new Uint8Array(w * h * 4);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) data.set(f(x, y), (y * w + x) * 4);
  return encodePng(ck, w, h, data);
}

/** A document plus its assets, built from a list of layers (bottom first). */
export function scene(width: number, height: number, layers: Layer[], images: Record<string, Uint8Array> = {}): { doc: Doc; assets: AssetBytes } {
  const doc = emptyDoc(width, height);
  doc.root.children = layers;
  for (const id of Object.keys(images)) doc.assets[id] = { kind: "image", path: `assets/${id}.png` };
  return { doc, assets: images };
}

export function pixelAt(result: RenderResult, x: number, y: number): number[] {
  const i = (y * result.width + x) * 4;
  return [...result.pixels.slice(i, i + 4)];
}

/** Largest per-channel difference between two renders of the same size. */
export function maxDiff(a: RenderResult, b: RenderResult): number {
  if (a.width !== b.width || a.height !== b.height) return 255;
  let max = 0;
  for (let i = 0; i < a.pixels.length; i++) max = Math.max(max, Math.abs(a.pixels[i]! - b.pixels[i]!));
  return max;
}
