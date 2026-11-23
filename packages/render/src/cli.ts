/**
 * pnpm render <doc.json | file.cardsmith> -o <out.png>
 * pnpm render --samples <dir>      write the built-in sample documents and their renders
 *
 * For a bare doc.json, assets are read from the paths its asset table names,
 * relative to the JSON file.
 */
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { loadCardsmith, parseDoc, saveCardsmith, type AssetBytes, type Doc } from "@cardsmith/doc";
import { loadCanvasKit } from "./canvaskit.js";
import { Renderer, encodePng } from "./render.js";
import { buildSamples } from "./samples.js";

export async function loadDocument(file: string): Promise<{ doc: Doc; assets: AssetBytes }> {
  const bytes = await readFile(file);
  // Zip files start with "PK"; anything else is treated as JSON.
  if (bytes[0] === 0x50 && bytes[1] === 0x4b) return loadCardsmith(bytes);
  const doc = parseDoc(bytes.toString("utf8"));
  const assets: AssetBytes = {};
  for (const [id, ref] of Object.entries(doc.assets)) {
    try {
      assets[id] = await readFile(join(dirname(file), ref.path));
    } catch {
      // Left out on purpose: the renderer reports missing assets as warnings.
    }
  }
  return { doc, assets };
}

/** Returns the process exit code. `log` receives progress and warnings. */
export async function main(argv: string[], log: (line: string) => void = console.error): Promise<number> {
  const { values, positionals } = parseArgs({
    args: argv,
    allowPositionals: true,
    options: { out: { type: "string", short: "o" }, samples: { type: "string" } },
  });
  const ck = await loadCanvasKit();
  const renderer = new Renderer(ck);
  try {
    if (values.samples) {
      const dir = resolve(values.samples);
      await mkdir(dir, { recursive: true });
      for (const sample of buildSamples(ck)) {
        const r = renderer.render(sample.doc, sample.assets);
        await writeFile(join(dir, `${sample.name}.cardsmith`), saveCardsmith(sample.doc, sample.assets));
        await writeFile(join(dir, `${sample.name}.png`), encodePng(ck, r.width, r.height, r.pixels));
        log(`${sample.name}: ${r.width}×${r.height} -> ${join(dir, sample.name)}.{cardsmith,png}`);
      }
      return 0;
    }
    const input = positionals[0];
    if (!input || !values.out) {
      log("usage: pnpm render <doc.json | file.cardsmith> -o <out.png>\n       pnpm render --samples <dir>");
      return 2;
    }
    const { doc, assets } = await loadDocument(resolve(input));
    const started = performance.now();
    const r = renderer.render(doc, assets);
    const out = resolve(values.out);
    await mkdir(dirname(out), { recursive: true });
    await writeFile(out, encodePng(ck, r.width, r.height, r.pixels));
    for (const warning of r.warnings) log(`warning: ${warning}`);
    log(`${r.width}×${r.height} in ${Math.round(performance.now() - started)} ms -> ${out}`);
    return 0;
  } catch (err) {
    log(`error: ${err instanceof Error ? err.message : String(err)}`);
    return 1;
  } finally {
    renderer.dispose();
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exit(await main(process.argv.slice(2)));
}
