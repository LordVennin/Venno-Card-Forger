/**
 * Times the things that have to feel instant: dragging a picture layer and erasing on it,
 * on a built card, in the WebAssembly core at a fit-to-window zoom.
 *   tsx scripts/lag-timing.ts <template-dir> <fonts-dir> <card.json> <art> [zoom]
 */
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { Core } from "../src/lib/core";

const [templateDir, fontsDir, cardFile, art, zoom] = process.argv.slice(2);
const core = await Core.load(readFileSync(new URL("../src/wasm/cardsmith.wasm", import.meta.url)));
const images = () => {
  const missing = core.missingAssets();
  missing.forEach((m, i) => core.addAsset(m.id, readFileSync(join(templateDir!, m.path)), i === missing.length - 1));
};
/** Run `step` n times and report the first call, then the median and the worst of the rest. */
const steps = (what: string, n: number, step: (i: number) => void) => {
  const times: number[] = [];
  for (let i = 0; i < n; i++) {
    const started = performance.now();
    step(i);
    times.push(performance.now() - started);
  }
  const rest = times.slice(1).sort((a, b) => a - b);
  console.log(`${what}: first ${times[0]!.toFixed(0)} ms, then median ${rest[Math.floor(rest.length / 2)]!.toFixed(0)} ms, worst ${rest[rest.length - 1]!.toFixed(0)} ms`);
};
for (const name of readdirSync(fontsDir!)) {
  try {
    core.addFont(readFileSync(join(fontsDir!, name)));
  } catch {}
}
core.openTemplate(readFileSync(join(templateDir!, "doc.json"), "utf8"), readFileSync(join(templateDir!, "manifest.json"), "utf8"));
core.setView({ scale: Number(zoom ?? 0.2), x: 0, y: 0, width: 1200, height: 900 });
images();
core.setArt(readFileSync(art!));
core.buildCard({ scryfall: JSON.parse(readFileSync(cardFile!, "utf8")) });
images();
const find = (layers: any[], test: (l: any) => boolean): any => layers.flatMap((l) => (test(l) ? [l] : l.children ? [find(l.children, test)].filter(Boolean) : []))[0];
const doc = core.doc();
const artLayer = find(doc.root.children, (l) => l.role === "card.art");
const eraser = { size: 120, hardness: 0.8, opacity: 1, erase: true, color: [0, 0, 0] as [number, number, number] };

// A picture added on top of everything, as "+ Image" does.
const bytes = readFileSync(art!);
core.addAsset("top-picture", bytes, false);
const size = core.setArt(bytes); // only to learn its size
core.execute({ type: "batch", edits: [
  { type: "setAsset", id: "top-picture", asset: { kind: "image", path: "assets/top-picture.png", width: size.width, height: size.height } },
  { type: "addLayer", parent: doc.root.id, layer: { id: "top-picture", name: "Picture", kind: "pixel", visible: true, opacity: 1, blend: "normal", clip: false, asset: "top-picture", transform: [1.5, 0, 300, 0, 1.5, 600] } },
] }, "New image layer");

steps("drag the picture on top", 25, (i) => core.drag("top-picture", 1, (i + 1) * 9, (i + 1) * 6));
core.dragEnd(true);
steps("drag the art (behind the frame)", 25, (i) => core.drag(artLayer.id, 1, (i + 1) * 9, (i + 1) * 6));
core.dragEnd(true);
core.strokeBegin("top-picture", eraser);
steps("erase on the picture on top, per pointer move", 40, (i) => core.strokeTo(700 + i * 40, 1200 + i * 25, 1));
core.strokeEnd("Eraser");
core.strokeBegin("top-picture", eraser);
steps("a second eraser stroke on it", 40, (i) => core.strokeTo(900 + i * 40, 1000 + i * 25, 1));
core.strokeEnd("Eraser");
core.strokeBegin(artLayer.id, eraser);
steps("erase on the art, per pointer move", 40, (i) => core.strokeTo(700 + i * 40, 900 + i * 25, 1));
core.strokeEnd("Eraser");
// What a stroke costs before its first move and after its last, and with a big brush.
const once = (what: string, f: () => void) => {
  const started = performance.now();
  f();
  console.log(`${what}: ${(performance.now() - started).toFixed(0)} ms`);
};
once("begin a stroke on the picture on top", () => core.strokeBegin("top-picture", eraser));
core.strokeTo(1500, 1500, 1);
once("end it", () => core.strokeEnd("Eraser"));
core.strokeBegin("top-picture", { ...eraser, size: 500 });
steps("erase with a 500 px eraser, per pointer move", 30, (i) => core.strokeTo(600 + i * 50, 1400 + i * 30, 1));
once("end it", () => core.strokeEnd("Eraser"));
once("undo it", () => core.undo());
