/**
 * The Rust core, loaded as WebAssembly (crates/wasm).
 *
 * The core owns the document, its history and the rendered frame. This wrapper
 * only moves small things across the boundary (JSON, file bytes) and exposes
 * the frame as a view over the module's memory, so pixels are never copied
 * between processes. It has no DOM dependencies and also runs in Node.
 */
import type { Doc } from "@cardsmith/doc";

type Exports = {
  memory: WebAssembly.Memory;
  cs_alloc(len: number): number;
  cs_free(ptr: number, len: number): void;
  cs_text_ptr(): number;
  cs_text_len(): number;
  cs_sample_names(): number;
  cs_open_sample(index: number): number;
  cs_open_cardsmith(ptr: number, len: number): number;
  cs_open_json(ptr: number, len: number): number;
  cs_set_asset_data(idPtr: number, idLen: number, ptr: number, len: number): number;
  cs_execute(editPtr: number, editLen: number, labelPtr: number, labelLen: number): number;
  cs_undo(): number;
  cs_redo(): number;
  cs_doc_json(): number;
  cs_state_json(): void;
  cs_render(): void;
  cs_frame_ptr(): number;
  cs_frame_width(): number;
  cs_frame_height(): number;
  cs_frame_hash_hi(): number;
  cs_frame_hash_lo(): number;
};

export type CoreState = { canUndo: boolean; canRedo: boolean; labels: string[]; warnings: string[] };
export type Frame = { width: number; height: number; pixels: Uint8ClampedArray<ArrayBuffer> };

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

  private text(): string {
    return decoder.decode(new Uint8Array(this.x.memory.buffer, this.x.cs_text_ptr(), this.x.cs_text_len()));
  }

  /** Throw the core's error message if a call reported failure. */
  private check(ok: number): void {
    if (!ok) throw new Error(this.text());
  }

  sampleNames(): string[] {
    this.x.cs_sample_names();
    return JSON.parse(this.text());
  }

  openSample(index: number): void {
    this.check(this.x.cs_open_sample(index));
  }

  openCardsmith(file: Uint8Array): void {
    this.check(this.pass(file, (ptr, len) => this.x.cs_open_cardsmith(ptr, len)));
  }

  openJson(doc: string, assets: Record<string, Uint8Array> = {}): void {
    this.check(this.pass(encoder.encode(doc), (ptr, len) => this.x.cs_open_json(ptr, len)));
    for (const [id, data] of Object.entries(assets)) {
      const ok = this.pass(encoder.encode(id), (idPtr, idLen) => this.pass(data, (ptr, len) => this.x.cs_set_asset_data(idPtr, idLen, ptr, len)));
      this.check(ok);
    }
  }

  /** Apply an edit. Throws, leaving the document unchanged, if the core refuses it. */
  execute(edit: Edit, label: string): void {
    const ok = this.pass(encoder.encode(JSON.stringify(edit)), (editPtr, editLen) =>
      this.pass(encoder.encode(label), (labelPtr, labelLen) => this.x.cs_execute(editPtr, editLen, labelPtr, labelLen)),
    );
    this.check(ok);
  }

  undo(): boolean {
    return this.x.cs_undo() === 1;
  }

  redo(): boolean {
    return this.x.cs_redo() === 1;
  }

  doc(): Doc {
    this.check(this.x.cs_doc_json());
    return JSON.parse(this.text());
  }

  state(): CoreState {
    this.x.cs_state_json();
    return JSON.parse(this.text());
  }

  /**
   * Render the document. The returned pixels are a view over the module's
   * memory: use them (for example with `putImageData`) before calling the core
   * again, and do not keep them.
   */
  render(): Frame {
    this.x.cs_render();
    const width = this.x.cs_frame_width();
    const height = this.x.cs_frame_height();
    // A WebAssembly.Memory that is not shared is always backed by a plain ArrayBuffer.
    const buffer = this.x.memory.buffer as ArrayBuffer;
    return { width, height, pixels: new Uint8ClampedArray(buffer, this.x.cs_frame_ptr(), width * height * 4) };
  }

  /** Hash of the last rendered frame, in the same format as `cardsmith hashes`. */
  frameHash(): string {
    const part = (n: number) => (n >>> 0).toString(16).padStart(8, "0");
    return part(this.x.cs_frame_hash_hi()) + part(this.x.cs_frame_hash_lo());
  }
}
