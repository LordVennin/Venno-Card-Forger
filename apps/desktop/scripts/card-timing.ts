/**
 * Times the card workbench in the WebAssembly core, outside the app:
 *   pnpm --filter @cardsmith/desktop exec tsx scripts/card-timing.ts <template-dir> <fonts-dir> <card.json> [art]
 * `card.json` is Scryfall's JSON for a card (cardsmith card "<name>" --raw).
 */
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { Core } from "../src/lib/core";

const [templateDir, fontsDir, cardFile, art] = process.argv.slice(2);
const core = await Core.load(readFileSync(new URL("../src/wasm/cardsmith.wasm", import.meta.url)));
const time = <T>(what: string, f: () => T): T => {
  const started = performance.now();
  const out = f();
  console.log(`${(performance.now() - started).toFixed(0).padStart(6)} ms  ${what}`);
  return out;
};
const images = () => {
  const missing = core.missingAssets();
  missing.forEach((m, i) => core.addAsset(m.id, readFileSync(join(templateDir!, m.path)), i === missing.length - 1));
  return missing.length;
};

for (const name of readdirSync(fontsDir!)) {
  try {
    core.addFont(readFileSync(join(fontsDir!, name)));
  } catch {}
}
time("open the template", () => core.openTemplate(readFileSync(join(templateDir!, "doc.json"), "utf8"), readFileSync(join(templateDir!, "manifest.json"), "utf8")));
time("set a screen-sized view", () => core.setView({ scale: 0.2, x: 0, y: 0, width: 1200, height: 900 }));
time(`load its showing images (${core.missingAssets().length})`, images);
const scryfall = JSON.parse(readFileSync(cardFile!, "utf8"));
if (art) time("set the art", () => core.setArt(readFileSync(art)));
time("build the card", () => core.buildCard({ scryfall }));
time(`load the images the card shows (${core.missingAssets().length})`, images);
const doc = time("read the document back (what the panel does after every edit)", () => core.doc());
console.log(`         the document is ${(JSON.stringify(doc).length / 1e6).toFixed(1)} MB of JSON`);
scryfall.oracle_text += " x";
time("rebuild after typing one letter of rules text", () => core.buildCard({ scryfall }));
scryfall.name += "x";
time("rebuild after typing one letter of the name", () => core.buildCard({ scryfall }));
const find = (layers: any[], test: (l: any) => boolean): any => layers.flatMap((l) => (test(l) ? [l] : l.children ? [find(l.children, test)].filter(Boolean) : []))[0];
const text = find(doc.root.children, (l) => l.kind === "text" && l.visible);
time(`hide one text layer (${text.name})`, () => core.execute({ type: "setLayerProps", id: text.id, props: { visible: false } }, "Hide"));
time("undo", () => core.undo());
const art2 = find(core.doc().root.children, (l) => l.role === "card.art");
if (art2) {
  time("hide the art (the whole card is drawn again)", () => core.execute({ type: "setLayerProps", id: art2.id, props: { visible: false } }, "Hide"));
  time("undo", () => core.undo());
}
if (art2) {
  // Dragging the art: ten small moves, as pointer events would send them.
  const t = [...art2.transform];
  const started = performance.now();
  for (let i = 1; i <= 10; i++) core.execute({ type: "setLayerProps", id: art2.id, props: { transform: [t[0], t[1], t[2] + i * 12, t[3], t[4], t[5] + i * 8] } }, "Move");
  console.log(`${((performance.now() - started) / 10).toFixed(0).padStart(6)} ms  each step of dragging the art behind the frame`);
  time("the full-size picture catching up afterwards (as before saving or exporting)", () => core.frameHash());
}
if (art2) {
  const started = performance.now();
  for (let i = 1; i <= 20; i++) core.drag(art2.id, 1, i * 6, i * 4);
  console.log(`${((performance.now() - started) / 20).toFixed(0).padStart(6)} ms  each step of a live drag of the art (the move tool)`);
  core.dragEnd(true, "Move the art");
  const name = find(core.doc().root.children, (l) => l.role === "card.name");
  if (name) {
    const t0 = performance.now();
    for (let i = 1; i <= 20; i++) core.drag(name.id, 1, i * 6, i * 4);
    console.log(`${((performance.now() - t0) / 20).toFixed(0).padStart(6)} ms  each step of a live drag of the name`);
    core.dragEnd(false);
  }
}
