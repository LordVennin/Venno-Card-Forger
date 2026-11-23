/**
 * Document JSON and the `.cardsmith` save format: a zip holding `doc.json` and
 * every asset at the path its AssetRef names (normally under `assets/`).
 */
import { strFromU8, strToU8, unzipSync, zipSync } from "fflate";
import { walk } from "./tree.js";
import { BLEND_MODES, type AssetId, type Doc, type Layer } from "./types.js";

/** Asset bytes keyed by asset id. */
export type AssetBytes = Record<AssetId, Uint8Array>;

const KINDS = new Set(["group", "pixel", "shape", "text", "art", "fill"]);

function fail(message: string): never {
  throw new Error(`Invalid document: ${message}`);
}

function checkLayer(layer: Layer, doc: Doc, seen: Set<string>): void {
  const where = `layer "${layer?.id}"`;
  if (!layer || typeof layer !== "object") fail("a layer is not an object");
  if (typeof layer.id !== "string" || !layer.id) fail("a layer has no id");
  if (seen.has(layer.id)) fail(`duplicate layer id "${layer.id}"`);
  seen.add(layer.id);
  if (!KINDS.has(layer.kind)) fail(`${where} has unknown kind "${layer.kind}"`);
  if (typeof layer.visible !== "boolean") fail(`${where} needs a boolean "visible"`);
  if (typeof layer.clip !== "boolean") fail(`${where} needs a boolean "clip"`);
  if (!(layer.opacity >= 0 && layer.opacity <= 1)) fail(`${where} opacity must be 0..1`);
  if (!BLEND_MODES.includes(layer.blend)) fail(`${where} has unsupported blend mode "${layer.blend}"`);
  const needsAsset = (id: unknown, what: string) => {
    if (typeof id !== "string" || !doc.assets[id]) fail(`${where} ${what} refers to missing asset "${id}"`);
  };
  if (layer.kind === "pixel" || layer.kind === "fill") needsAsset(layer.asset, "image");
  if (layer.kind === "art" && layer.asset !== undefined) needsAsset(layer.asset, "image");
  if (layer.mask) needsAsset(layer.mask.asset, "mask");
  if (layer.kind === "group" && !Array.isArray(layer.children)) fail(`${where} is a group without children`);
}

/** Check the structure the renderer and commands rely on. Returns the same object. */
export function validateDoc(value: unknown): Doc {
  const doc = value as Doc;
  if (!doc || typeof doc !== "object") fail("not an object");
  if (doc.version !== 1) fail(`unsupported version ${JSON.stringify(doc.version)} (this build reads version 1)`);
  for (const key of ["width", "height", "dpi"] as const) {
    if (!(Number.isFinite(doc[key]) && doc[key] > 0)) fail(`"${key}" must be a positive number`);
  }
  if (!doc.assets || typeof doc.assets !== "object") fail(`"assets" is missing`);
  for (const [id, ref] of Object.entries(doc.assets)) {
    if (typeof ref?.path !== "string" || !ref.path) fail(`asset "${id}" has no path`);
    if (ref.path.startsWith("/") || ref.path.split(/[\\/]/).includes("..")) fail(`asset "${id}" has an unsafe path`);
  }
  if (doc.root?.kind !== "group") fail("root must be a group");
  const seen = new Set<string>();
  checkLayer(doc.root, doc, seen);
  walk(doc.root, (layer) => checkLayer(layer, doc, seen));
  return doc;
}

export function parseDoc(json: string): Doc {
  let value: unknown;
  try {
    value = JSON.parse(json);
  } catch (err) {
    fail(`not JSON (${(err as Error).message})`);
  }
  return validateDoc(value);
}

export function stringifyDoc(doc: Doc): string {
  return JSON.stringify(validateDoc(doc), null, 2) + "\n";
}

/** Pack a document and its asset bytes into `.cardsmith` zip bytes. */
export function saveCardsmith(doc: Doc, assets: AssetBytes): Uint8Array {
  const files: Record<string, Uint8Array | [Uint8Array, { level: 0 }]> = { "doc.json": strToU8(stringifyDoc(doc)) };
  for (const [id, ref] of Object.entries(doc.assets)) {
    const bytes = assets[id];
    if (!bytes) throw new Error(`Cannot save: no data for asset "${id}" (${ref.path})`);
    if (ref.path === "doc.json") throw new Error(`Asset "${id}" would overwrite doc.json`);
    // Images are already compressed; storing them is faster and no bigger.
    files[ref.path] = [bytes, { level: 0 }];
  }
  return zipSync(files);
}

/** Unpack `.cardsmith` zip bytes. Throws if an asset the document names is absent. */
export function loadCardsmith(bytes: Uint8Array): { doc: Doc; assets: AssetBytes } {
  let files: Record<string, Uint8Array>;
  try {
    files = unzipSync(bytes);
  } catch {
    throw new Error("Not a .cardsmith file (it is not a zip archive)");
  }
  const json = files["doc.json"];
  if (!json) throw new Error("Not a .cardsmith file (no doc.json inside)");
  const doc = parseDoc(strFromU8(json));
  const assets: AssetBytes = {};
  for (const [id, ref] of Object.entries(doc.assets)) {
    const data = files[ref.path];
    if (!data) throw new Error(`Save file is missing asset "${id}" (${ref.path})`);
    assets[id] = data;
  }
  return { doc, assets };
}
