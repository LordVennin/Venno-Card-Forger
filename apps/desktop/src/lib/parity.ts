/**
 * Dev-only half of `pnpm test:parity`. If the dev server offers the parity
 * endpoint, render every sample here and report the frame hashes. In normal
 * use the endpoint does not exist and this does nothing.
 */
import type { Core } from "./core";
import { getCore } from "./renderer";

export async function runParityIfRequested(): Promise<void> {
  if (!import.meta.env.DEV) return;
  try {
    const response = await fetch("/__parity/input");
    if (!response.ok || !response.headers.get("content-type")?.includes("json")) return;
  } catch {
    return;
  }
  const core = await getCore();
  const results = core.sampleNames().map((name, index) => {
    const started = performance.now();
    core.openSample(index);
    const doc = core.doc();
    return { name, size: `${doc.width}x${doc.height}`, hash: core.frameHash(), ms: Math.round(performance.now() - started) };
  });
  await fetch("/__parity/result", { method: "POST", body: JSON.stringify({ userAgent: navigator.userAgent, results, bench: benchStrokes(core) }) });
}

/**
 * Scripted brush strokes on the card-sized sample (left open by the hash pass),
 * timed the way the app paints: core work per pointer move, then the dirty
 * rectangle onto a real canvas.
 */
function benchStrokes(core: Core): string[] {
  const canvas = document.createElement("canvas");
  canvas.width = 1300;
  canvas.height = 900;
  document.body.append(canvas);
  const ctx = canvas.getContext("2d")!;
  const doc = core.doc();
  // Fit to the window, as the app does on open.
  const scale = Math.min(canvas.width / doc.width, canvas.height / doc.height) * 0.94;
  const lines: string[] = [];
  const stats = (v: number[]) => {
    const s = [...v].sort((a, b) => a - b);
    return `median ${s[s.length >> 1]!.toFixed(1)}, 95th ${s[Math.floor(s.length * 0.95)]!.toFixed(1)}, worst ${s[s.length - 1]!.toFixed(1)} ms`;
  };
  for (const [label, zoom] of [["fit to window", scale], ["100% zoom", 1]] as const) {
    const started = performance.now();
    core.setView({ scale: zoom, x: zoom === 1 ? 400 : 0, y: zoom === 1 ? 600 : 0, width: canvas.width, height: canvas.height });
    const set = performance.now() - started;
    ctx.putImageData(new ImageData(core.viewPixels(), canvas.width, canvas.height), 0, 0);
    lines.push(`view ${label}: set ${set.toFixed(1)} ms, full blit ${(performance.now() - started - set).toFixed(1)} ms`);
    const layer = core.addPaintLayer("Bench");
    for (const size of [32, 128, 256, 600]) {
      const begin = performance.now();
      core.strokeBegin(layer, { size, hardness: 0.8, opacity: 1, color: [200, 30, 30], erase: false });
      const beginMs = performance.now() - begin;
      const inCore: number[] = [], toScreen: number[] = [];
      for (let i = 0; i < 60; i++) {
        const t0 = performance.now();
        core.strokeTo(500 + i * 36, 700 + i * 40);
        const d = core.dirty();
        const t1 = performance.now();
        if (d) ctx.putImageData(new ImageData(core.viewPixels(), canvas.width, canvas.height), 0, 0, d.x, d.y, d.width, d.height);
        inCore.push(t1 - t0);
        toScreen.push(performance.now() - t1);
      }
      core.strokeEnd("Bench");
      lines.push(`  ${label}, brush ${size} px: begin ${beginMs.toFixed(1)} ms; core ${stats(inCore)}; to screen ${stats(toScreen)}`);
    }
    // A second pass with the same brush, now that the layers beneath are cached.
    core.strokeBegin(layer, { size: 256, hardness: 0.8, opacity: 1, color: [30, 30, 200], erase: false });
    const again: number[] = [];
    for (let i = 0; i < 60; i++) {
      const t0 = performance.now();
      core.strokeTo(520 + i * 36, 720 + i * 40);
      again.push(performance.now() - t0);
    }
    core.strokeEnd("Bench");
    lines.push(`  ${label}, brush 256 px over the same path again: core ${stats(again)}`);
    const undo = performance.now();
    core.undo();
    lines.push(`  ${label}: undo that stroke ${(performance.now() - undo).toFixed(0)} ms`);
  }
  canvas.remove();
  return lines;
}
