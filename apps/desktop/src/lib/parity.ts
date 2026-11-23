/**
 * Dev-only half of `pnpm test:parity`. If the dev server offers parity input,
 * render it here and send the pixels back. In normal use the endpoint does not
 * exist and this does nothing.
 */
import { validateDoc, type AssetBytes } from "@cardsmith/doc";
import { getRenderer } from "./renderer";

type Input = { name: string; doc: unknown; assets: Record<string, string> }[];

const fromBase64 = (text: string) => Uint8Array.from(atob(text), (c) => c.charCodeAt(0));

function toBase64(bytes: Uint8Array): string {
  let text = "";
  for (let i = 0; i < bytes.length; i += 0x8000) text += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(text);
}

export async function runParityIfRequested(): Promise<void> {
  if (!import.meta.env.DEV) return;
  let input: Input;
  try {
    const response = await fetch("/__parity/input");
    if (!response.ok || !response.headers.get("content-type")?.includes("json")) return;
    input = await response.json();
  } catch {
    return;
  }
  const { renderer } = await getRenderer();
  const results = input.map(({ name, doc, assets }) => {
    const bytes: AssetBytes = {};
    for (const [id, data] of Object.entries(assets)) bytes[id] = fromBase64(data);
    const r = renderer.render(validateDoc(doc), bytes);
    return { name, width: r.width, height: r.height, pixels: toBase64(r.pixels), warnings: r.warnings };
  });
  await fetch("/__parity/result", { method: "POST", body: JSON.stringify({ userAgent: navigator.userAgent, results }) });
}
