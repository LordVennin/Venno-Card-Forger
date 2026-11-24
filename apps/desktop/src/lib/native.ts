/**
 * The app's native side (src-tauri): folders the user chose, Scryfall and
 * hexproof.io lookups, and fills. Only commands and files cross this bridge;
 * frames stay in the WebAssembly core.
 */
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { FillInfo } from "./core";

/** False in a plain browser (the parity tests, `vite` on its own), where there is no native side. */
export const hasNative = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export type Settings = { templatesDir: string; fontsDir: string };
export type TemplateEntry = { folder: string; name: string; buildable: boolean };
export type Printing = { set: string; setName: string; number: string; year: string; rarity: string; artist: string; thumbnail: string | null };
export type Extras = { setSize: number | null; symbolSvg: string | null; watermarkSvg: string | null };
/** Scryfall's JSON for a card with its layout settled, the face wanted, and what goes with its set. */
export type Found = { scryfall: Record<string, any>; name: string; extras: Extras };
export type TierState = { id: "sd" | "lama" | "patchmatch"; ok: boolean; reason: string | null };
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
  fillTiers: () => invoke<TierState[]>("fill_tiers"),
  fill: (artPath: string, info: FillInfo, tier: string, seed: number, prompt: string) => invoke<FillResult>("fill", { artPath, info, tier, seed, prompt }),
  fillImage: (ticket: number) => bytes("fill_image", { ticket }),

  async chooseFolder(title: string): Promise<string | null> {
    const picked = await open({ directory: true, title });
    return typeof picked === "string" ? picked : null;
  },
  async chooseArt(): Promise<string | null> {
    const picked = await open({ title: "Choose the card's art", filters: [{ name: "Pictures", extensions: ["png", "jpg", "jpeg", "webp"] }] });
    return typeof picked === "string" ? picked : null;
  },
};
