/**
 * pnpm test:cards [template...] [--update <template>/<card> | --update all]
 *
 * Builds every card in tests/cards/cards.json (the cards Proxyshop renders to
 * test its normal layout) from every imported template the card engine can
 * build, and renders it. It checks that the engine raised no warning and,
 * where a reference exists, that the render matches it.
 *
 *   fixtures/local/imported/<template>/                from `pnpm import:psd`
 *   fixtures/local/fonts/                              the fonts
 *   fixtures/local/card-golden/<template>/<card>.png   references
 *
 * With no templates or no fonts it skips cleanly. Output goes to
 * out/cards/<template>/: one image per card and sheet.png with all of them.
 * The art is tests/cards/art.png, a gradient standing in for a picture.
 *
 * `--update` replaces references with the current renders. Do that only after
 * a person has looked at the new output and confirmed it is right.
 */
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";

const repo = resolve(import.meta.dirname, "../..");
const imported = join(repo, "fixtures/local/imported");
const fonts = join(repo, "fixtures/local/fonts");
const golden = join(repo, "fixtures/local/card-golden");
const { values, positionals } = parseArgs({ allowPositionals: true, options: { update: { type: "string" } } });

/** Compared like the template goldens: see tests/golden/thresholds.json. */
const limits = { minPsnr: 40, maxBlockError: 7 };

type Manifest = { bindings?: { kind?: string } };
const all = existsSync(imported) ? readdirSync(imported).filter((name) => existsSync(join(imported, name, "manifest.json"))).sort() : [];
const buildable = all.filter((name) => ["raster", "vector"].includes((JSON.parse(readFileSync(join(imported, name, "manifest.json"), "utf8")) as Manifest).bindings?.kind ?? ""));
const templates = positionals.length ? buildable.filter((n) => positionals.includes(n)) : buildable;
if (templates.length === 0) {
  console.log(`No imported templates the card engine can build in ${imported}. Nothing to test.`);
  process.exit(0);
}
if (!existsSync(fonts) || !readdirSync(fonts).some((f) => /\.(ttf|otf)$/i.test(f))) {
  console.log(`No fonts in ${fonts}. Nothing to test.`);
  process.exit(0);
}
for (const skipped of all.filter((n) => !buildable.includes(n))) console.log(`– ${skipped}: no bindings the card engine understands`);

execFileSync("cargo", ["build", "-q", "--release", "-p", "cardsmith-cli"], { cwd: repo, stdio: "inherit" });
const cardsmith = join(repo, "target/release/cardsmith");
const suite = JSON.parse(readFileSync(join(repo, "tests/cards/cards.json"), "utf8")) as { cards: { case: string; card: { name: string } }[] };
const slug = (name: string) => name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");

let failed = 0;
let unreferenced = 0;
for (const template of templates) {
  const out = join(repo, "out/cards", template);
  mkdirSync(out, { recursive: true });
  const renders: string[] = [];
  for (const { case: what, card } of suite.cards) {
    const key = slug(card.name);
    const data = join(out, `${key}.json`);
    const render = join(out, `${key}.png`);
    writeFileSync(data, JSON.stringify(card));
    const built = spawnSync(cardsmith, ["build", join(imported, template), data, "-o", render, "--fonts", fonts, "--art", join(repo, "tests/cards/art.png")], { encoding: "utf8" });
    const problems = built.stderr.split("\n").filter((line) => line.startsWith("warning:") || line.startsWith("error:")).map((line) => line.replace(/^\w+: /, ""));
    if (built.status !== 0 && problems.length === 0) problems.push(built.stderr.trim() || "the build failed");

    let compared = "no reference";
    const reference = join(golden, template, `${key}.png`);
    if (built.status === 0) {
      renders.push(render);
      if (values.update === `${template}/${key}` || values.update === "all") {
        mkdirSync(join(golden, template), { recursive: true });
        copyFileSync(render, reference);
        compared = "reference replaced with the current render";
      } else if (existsSync(reference)) {
        try {
          const r = JSON.parse(execFileSync(cardsmith, ["compare", render, reference, "--heatmap", join(out, `${key}.diff.png`)], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] })) as { psnr: number | null; worstBlock: { meanError: number } };
          const psnr = r.psnr ?? Infinity;
          compared = `PSNR ${Number.isFinite(psnr) ? psnr.toFixed(1) + " dB" : "∞"}, worst block ${r.worstBlock.meanError.toFixed(1)}`;
          if (psnr < limits.minPsnr || r.worstBlock.meanError > limits.maxBlockError) problems.push(`differs from its reference (limits ${limits.minPsnr} dB, ${limits.maxBlockError})`);
        } catch (err) {
          problems.push(String((err as { stderr?: string }).stderr ?? err).trim());
        }
      } else {
        unreferenced++;
      }
    }
    if (problems.length) failed++;
    console.log(`${problems.length ? "✗" : "✓"} ${template} / ${card.name} (${what}): ${compared}${problems.length ? " — " + problems.join("; ") : ""}`);
  }
  if (renders.length) execFileSync(cardsmith, ["sheet", "-o", join(out, "sheet.png"), "--across", "4", "--shrink", "4", ...renders]);
}
console.log(`\nRenders and a sheet per template are in ${join(repo, "out/cards")}.`);
if (unreferenced) console.log(`${unreferenced} had no reference in ${golden}; they were only checked for engine warnings.`);
process.exit(failed ? 1 : 0);
