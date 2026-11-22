/**
 * pnpm composite <psd-or-dir> [-o <out-dir>]
 *
 * Extracts each PSD's embedded composite image (the flattened picture Photoshop
 * saved, showing the default visibility state) as a PNG. These are the
 * minimum-viable golden references for the importer (SPEC Phase 0, task 5).
 * Output defaults to fixtures/local/golden, which is gitignored.
 */
import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import { basename, resolve } from "node:path";
import { parseArgs } from "node:util";
import { readPsd } from "ag-psd";
import { loadCanvasKit } from "@cardsmith/render";
import { initHeadlessPsd } from "../headless.js";
import { findPsds } from "../survey/run.js";

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: { out: { type: "string", short: "o", default: "fixtures/local/golden" } },
});
const input = positionals[0];
if (!input) {
  console.error("usage: pnpm composite <psd-or-dir> [-o <out-dir>]");
  process.exit(2);
}

const target = resolve(input);
const files = (await stat(target)).isDirectory() ? await findPsds(target) : [target];
const outDir = resolve(values.out);
await mkdir(outDir, { recursive: true });
const ck = await loadCanvasKit();
initHeadlessPsd();

let failed = 0;
for (const file of files) {
  const name = basename(file).replace(/\.ps[db]$/i, "");
  try {
    const psd = readPsd(await readFile(file), {
      skipLayerImageData: true,
      skipThumbnail: true,
      skipLinkedFilesData: true,
      useImageData: true,
    });
    const data = psd.imageData;
    if (!data || !(data.data instanceof Uint8ClampedArray || data.data instanceof Uint8Array)) {
      throw new Error("no 8-bit composite image (was the PSD saved without Maximize Compatibility?)");
    }
    const image = ck.MakeImage(
      {
        width: data.width,
        height: data.height,
        colorType: ck.ColorType.RGBA_8888,
        alphaType: ck.AlphaType.Unpremul,
        colorSpace: ck.ColorSpace.SRGB,
      },
      data.data,
      data.width * 4,
    );
    const png = image?.encodeToBytes();
    image?.delete();
    if (!png) throw new Error("PNG encode failed");
    const out = `${outDir}/${name}.default.png`;
    await writeFile(out, png);
    console.error(`${basename(file)} -> ${out} (${data.width}x${data.height})`);
  } catch (err) {
    failed++;
    console.error(`${basename(file)}: ${err instanceof Error ? err.message : err}`);
  }
}
process.exit(failed ? 1 : 0);
