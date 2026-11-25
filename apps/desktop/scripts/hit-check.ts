/**
 * Builds a card in the WebAssembly core and says which layer a click picks at a grid of points:
 *   tsx scripts/hit-check.ts <template-dir> <fonts-dir> <card.json> <art>
 */
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { Core } from "../src/lib/core";

const [templateDir, fontsDir, cardFile, art] = process.argv.slice(2);
const core = await Core.load(readFileSync(new URL("../src/wasm/cardsmith.wasm", import.meta.url)));
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
const doc = core.doc();
const names = new Map<string, string>();
const walk = (layers: any[], path: string) => layers.forEach((l) => (names.set(l.id, `${path}${l.name}`), l.children && walk(l.children, `${path}${l.name} / `)));
walk(doc.root.children, "");
for (const fy of [0.05, 0.12, 0.2, 0.35, 0.5, 0.62, 0.75, 0.9, 0.97]) {
  const row = [0.1, 0.5, 0.9].map((fx) => {
    const id = core.hit(doc.width * fx, doc.height * fy);
    return (id ? names.get(id)! : "nothing").padEnd(44).slice(0, 44);
  });
  console.log(`${Math.round(fy * 100).toString().padStart(3)}%  ${row.join(" | ")}`);
}
