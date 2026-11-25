/**
 * The app's native side (src-tauri): folders the user chose, Scryfall and
 * hexproof.io lookups, and fills. Only commands and files cross this bridge;
 * frames stay in the WebAssembly core.
 */
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { FillInfo } from "./core";

/** False in a plain browser (the parity tests, `vite` on its own), where there is no native side. */
export const hasNative = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export type Settings = { templatesDir: string; fontsDir: string };
export type TemplateEntry = { folder: string; id: string; name: string; buildable: boolean };
export type Printing = { set: string; setName: string; number: string; year: string; rarity: string; artist: string; thumbnail: string | null };
export type Extras = { setSize: number | null; symbolSvg: string | null; watermarkSvg: string | null };
/** Scryfall's JSON for a card with its layout settled, the face wanted, and what goes with its set. */
export type Found = { scryfall: Record<string, any>; name: string; extras: Extras };
export type SetEntry = { code: string; name: string; released: string; kind: string };
export type TierState = { id: "sd" | "lama" | "patchmatch"; ok: boolean; reason: string | null };
/** A patch for a picture: where it goes in the picture's pixels, and how many of them one of its pixels covers. */
export type PatchResult = { ticket: number; provider: string; notices: string[]; ms: number; x: number; y: number; width: number; height: number; scale: number };
export type FillResult = { ticket: number | null; provider: string; notices: string[]; ms: number; workWidth: number };

const bytes = async (command: string, args: Record<string, unknown>) => new Uint8Array(await invoke<ArrayBuffer>(command, args));

export const native = {
  settings: () => invoke<Settings>("settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  templates: () => invoke<TemplateEntry[]>("templates"),
  template: (folder: string) => invoke<{ doc: string; manifest: string }>("template", { folder }),
  templateAsset: (folder: string, path: string) => bytes("template_asset", { folder, path }),
  fonts: () => invoke<string[]>("fonts"),
  font: (name: string) => bytes("font", { name }),
  artFile: (path: string) => bytes("art_file", { path }),
  autocomplete: (typed: string) => invoke<string[]>("autocomplete", { typed }),
  printings: (name: string) => invoke<Printing[]>("printings", { name }),
  card: (name: string, set?: string, number?: string) => invoke<Found>("card", { name, set, number }),
  setSymbol: (set: string, rarity: string, typeLine: string) => invoke<Extras>("set_symbol", { set, rarity, typeLine }),
  sets: () => invoke<SetEntry[]>("sets"),
  fillTiers: () => invoke<TierState[]>("fill_tiers"),
  fill: (artPath: string, info: FillInfo, tier: string, seed: number, prompt: string) => invoke<FillResult>("fill", { artPath, info, tier, seed, prompt }),
  fillImage: (ticket: number) => bytes("fill_image", { ticket }),
  /** Paint over a rectangle (left, top, right, bottom, in the picture's pixels) of a picture from its surroundings. */
  fillSelection: (picture: Uint8Array, hole: [number, number, number, number], tier: string, seed: number, prompt = "") =>
    invoke<PatchResult>("fill_selection", picture, { headers: { "x-args": encodeURIComponent(JSON.stringify({ hole, tier, seed, prompt })) } }),

  /** Ask where to save a picture, then write it there. Returns the path, or null if the user backed out. */
  async savePng(suggested: string, data: () => Uint8Array): Promise<string | null> {
    const path = await save({ title: "Export the card as a picture", defaultPath: suggested, filters: [{ name: "PNG picture", extensions: ["png"] }] });
    if (!path) return null;
    await native.writeFile(path, data());
    return path;
  },

  /** Ask where to save the document, then write it there. Returns the path, or null if the user backed out. */
  async saveDocument(suggested: string, data: () => Uint8Array): Promise<string | null> {
    const path = await save({ title: "Save the card", defaultPath: suggested, filters: [{ name: "Cardsmith document", extensions: ["cardsmith"] }] });
    if (!path) return null;
    await native.writeFile(path, data());
    return path;
  },

  /** Ask for a saved document and read it. Null if the user backed out. */
  async openDocument(): Promise<{ path: string; data: Uint8Array } | null> {
    const path = await open({ title: "Open a card", filters: [{ name: "Cardsmith document", extensions: ["cardsmith"] }] });
    return typeof path === "string" ? { path, data: await bytes("document_file", { path }) } : null;
  },

  /** Write bytes to a file. The bytes go as the request's body, not as JSON. */
  writeFile: (path: string, data: Uint8Array) => invoke<void>("write_file", data, { headers: { "x-path": encodeURIComponent(path) } }),

  async chooseFolder(title: string): Promise<string | null> {
    const picked = await open({ directory: true, title });
    return typeof picked === "string" ? picked : null;
  },
  async chooseArt(): Promise<string | null> {
    const picked = await open({ title: "Choose the card's art", filters: [{ name: "Pictures", extensions: ["png", "jpg", "jpeg", "webp"] }] });
    return typeof picked === "string" ? picked : null;
  },
};
