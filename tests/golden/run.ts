/**
 * pnpm test:golden [template...] [--update <template>.<state>]
 *
 * Renders each imported template with the Rust core, in its default state and
 * in each state named in states.json, and compares the result with a reference
 * image where one exists. Everything it needs lives in the gitignored
 * fixtures/local/ folder; with nothing there it skips cleanly.
 *
 *   fixtures/local/imported/<template>/doc.json        from `pnpm import:psd`
 *   fixtures/local/golden/<template>.default.png       from `pnpm composite`
 *   fixtures/local/golden/<template>.<state>.png       exported from Photoshop or Photopea
 *
 * Output goes to out/golden/: the render and a difference heatmap per state.
 *
 * `--update <template>.<state>` replaces a reference with the current render.
 * Do that only after a person has looked at the new output and confirmed it
 * is right.
 */
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";

const repo = resolve(import.meta.dirname, "../..");
const imported = join(repo, "fixtures/local/imported");
const golden = join(repo, "fixtures/local/golden");
const out = join(repo, "out/golden");

type Limits = { minPsnr: number; maxBlockError: number };
const thresholds = JSON.parse(readFileSync(join(import.meta.dirname, "thresholds.json"), "utf8")) as { default: Limits; templates: Record<string, Partial<Limits>> };
const { values, positionals } = parseArgs({ allowPositionals: true, options: { update: { type: "string" } } });

type DocLayer = { kind: string; name: string; visible: boolean; effectsEnabled?: boolean; box?: { x: number; y: number; w: number; h: number }; children?: DocLayer[]; effects?: { enabled: boolean; size?: number; distance?: number }[] };

/**
 * Areas to leave out of the comparison: where visible text layers are. Text is
 * not drawn until Phase 3, so those pixels cannot match yet. Remove this when
 * it is.
 */
function textRegions(layers: DocLayer[], visible = true, regions: string[] = []): string[] {
  for (const layer of layers) {
    const shown = visible && layer.visible;
    if (layer.kind === "group") textRegions(layer.children ?? [], shown, regions);
    if (layer.kind !== "text" || !shown || !layer.box) continue;
    // Room for anti-aliasing, glyphs that overhang the box, and the layer's shadow or stroke.
    const reach = layer.effectsEnabled === false ? 0 : Math.max(0, ...(layer.effects ?? []).filter((e) => e.enabled).map((e) => (e.size ?? 0) * 2 + (e.distance ?? 0)));
    const pad = Math.ceil(24 + reach);
    regions.push([Math.floor(layer.box.x - pad), Math.floor(layer.box.y - pad), Math.ceil(layer.box.w + pad * 2), Math.ceil(layer.box.h + pad * 2)].join(","));
  }
  return regions;
}

const names = existsSync(imported) ? readdirSync(imported).filter((name) => existsSync(join(imported, name, "doc.json"))).sort() : [];
const wanted = positionals.length ? names.filter((n) => positionals.includes(n)) : names;
if (wanted.length === 0) {
  console.log(`No imported templates in ${imported}. Nothing to test (run \`pnpm import:psd\` first).`);
  process.exit(0);
}

mkdirSync(out, { recursive: true });
execFileSync("cargo", ["build", "-q", "--release", "-p", "cardsmith-cli"], { cwd: repo, stdio: "inherit" });
const cardsmith = join(repo, "target/release/cardsmith");

type State = { about?: string; show?: string[]; hide?: string[]; styleOn?: string[] };
const states = JSON.parse(readFileSync(join(import.meta.dirname, "states.json"), "utf8")) as Record<string, Record<string, State> | string>;

/** The layer at a path such as "/Pinlines/Shape/Textbox". Fails if the path is missing or ambiguous. */
function layerAt(root: { children: DocLayer[] }, path: string): DocLayer {
  let level = root.children;
  let found: DocLayer | undefined;
  for (const name of path.split("/").slice(1)) {
    const matches = level.filter((l) => l.name === name);
    if (matches.length !== 1) throw new Error(`"${path}": ${matches.length ? "more than one layer is" : "no layer is"} named "${name}" there`);
    found = matches[0]!;
    level = found.children ?? [];
  }
  if (!found) throw new Error(`"${path}" is not a layer path`);
  return found;
}

let failed = 0;
let skipped = 0;
for (const name of wanted) {
  const docFile = join(imported, name, "doc.json");
  const base = JSON.parse(readFileSync(docFile, "utf8")) as { root: { children: DocLayer[] } };
  const templateStates = typeof states[name] === "object" ? (states[name] as Record<string, State>) : {};

  for (const [stateName, state] of [["default", {} as State], ...Object.entries(templateStates)] as [string, State][]) {
    const label = `${name} [${stateName}]`;
    const doc = structuredClone(base);
    try {
      for (const path of state.show ?? []) layerAt(doc.root, path).visible = true;
      for (const path of state.hide ?? []) layerAt(doc.root, path).visible = false;
      for (const path of state.styleOn ?? []) layerAt(doc.root, path).effectsEnabled = true;
    } catch (err) {
      failed++;
      console.log(`✗ ${label}: ${(err as Error).message}`);
      continue;
    }
    // Written beside doc.json so its asset paths still resolve.
    const stateDoc = stateName === "default" ? docFile : join(imported, name, `doc.${stateName}.json`);
    if (stateName !== "default") writeFileSync(stateDoc, JSON.stringify(doc));
    const reference = join(golden, `${name}.${stateName}.png`);
    const render = join(out, `${name}.${stateName}.png`);
    execFileSync(cardsmith, ["render", stateDoc, "-o", render], { stdio: ["ignore", "ignore", "ignore"] });

    if (values.update === `${name}.${stateName}`) {
      mkdirSync(golden, { recursive: true });
      copyFileSync(render, reference);
      console.log(`↻ ${label}: reference replaced with the current render`);
      continue;
    }
    if (!existsSync(reference)) {
      skipped++;
      console.log(`– ${label}: no reference at ${reference}; rendered only`);
      continue;
    }

    const ignore = textRegions(doc.root.children);
    let result: { psnr: number | null; maxDiff: number; differing: number; compared: number; worstBlock: { x: number; y: number; meanError: number } };
    try {
      result = JSON.parse(execFileSync(cardsmith, ["compare", render, reference, "--heatmap", join(out, `${name}.${stateName}.diff.png`), ...ignore.flatMap((r) => ["--ignore", r])], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }));
    } catch (err) {
      failed++;
      console.log(`✗ ${label}: ${String((err as { stderr?: string }).stderr ?? err).trim()}`);
      continue;
    }

    const limits = { ...thresholds.default, ...thresholds.templates[name], ...thresholds.templates[`${name}.${stateName}`] };
    const psnr = result.psnr ?? Infinity;
    const ok = psnr >= limits.minPsnr && result.worstBlock.meanError <= limits.maxBlockError;
    if (!ok) failed++;
    console.log(
      `${ok ? "✓" : "✗"} ${label}: PSNR ${Number.isFinite(psnr) ? psnr.toFixed(1) + " dB" : "∞"} (min ${limits.minPsnr}), ` +
        `worst block ${result.worstBlock.meanError.toFixed(1)} at (${result.worstBlock.x}, ${result.worstBlock.y}) (max ${limits.maxBlockError}), ` +
        `${((100 * result.differing) / result.compared).toFixed(2)}% of pixels differ, largest difference ${result.maxDiff}/255, ` +
        `${ignore.length} text area${ignore.length === 1 ? "" : "s"} left out`,
    );
  }
}
console.log(`\nRenders and heatmaps are in ${out}.`);
if (skipped) console.log(`${skipped} state${skipped === 1 ? "" : "s"} had no reference to compare with.`);
process.exit(failed ? 1 : 0);
