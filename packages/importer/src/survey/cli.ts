/**
 * pnpm survey <dir-of-psds> [-o <out-dir>] [--layers <path/to/layers.py>]
 *
 * Writes survey.json and survey.md to the output directory
 * (default: fixtures/local/survey, which is gitignored).
 */
import { existsSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { parseArgs } from "node:util";
import { renderMarkdown } from "./markdown.js";
import { surveyDirectory } from "./run.js";

const DEFAULT_LAYERS_PY = "reference/Proxyshop/src/enums/layers.py";

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    out: { type: "string", short: "o", default: "fixtures/local/survey" },
    layers: { type: "string" },
  },
});

const dir = positionals[0];
if (!dir) {
  console.error("usage: pnpm survey <dir-of-psds> [-o <out-dir>] [--layers <layers.py>]");
  process.exit(2);
}

let layersPy: string | undefined = values.layers ?? DEFAULT_LAYERS_PY;
if (!existsSync(layersPy)) {
  if (values.layers) {
    console.error(`layers file not found: ${layersPy}`);
    process.exit(2);
  }
  console.warn(`note: ${DEFAULT_LAYERS_PY} not found; skipping layer-name matching.`);
  layersPy = undefined;
}

const survey = await surveyDirectory(resolve(dir), {
  layersPy,
  onFile: (file) => console.error(`reading ${file}`),
});
if (survey.files.length + survey.failed.length === 0) {
  console.error(`no .psd or .psb files found under ${dir}`);
  process.exit(1);
}

const outDir = resolve(values.out);
await mkdir(outDir, { recursive: true });
await writeFile(`${outDir}/survey.json`, JSON.stringify(survey, null, 2) + "\n");
await writeFile(`${outDir}/survey.md`, renderMarkdown(survey));
console.error(
  `surveyed ${survey.files.length} file(s), ${survey.totalLayers} layers` +
    (survey.failed.length ? `, ${survey.failed.length} failed` : "") +
    ` -> ${outDir}/survey.{json,md}`,
);
process.exit(survey.failed.length ? 1 : 0);
