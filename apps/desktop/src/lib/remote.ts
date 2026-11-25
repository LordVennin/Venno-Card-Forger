/**
 * The window's end of the core's worker: every method of `Core`, answering
 * with a promise, plus the few calls that are several of the core's in one.
 * Calls are answered in the order they were made.
 */
import type { Doc } from "@cardsmith/doc";
import type { Core, CoreState, Rect } from "./core";
import type { Call, FramePart, Reply } from "./core.worker";

export type { FramePart };

type Async<T> = { [K in keyof T]: T[K] extends (...args: infer A) => infer R ? (...args: A) => Promise<R> : never };

export type RemoteCore = Async<Omit<Core, "display" | "viewPixels" | "dirty" | "setDirect">> & {
  /** Everything the panels show after an edit, in one trip. */
  snapshot(active: string | null): Promise<{ doc: Doc; state: CoreState; box: Rect | null; fonts: string[] }>;
  /** Several images at once, drawn together when the last is in. */
  addAssets(assets: { id: string; data: Uint8Array }[]): Promise<void>;
  /** A drag step (as `drag`) and the dragged layer's box after it. */
  dragStep(id: string, scale: number, tx: number, ty: number, turn: number): Promise<Rect | null>;
  /** How many calls are waiting for an answer. */
  readonly pending: number;
};

/**
 * Start the core in its worker. `onFrame` is given each part of the picture
 * a call changed, before that call's promise settles.
 */
export function startCore(onFrame: (part: FramePart) => void): RemoteCore {
  const worker = new Worker(new URL("./core.worker.ts", import.meta.url), { type: "module" });
  const waiting = new Map<number, { resolve: (value: unknown) => void; reject: (reason: Error) => void }>();
  let next = 1;
  worker.onmessage = (event: MessageEvent<Reply>) => {
    const { id, result, error, frame } = event.data;
    if (frame) onFrame(frame);
    const call = waiting.get(id);
    waiting.delete(id);
    if (error !== undefined) call?.reject(new Error(error));
    else call?.resolve(result);
  };
  worker.onerror = (event) => {
    // The worker itself failed (it could not load, or the core crashed): nothing waiting will be answered.
    const failure = new Error(event.message || "The core stopped working.");
    for (const call of waiting.values()) call.reject(failure);
    waiting.clear();
  };
  const call = (method: string, args: unknown[]) =>
    new Promise<unknown>((resolve, reject) => {
      const id = next++;
      waiting.set(id, { resolve, reject });
      // Bytes handed to the core (a font, an image, a file) are given over, not copied. The caller does not use them again.
      const transfer: Transferable[] = [];
      const give = (value: unknown) => {
        if (value instanceof Uint8Array && value.byteOffset === 0 && value.byteLength === value.buffer.byteLength && value.buffer instanceof ArrayBuffer) transfer.push(value.buffer);
        else if (Array.isArray(value)) value.forEach((item) => item && typeof item === "object" && "data" in item && give((item as { data: unknown }).data));
      };
      args.forEach(give);
      worker.postMessage({ id, method, args } satisfies Call, transfer);
    });
  return new Proxy({} as RemoteCore, {
    // Not a promise itself: `then` must be absent, or awaiting the core would call it.
    get: (_, method: string | symbol) => (typeof method !== "string" || method === "then" ? undefined : method === "pending" ? waiting.size : (...args: unknown[]) => call(method, args)),
  });
}
