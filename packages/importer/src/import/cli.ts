/**
 * pnpm import:psd <psd> -o <template-dir>
 *
 * Converts a Proxyshop-style PSD into a template package (SPEC §3): doc.json,
 * assets/ and manifest.json. The PSD is only ever read here; rendering uses
 * the package.
 */
import { createHash } from "node:crypto";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { basename, dirname, join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { decodeLayerPixels, readPsd } from "ag-psd";
import { initHeadlessPsd } from "../headless.js";
import { describeBindings, generateBindings } from "./bindings.js";
import { convertPsd } from "./convert.js";

const { values, positionals } = parseArgs({ allowPositionals: true, options: { out: { type: "string", short: "o" } } });
const input = positionals[0];
if (!input || !values.out) {
  console.error("usage: pnpm import:psd <psd> -o <template-dir>");
  process.exit(2);
}

const started = performance.now();
const file = resolve(input);
const outDir = resolve(values.out);
const name = basename(file).replace(/\.ps[db]$/i, "");
initHeadlessPsd();

const bytes = await readFile(file);
// Layer pixels stay compressed until each layer is converted, so memory use follows the largest layer, not the file.
const psd = readPsd(bytes, { useRawData: true, useImageData: true, skipCompositeImageData: true, skipThumbnail: true, skipLinkedFilesData: true });

await rm(outDir, { recursive: true, force: true });
await mkdir(join(outDir, "assets"), { recursive: true });
const writes: Promise<void>[] = [];
let images = 0;
let imageBytes = 0;

const { doc, warnings, textInk } = convertPsd(psd, {
  emit(path, png) {
    images++;
    imageBytes += png.length;
    const target = join(outDir, path);
    writes.push(mkdir(dirname(target), { recursive: true }).then(() => writeFile(target, png)));
  },
  decode(layer) {
    if (layer.rawData || layer.mask) decodeLayerPixels(layer, true);
  },
  release(layer) {
    layer.imageData = undefined;
    layer.rawData = undefined;
    if (layer.mask) layer.mask.imageData = undefined;
  },
});
await Promise.all(writes);

let layers = 0;
const count = (list: { kind: string; children?: unknown[] }[]) => {
  for (const layer of list) {
    layers++;
    if (layer.kind === "group") count(layer.children as { kind: string }[]);
  }
};
count(doc.root.children);

const bindings = generateBindings(doc);
await writeFile(join(outDir, "doc.json"), JSON.stringify(doc, null, 1) + "\n");
await writeFile(
  join(outDir, "manifest.json"),
  JSON.stringify(
    {
      id: name,
      name,
      source: { file: basename(file), sha256: createHash("sha256").update(bytes).digest("hex"), bytes: bytes.length },
      width: doc.width,
      height: doc.height,
      dpi: doc.dpi,
      layers,
      warnings,
      bindings,
      textInk,
    },
    null,
    2,
  ) + "\n",
);

for (const warning of warnings) console.error(`warning: ${warning}`);
console.error(describeBindings(bindings));
console.error(
  `${basename(file)} -> ${outDir}: ${layers} layers, ${images} images (${(imageBytes / 1e6).toFixed(1)} MB), ` +
    `${warnings.length} warning${warnings.length === 1 ? "" : "s"}, ${((performance.now() - started) / 1000).toFixed(1)} s`,
);
