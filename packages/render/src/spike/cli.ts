import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { loadCanvasKit } from "../canvaskit.js";
import { renderSpike } from "./fitParagraph.js";
import { SAMPLE_OPTIONS, SAMPLE_RUNS, loadSampleFont } from "./sample.js";

const outFlag = process.argv.indexOf("-o");
const out = resolve(outFlag > 0 ? process.argv[outFlag + 1]! : "out/spike-text-fit.png");

const ck = await loadCanvasKit();
const { png, result } = renderSpike(ck, await loadSampleFont(), SAMPLE_RUNS, SAMPLE_OPTIONS);
await mkdir(dirname(out), { recursive: true });
await writeFile(out, png);
console.log(JSON.stringify({ out, ...result }, null, 2));
