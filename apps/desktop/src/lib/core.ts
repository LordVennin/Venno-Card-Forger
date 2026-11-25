/**
 * The Rust core, loaded as WebAssembly (crates/wasm).
 *
 * The core owns the document, its history and its pixels. This wrapper only
 * moves small things across the boundary (JSON, file bytes, pointer positions)
 * and exposes the on-screen view as a window onto the module's memory, so
 * pixels are never copied between processes. It has no DOM dependencies and
 * also runs in Node.
 */
import type { Doc } from "@cardsmith/doc";

type Exports = {
  memory: WebAssembly.Memory;
  cs_alloc(len: number): number;
  cs_free(ptr: number, len: number): void;
  cs_out_ptr(): number;
  cs_out_len(): number;
  cs_sample_names(): void;
  cs_open_sample(index: number): number;
  cs_open_cardsmith(ptr: number, len: number): number;
  cs_open_json(jsonPtr: number, jsonLen: number, assetsPtr: number, assetsLen: number): number;
  cs_open_template(docPtr: number, docLen: number, manifestPtr: number, manifestLen: number): number;
  cs_missing_assets(): void;
  cs_add_asset(idPtr: number, idLen: number, ptr: number, len: number, redraw: number): number;
  cs_set_art(ptr: number, len: number): number;
  cs_build_card(ptr: number, len: number): number;
  cs_add_fill(build: number, ptr: number, len: number, sourcePtr: number, sourceLen: number, paramsPtr: number, paramsLen: number): number;
  cs_hit(x: number, y: number): void;
  cs_layer_box(ptr: number, len: number): number;
  cs_drag(ptr: number, len: number, scale: number, tx: number, ty: number): number;
  cs_drag_end(keep: number, labelPtr: number, labelLen: number): number;
  cs_font_names(): void;
  cs_save(): number;
  cs_doc_json(): number;
  cs_state_json(): void;
  cs_execute(editPtr: number, editLen: number, labelPtr: number, labelLen: number): number;
  cs_undo(): number;
  cs_redo(): number;
  cs_add_font(ptr: number, len: number): number;
  cs_add_paint_layer(namePtr: number, nameLen: number): number;
  cs_stroke_begin(layerPtr: number, layerLen: number, brushPtr: number, brushLen: number): number;
  cs_stroke_to(x: number, y: number, pressure: number): void;
  cs_stroke_end(labelPtr: number, labelLen: number): void;
  cs_set_view(scale: number, x: number, y: number, w: number, h: number): void;
  cs_view_ptr(): number;
  cs_dirty_x(): number;
  cs_dirty_y(): number;
  cs_dirty_w(): number;
  cs_dirty_h(): number;
  cs_frame_ptr(): number;
  cs_frame_hash_hi(): number;
  cs_frame_hash_lo(): number;
};

/** Where the art sits and what of it to trust, in document pixels, for whoever makes the fill. */
export type FillInfo = { artPosition: [number, number]; artScale: number; canvas: [number, number]; keep: [number, number, number, number] };
/** What building a card reports. `build` names it; a fill made for it is given back with that number. `fillKept`: the art has not moved, so the fill already there was kept. */
export type BuildReport = { build: number; warnings: string[]; fill: FillInfo | null; fillKept: boolean };
/**
 * What to build: Scryfall's JSON for the card (as looked up, or as edited in
 * the card form) and the name of the face wanted, or card data already read from it.
 */
export type BuildRequest = {
  scryfall?: Record<string, unknown>;
  name?: string;
  card?: Record<string, unknown>;
  symbolSvg?: string | null;
  watermarkSvg?: string | null;
  setSize?: number | null;
  removeFlavor?: boolean;
  removeReminder?: boolean;
  hideFlavorDivider?: boolean;
  verticalFullart?: boolean;
};

export type CoreState = { canUndo: boolean; canRedo: boolean; labels: string[]; warnings: string[] };

/** `scale` screen pixels per document pixel, with document point (x, y) at the view's top-left. */
export type View = { scale: number; x: number; y: number; width: number; height: number };
export type Rect = { x: number; y: number; width: number; height: number };

export type Brush = {
  /** Diameter in document pixels. */
  size: number;
  /** 0..1. */
  hardness: number;
  /** 0..1, for the whole stroke. */
  opacity: number;
  /** sRGB, 0..255. */
  color: [number, number, number];
  erase: boolean;
};

/** An edit, as the core's `Edit` enum serializes. */
export type Edit =
  | { type: "setLayerProps"; id: string; props: Record<string, unknown> }
  | { type: "addLayer"; parent: string; layer: unknown; index?: number }
  | { type: "removeLayer"; id: string }
  | { type: "moveLayer"; id: string; parent: string; index: number }
  | { type: "setAsset"; id: string; asset?: unknown }
  | { type: "setDocProps"; props: Record<string, unknown> }
  | { type: "batch"; edits: Edit[] };

const encoder = new TextEncoder();
const decoder = new TextDecoder();

export class Core {
  private view: View = { scale: 1, x: 0, y: 0, width: 0, height: 0 };

  private constructor(private x: Exports) {}

  static async load(wasm: BufferSource): Promise<Core> {
    const { instance } = await WebAssembly.instantiate(wasm, {});
    return new Core(instance.exports as unknown as Exports);
  }

  /** Copy bytes into the module, call `f` with their location, then free them. */
  private pass<T>(data: Uint8Array, f: (ptr: number, len: number) => T): T {
    const ptr = this.x.cs_alloc(data.length);
    new Uint8Array(this.x.memory.buffer, ptr, data.length).set(data);
    try {
      return f(ptr, data.length);
    } finally {
      this.x.cs_free(ptr, data.length);
    }
  }

  private passText<T>(text: string, f: (ptr: number, len: number) => T): T {
    return this.pass(encoder.encode(text), f);
  }

  private out(): Uint8Array {
    return new Uint8Array(this.x.memory.buffer, this.x.cs_out_ptr(), this.x.cs_out_len());
  }

  /** Throw the core's error message if a call reported failure. */
  private check(ok: number): void {
    if (!ok) throw new Error(decoder.decode(this.out()));
  }

  // ---- opening and saving ------------------------------------------------------

  sampleNames(): string[] {
    this.x.cs_sample_names();
    return JSON.parse(decoder.decode(this.out()));
  }

  openSample(index: number): void {
    this.check(this.x.cs_open_sample(index));
  }

  openCardsmith(file: Uint8Array): void {
    this.check(this.pass(file, (ptr, len) => this.x.cs_open_cardsmith(ptr, len)));
  }

  openJson(doc: string, assets: Record<string, Uint8Array> = {}): void {
    // Assets are packed as [id length][id][data length][data], lengths as 32-bit little-endian.
    const parts: Uint8Array[] = [];
    const length = (n: number) => new Uint8Array(new Uint32Array([n]).buffer);
    for (const [id, data] of Object.entries(assets)) {
      const name = encoder.encode(id);
      parts.push(length(name.length), name, length(data.length), data);
    }
    const packed = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
    let at = 0;
    for (const part of parts) {
      packed.set(part, at);
      at += part.length;
    }
    this.check(this.passText(doc, (jsonPtr, jsonLen) => this.pass(packed, (ptr, len) => this.x.cs_open_json(jsonPtr, jsonLen, ptr, len))));
  }

  // ---- cards --------------------------------------------------------------------

  /** Open a template package (its doc.json and manifest.json) to build a card on. Its images come later, through `missingAssets`. */
  openTemplate(doc: string, manifest: string): void {
    this.check(this.passText(doc, (docPtr, docLen) => this.passText(manifest, (ptr, len) => this.x.cs_open_template(docPtr, docLen, ptr, len))));
  }

  /** The images that layers now showing need and that have not been supplied. */
  missingAssets(): { id: string; path: string }[] {
    this.x.cs_missing_assets();
    return JSON.parse(decoder.decode(this.out()));
  }

  /** Supply an image. Pass `redraw` false for all but the last of several. */
  addAsset(id: string, data: Uint8Array, redraw = true): void {
    this.check(this.passText(id, (idPtr, idLen) => this.pass(data, (ptr, len) => this.x.cs_add_asset(idPtr, idLen, ptr, len, redraw ? 1 : 0))));
  }

  /** Use a picture as the card's art from the next build on. Returns its size. */
  setArt(file: Uint8Array): { width: number; height: number } {
    this.check(this.pass(file, (ptr, len) => this.x.cs_set_art(ptr, len)));
    return JSON.parse(decoder.decode(this.out()));
  }

  /** Build the card into the document as one undoable step, replacing the card that is showing. */
  buildCard(request: BuildRequest): BuildReport {
    this.check(this.passText(JSON.stringify(request), (ptr, len) => this.x.cs_build_card(ptr, len)));
    return JSON.parse(decoder.decode(this.out()));
  }

  /** Add a finished fill (PNG) made for a build. False, changing nothing, if that card is no longer showing. */
  addFill(build: number, png: Uint8Array, source: string, params: Record<string, unknown>): boolean {
    const ok = this.pass(png, (ptr, len) =>
      this.passText(source, (sourcePtr, sourceLen) => this.passText(JSON.stringify(params), (paramsPtr, paramsLen) => this.x.cs_add_fill(build, ptr, len, sourcePtr, sourceLen, paramsPtr, paramsLen))),
    );
    this.check(ok);
    return decoder.decode(this.out()) === "true";
  }

  /** The document as `.cardsmith` file bytes. */
  save(): Uint8Array {
    this.check(this.x.cs_save());
    return this.out().slice();
  }

  // ---- state -------------------------------------------------------------------

  doc(): Doc {
    this.check(this.x.cs_doc_json());
    return JSON.parse(decoder.decode(this.out()));
  }

  state(): CoreState {
    this.x.cs_state_json();
    return JSON.parse(decoder.decode(this.out()));
  }

  // ---- edits -------------------------------------------------------------------

  /** Apply an edit. Throws, leaving the document unchanged, if the core refuses it. */
  execute(edit: Edit, label: string): void {
    const ok = this.passText(JSON.stringify(edit), (editPtr, editLen) =>
      this.passText(label, (labelPtr, labelLen) => this.x.cs_execute(editPtr, editLen, labelPtr, labelLen)),
    );
    this.check(ok);
  }

  undo(): boolean {
    return this.x.cs_undo() === 1;
  }

  redo(): boolean {
    return this.x.cs_redo() === 1;
  }

  // ---- arranging ----------------------------------------------------------------

  /** The topmost showing layer with ink at a document point, or null. */
  hit(x: number, y: number): string | null {
    this.x.cs_hit(x, y);
    return decoder.decode(this.out()) || null;
  }

  /** A layer's box in document pixels, without the reach of its effects. Null if it shows nothing. */
  layerBox(id: string): Rect | null {
    this.check(this.passText(id, (ptr, len) => this.x.cs_layer_box(ptr, len)));
    return JSON.parse(decoder.decode(this.out()));
  }

  /**
   * Show a layer moved and scaled, as a drag in progress: every point p of it,
   * as it was when the drag began, goes to scale * p + (tx, ty). Call as often
   * as the pointer moves, then `dragEnd`.
   */
  drag(id: string, scale: number, tx: number, ty: number): void {
    this.check(this.passText(id, (ptr, len) => this.x.cs_drag(ptr, len, scale, tx, ty)));
  }

  /** Finish a drag: keep it as one undoable step, or put the layer back. */
  dragEnd(keep: boolean, label = "Move"): void {
    this.check(this.passText(label, (ptr, len) => this.x.cs_drag_end(keep ? 1 : 0, ptr, len)));
  }

  /** The PostScript names of the loaded fonts. */
  fontNames(): string[] {
    this.x.cs_font_names();
    return JSON.parse(decoder.decode(this.out()));
  }

  // ---- painting ----------------------------------------------------------------

  /**
   * Load a font file (TTF or OTF) for text layers to use, and redraw. Returns
   * the PostScript names it provides. Fonts stay loaded when another document is opened.
   */
  addFont(file: Uint8Array): string[] {
    this.check(this.pass(file, (ptr, len) => this.x.cs_add_font(ptr, len)));
    return JSON.parse(decoder.decode(this.out())) as string[];
  }

  /** Add an empty paint layer on top and return its id. */
  addPaintLayer(name: string): string {
    this.check(this.passText(name, (ptr, len) => this.x.cs_add_paint_layer(ptr, len)));
    return decoder.decode(this.out());
  }

  strokeBegin(layerId: string, brush: Brush): void {
    const ok = this.passText(layerId, (layerPtr, layerLen) =>
      this.passText(JSON.stringify(brush), (brushPtr, brushLen) => this.x.cs_stroke_begin(layerPtr, layerLen, brushPtr, brushLen)),
    );
    this.check(ok);
  }

  /** Extend the stroke to a document position. `pressure` (0..1) scales the brush size. */
  strokeTo(x: number, y: number, pressure = 1): void {
    this.x.cs_stroke_to(x, y, pressure);
  }

  strokeEnd(label: string): void {
    this.passText(label, (ptr, len) => this.x.cs_stroke_end(ptr, len));
  }

  // ---- pixels ------------------------------------------------------------------

  setView(view: View): void {
    this.view = view;
    this.x.cs_set_view(view.scale, view.x, view.y, view.width, view.height);
  }

  /**
   * The on-screen view: a window onto the module's memory, not a copy. Use it
   * (for example with `putImageData`) before calling the core again, and do not
   * keep it.
   */
  viewPixels(): Uint8ClampedArray<ArrayBuffer> {
    // A WebAssembly.Memory that is not shared is always backed by a plain ArrayBuffer.
    const buffer = this.x.memory.buffer as ArrayBuffer;
    return new Uint8ClampedArray(buffer, this.x.cs_view_ptr(), this.view.width * this.view.height * 4);
  }

  /** The part of the view the last call redrew. Null if nothing changed. */
  dirty(): Rect | null {
    const width = this.x.cs_dirty_w();
    const height = this.x.cs_dirty_h();
    return width > 0 && height > 0 ? { x: this.x.cs_dirty_x(), y: this.x.cs_dirty_y(), width, height } : null;
  }

  /** Hash of the full-resolution frame, in the same format as `cardsmith hashes`. */
  frameHash(): string {
    const part = (n: number) => (n >>> 0).toString(16).padStart(8, "0");
    return part(this.x.cs_frame_hash_hi()) + part(this.x.cs_frame_hash_lo());
  }
}
