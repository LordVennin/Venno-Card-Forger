/**
 * Builds a card in the WebAssembly core, cuts the sky out of its art with the magic eraser,
 * rubs a line across the name bar with the eraser, and writes the result as a PNG:
 *   tsx scripts/erase-check.ts <template-dir> <fonts-dir> <card.json> <art> <out.png>
 */
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { Core } from "../src/lib/core";

const [templateDir, fontsDir, cardFile, art, out] = process.argv.slice(2);
const core = await Core.load(readFileSync(new URL("../src/wasm/cardsmith.wasm", import.meta.url)));
const time = <T>(what: string, f: () => T): T => {
  const started = performance.now();
  const result = f();
  console.log(`${Math.round(performance.now() - started).toString().padStart(6)} ms  ${what}`);
  return result;
};
const images = () => {
  const missing = core.missingAssets();
  missing.forEach((m, i) => core.addAsset(m.id, readFileSync(join(templateDir!, m.path)), i === missing.length - 1));
};
for (const name of readdirSync(fontsDir!)) {
  try {
    core.addFont(readFileSync(join(fontsDir!, name)));
  } catch {}
}
core.openTemplate(readFileSync(join(templateDir!, "doc.json"), "utf8"), readFileSync(join(templateDir!, "manifest.json"), "utf8"));
core.setView({ scale: 0.2, x: 0, y: 0, width: 1200, height: 900 });
images();
core.setArt(readFileSync(art!));
core.buildCard({ scryfall: JSON.parse(readFileSync(cardFile!, "utf8")) });
images();
const find = (layers: any[], test: (l: any) => boolean): any => layers.flatMap((l) => (test(l) ? [l] : l.children ? [find(l.children, test)].filter(Boolean) : []))[0];
const doc = core.doc();
const artLayer = find(doc.root.children, (l) => l.role === "card.art");
const gone = time("magic eraser on the art's sky (first use on a layer makes its mask)", () => core.magicErase(artLayer.id, doc.width * 0.7, doc.height * 0.12, 40, true));
console.log(`         ${gone} pixels erased`);
time("magic eraser again elsewhere", () => core.magicErase(artLayer.id, doc.width * 0.5, doc.height * 0.52, 24, true));
const name = find(doc.root.children, (l) => l.role === "card.name");
const box = core.layerBox(name.id)!;
time("an eraser stroke across the card name (a text layer)", () => {
  core.strokeBegin(name.id, { size: 40, hardness: 0.9, opacity: 1, erase: true, color: [0, 0, 0] });
  for (let i = 0; i <= 20; i++) core.strokeTo(box.x + (box.width * i) / 20, box.y + box.height / 2, 1);
  core.strokeEnd("Eraser");
});
time("undo the stroke", () => core.undo());
console.log("history:", core.state().labels.join(" | "));
time("export", () => writeFileSync(out!, core.exportPng()));
