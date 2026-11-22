import { initializeCanvas } from "ag-psd";

let done = false;

/**
 * Let ag-psd decode pixels in Node with no DOM and no node-canvas.
 *
 * With `useImageData: true` ag-psd only needs an ImageData-shaped buffer, so we
 * hand it plain objects. Anything that still asks for a real canvas is a code
 * path we must not depend on, and fails loudly.
 */
export function initHeadlessPsd(): void {
  if (done) return;
  done = true;
  initializeCanvas(
    () => {
      throw new Error("ag-psd asked for a canvas; read with { useImageData: true } instead");
    },
    (width, height) =>
      ({ width, height, data: new Uint8ClampedArray(width * height * 4), colorSpace: "srgb" }) as ImageData,
  );
}
