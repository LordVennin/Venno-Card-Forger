/**
 * Dev-only half of `pnpm test:parity`. If the dev server offers the parity
 * endpoint, render every sample here and report the frame hashes. In normal
 * use the endpoint does not exist and this does nothing.
 */
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
    core.openSample(index);
    const started = performance.now();
    const frame = core.render();
    return { name, size: `${frame.width}x${frame.height}`, hash: core.frameHash(), ms: Math.round(performance.now() - started) };
  });
  await fetch("/__parity/result", { method: "POST", body: JSON.stringify({ userAgent: navigator.userAgent, results }) });
}
