import { createRequire } from "node:module";
import type { CanvasKit } from "canvaskit-wasm";

let instance: Promise<CanvasKit> | undefined;

/**
 * Load CanvasKit once per process.
 *
 * `locateFile` tells CanvasKit where its .wasm lives. In Node we resolve it from
 * the installed package; in the app the caller passes a URL resolver (Vite asset).
 */
export function loadCanvasKit(locateFile?: (file: string) => string): Promise<CanvasKit> {
  instance ??= (async () => {
    const { default: init } = await import("canvaskit-wasm");
    const locate =
      locateFile ??
      ((file: string) => createRequire(import.meta.url).resolve(`canvaskit-wasm/bin/${file}`));
    return init({ locateFile: locate });
  })();
  return instance;
}
