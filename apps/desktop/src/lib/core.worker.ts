/**
 * The core, in a worker of its own.
 *
 * The WebAssembly core does everything on the thread it runs on, and some of
 * what it does takes seconds (decoding a template's images, drawing a card for
 * the first time, exporting). Run on the window's thread, each of those froze
 * the whole window: no typing, no scrolling, no repainting. Here it runs
 * beside the window instead. The window sends a call and gets the answer
 * back, with whatever part of the picture the call changed.
 *
 * The core itself is unchanged: still one thread, no clocks, no files.
 * Pixels go from here to the window as transferred buffers, which is a
 * hand-over, not a copy through the app's bridge to its native side.
 */
import { Core, type Rect } from "./core";
import wasmUrl from "../wasm/cardsmith.wasm?url";

/** The part of the frame a call changed: where it goes in the frame, and its pixels (straight RGBA). */
export type FramePart = { per: number; width: number; height: number; at: Rect; pixels: ArrayBuffer };
export type Call = { id: number; method: string; args: unknown[] };
/** `on` says which frame is on display after the call, whether or not any of it changed. */
export type Reply = { id: number; result?: unknown; error?: string; frame?: FramePart; on?: number };

let core: Core | undefined;
const ready = fetch(wasmUrl)
  .then((response) => response.arrayBuffer())
  .then((bytes) => Core.load(bytes))
  .then((loaded) => {
    loaded.setDirect(true);
    core = loaded;
  });

/** Which frame is on display, and what changed in it since last asked, cut out of the core's memory. */
function changed(core: Core, reply: Reply): FramePart | undefined {
  const shown = core.display();
  reply.on = shown.per;
  const at = shown.dirty;
  if (!at || at.width <= 0 || at.height <= 0) return undefined;
  const out = new Uint8ClampedArray(at.width * at.height * 4);
  for (let y = 0; y < at.height; y++) {
    const from = ((at.y + y) * shown.width + at.x) * 4;
    out.set(shown.pixels.subarray(from, from + at.width * 4), y * at.width * 4);
  }
  return { per: shown.per, width: shown.width, height: shown.height, at, pixels: out.buffer };
}

/** Calls that are several of the core's in one, to save trips. */
const combined: Record<string, (core: Core, ...args: any[]) => unknown> = {
  /** Everything the panels show after an edit: the document, the history, the selected layer's box, the fonts. */
  snapshot: (core, active: string | null) => {
    const doc = core.doc();
    const state = core.state();
    let box: Rect | null = null;
    try {
      box = active ? core.layerBox(active) : null;
    } catch {
      // The layer is gone; the panel finds that out from the document.
    }
    return { doc, state, box, fonts: core.fontNames() };
  },
  /** Several images at once, drawn together when the last is in. */
  addAssets: (core, assets: { id: string; data: Uint8Array }[]) => assets.forEach((a, i) => core.addAsset(a.id, a.data, i === assets.length - 1)),
  /** A drag step and the dragged layer's new box. */
  dragStep: (core, id: string, scale: number, tx: number, ty: number, turn: number) => {
    core.drag(id, scale, tx, ty, turn);
    return core.layerBox(id);
  },
};

self.onmessage = async (event: MessageEvent<Call>) => {
  const { id, method, args } = event.data;
  const reply: Reply = { id };
  const transfer: Transferable[] = [];
  try {
    await ready;
    const made = method in combined ? combined[method]!(core!, ...args) : (core as unknown as Record<string, (...a: unknown[]) => unknown>)[method]!(...args);
    // Bytes made for the caller (a saved file, an exported picture) are handed over, not copied.
    reply.result = made;
    if (made instanceof Uint8Array) transfer.push(made.buffer);
    reply.frame = changed(core!, reply);
    if (reply.frame) transfer.push(reply.frame.pixels);
  } catch (err) {
    reply.error = err instanceof Error ? err.message : String(err);
    // Whatever was drawn before the call failed still has to reach the screen.
    if (core) reply.frame = changed(core, reply);
    if (reply.frame) transfer.push(reply.frame.pixels);
  }
  (self as unknown as Worker).postMessage(reply, transfer);
};
