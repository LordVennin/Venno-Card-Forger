/**
 * pnpm test:text [--update <card file name without .png> | --update all]
 *
 * Renders the rules text of every card in tests/text/cards.json into the
 * rules box of the normal template, the way the card engine will: symbols,
 * italics, bullets, flavor spacing, shrunk to fit and centered. It then checks
 * that every card fits its box, that no symbol or font is unknown, and, where
 * a reference exists, that the render matches it.
 *
 *   fixtures/local/fonts/               the fonts (without them this skips cleanly)
 *   fixtures/local/text-golden/<card>.png   references
 *
 * Output goes to out/text/: one image per card, sheet.png with all of them,
 * and results.json (size, line count, ink bounds per card).
 *
 * `--update` replaces references with the current renders. Do that only after
 * a person has looked at the new output and confirmed it is right.
 */
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";

const repo = resolve(import.meta.dirname, "../..");
const fonts = join(repo, "fixtures/local/fonts");
const golden = join(repo, "fixtures/local/text-golden");
const out = join(repo, "out/text");
const { values } = parseArgs({ options: { update: { type: "string" } } });

/** Text edges are compared like the template goldens: see tests/golden/thresholds.json. */
const limits = { minPsnr: 40, maxBlockError: 7 };

if (!existsSync(fonts) || !readdirSync(fonts).some((f) => /\.(ttf|otf)$/i.test(f))) {
  console.log(`No fonts in ${fonts}. Nothing to test.`);
  process.exit(0);
}

execFileSync("cargo", ["build", "-q", "--release", "-p", "cardsmith-cli"], { cwd: repo, stdio: "inherit" });
const cardsmith = join(repo, "target/release/cardsmith");
mkdirSync(out, { recursive: true });
execFileSync(cardsmith, ["text", join(repo, "tests/text/cards.json"), "-o", out, "--fonts", fonts], { stdio: ["ignore", "ignore", "ignore"] });

type Result = { name: string; file: string; size: number; lines: number; fits: boolean; unknownSymbols: string[]; missingFonts: string[] };
const results = JSON.parse(readFileSync(join(out, "results.json"), "utf8")) as Result[];

let failed = 0;
let unreferenced = 0;
for (const card of results) {
  const problems: string[] = [];
  if (!card.fits) problems.push("does not fit its box");
  if (card.unknownSymbols.length) problems.push(`unknown symbols ${card.unknownSymbols.join(" ")}`);
  if (card.missingFonts.length) problems.push(`missing fonts ${card.missingFonts.join(", ")}`);

  const key = card.file.replace(/\.png$/, "");
  const reference = join(golden, card.file);
  let compared = "no reference";
  if (values.update === key || values.update === "all") {
    mkdirSync(golden, { recursive: true });
    copyFileSync(join(out, card.file), reference);
    compared = "reference replaced with the current render";
  } else if (existsSync(reference)) {
    try {
      const r = JSON.parse(execFileSync(cardsmith, ["compare", join(out, card.file), reference, "--heatmap", join(out, `${key}.diff.png`)], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] })) as { psnr: number | null; worstBlock: { meanError: number } };
      const psnr = r.psnr ?? Infinity;
      compared = `PSNR ${Number.isFinite(psnr) ? psnr.toFixed(1) + " dB" : "∞"}, worst block ${r.worstBlock.meanError.toFixed(1)}`;
      if (psnr < limits.minPsnr || r.worstBlock.meanError > limits.maxBlockError) problems.push(`differs from its reference (limits ${limits.minPsnr} dB, ${limits.maxBlockError})`);
    } catch (err) {
      problems.push(String((err as { stderr?: string }).stderr ?? err).trim());
    }
  } else {
    unreferenced++;
  }

  if (problems.length) failed++;
  console.log(`${problems.length ? "✗" : "✓"} ${card.name}: ${card.lines} line${card.lines === 1 ? "" : "s"} at ${card.size.toFixed(1)} px, ${compared}${problems.length ? " — " + problems.join("; ") : ""}`);
}
console.log(`\n${results.length} cards. Renders and the sheet are in ${out}.`);
if (unreferenced) console.log(`${unreferenced} had no reference in ${golden}; they were only checked for fit, symbols and fonts.`);
process.exit(failed ? 1 : 0);
