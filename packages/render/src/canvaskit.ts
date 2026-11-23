import type { CanvasKit } from "canvaskit-wasm";

let instance: Promise<CanvasKit> | undefined;

/**
 * Load CanvasKit once per process.
 *
 * `locateFile` tells CanvasKit where its .wasm lives. The app passes a bundler
 * URL. In Node it can be omitted, and the file is resolved from the installed
 * package.
 */
export function loadCanvasKit(locateFile?: (file: string) => string): Promise<CanvasKit> {
  instance ??= (async () => {
    const { default: init } = await import("canvaskit-wasm");
    let locate = locateFile;
    if (!locate) {
      // Node only. Kept out of the static import graph so bundlers ignore it.
      const nodeModule = "node:module";
      const { createRequire } = (await import(/* @vite-ignore */ nodeModule)) as typeof import("node:module");
      const require = createRequire(import.meta.url);
      locate = (file) => require.resolve(`canvaskit-wasm/bin/${file}`);
    }
    return init({ locateFile: locate });
  })();
  return instance;
}
