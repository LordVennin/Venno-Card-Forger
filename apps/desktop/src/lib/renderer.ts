import wasmUrl from "canvaskit-wasm/bin/canvaskit.wasm?url";
import { Renderer, loadCanvasKit } from "@cardsmith/render";
import type { CanvasKit } from "canvaskit-wasm";

let ready: Promise<{ ck: CanvasKit; renderer: Renderer }> | undefined;

/** The app's single renderer. Same code and same WASM as the headless one. */
export function getRenderer() {
  ready ??= loadCanvasKit(() => wasmUrl).then((ck) => ({ ck, renderer: new Renderer(ck) }));
  return ready;
}
