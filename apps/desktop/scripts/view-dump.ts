/**
 * Builds a card in the WebAssembly core as the app does and writes what the screen would show, as raw RGBA:
 *   tsx scripts/view-dump.ts <template-dir> <fonts-dir> <card.json> <art> <out-prefix> [zoom]
 * One file after the build, one after a rules edit, one after dragging the name. Each is 1200 x 1600.
 */
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { Core } from "../src/lib/core";

const [templateDir, fontsDir, cardFile, art, out, zoom] = process.argv.slice(2);
const core = await Core.load(readFileSync(new URL("../src/wasm/cardsmith.wasm", import.meta.url)));
const images = () => {
  const missing = core.missingAssets();
  missing.forEach((m, i) => core.addAsset(m.id, readFileSync(join(templateDir!, m.path)), i === missing.length - 1));
};
const shot = (name: string) => writeFileSync(`${out}-${name}.rgba`, core.viewPixels());
// As in the app: the template is opened first, fonts arrive after.
core.openTemplate(readFileSync(join(templateDir!, "doc.json"), "utf8"), readFileSync(join(templateDir!, "manifest.json"), "utf8"));
core.setView({ scale: Number(zoom ?? 0.3), x: 0, y: 0, width: 1200, height: 1600 });
images();
for (const name of readdirSync(fontsDir!)) {
  try {
    core.addFont(readFileSync(join(fontsDir!, name)));
  } catch {}
}
const scryfall = JSON.parse(readFileSync(cardFile!, "utf8"));
core.setArt(readFileSync(art!));
core.buildCard({ scryfall });
images();
shot("built");
scryfall.oracle_text += " Draw a card.";
core.buildCard({ scryfall });
shot("edited");
const find = (layers: any[], test: (l: any) => boolean): any => layers.flatMap((l) => (test(l) ? [l] : l.children ? [find(l.children, test)].filter(Boolean) : []))[0];
const name = find(core.doc().root.children, (l) => l.role === "card.name");
core.drag(name.id, 1, 40, 30);
core.dragEnd(true);
shot("dragged");
// What Export PNG writes: the document at full size, whatever the screen was showing.
core.undo();
core.undo();
const started = performance.now();
writeFileSync(`${out}-export.png`, core.exportPng());
console.log(`exported in ${Math.round(performance.now() - started)} ms`);
