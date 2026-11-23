/**
 * pnpm test:parity [--app] [--timeout <seconds>]
 *
 * SPEC Phase 1 acceptance: the same document rendered in Node and in the app
 * must differ by at most 1/255 per channel.
 *
 * Renders the sample documents in Node, then has the frontend render the very
 * same input and post its pixels back through the dev server (see the parity
 * plugin in vite.config.ts and src/lib/parity.ts).
 *
 *   default   the frontend runs in headless Chromium (needs Playwright)
 *   --app     the frontend runs in the real Tauri window (needs a display)
 */
import { spawn, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { Renderer, buildSamples, loadCanvasKit } from "@cardsmith/render";

const LIMIT = 1; // per channel, out of 255
const appDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const { values } = parseArgs({ options: { app: { type: "boolean" }, timeout: { type: "string", default: "600" } } });

// 1. Render in Node and write the shared input.
const ck = await loadCanvasKit();
const renderer = new Renderer(ck);
const samples = buildSamples(ck);
const expected = samples.map((s) => renderer.render(s.doc, s.assets));
const dir = await mkdtemp(join(tmpdir(), "cardsmith-parity-"));
await writeFile(
  join(dir, "input.json"),
  JSON.stringify(samples.map((s) => ({
    name: s.name,
    doc: s.doc,
    assets: Object.fromEntries(Object.entries(s.assets).map(([id, bytes]) => [id, Buffer.from(bytes).toString("base64")])),
  }))),
);

// 2. Start the frontend with the parity endpoints switched on.
const env = { ...process.env, CARDSMITH_PARITY_DIR: dir };
const children: ChildProcess[] = [];
const start = (command: string, args: string[]) => {
  const child = spawn(command, args, { cwd: appDir, env, stdio: ["ignore", "pipe", "pipe"], detached: true });
  children.push(child);
  return child;
};
const stop = () => {
  for (const child of children) {
    try {
      process.kill(-child.pid!, "SIGTERM"); // the whole process group: pnpm, vite, cargo, the app
    } catch {
      // already gone
    }
  }
};
process.on("exit", stop);

let closeBrowser = async () => {};
if (values.app) {
  if (!process.env.DISPLAY && !process.env.WAYLAND_DISPLAY) {
    console.error("--app needs a display. On a headless machine run: xvfb-run -a pnpm test:parity --app");
    process.exit(2);
  }
  console.error("Starting the Tauri app (the first build can take a few minutes)…");
  start("pnpm", ["tauri", "dev"]);
} else {
  // Playwright is optional and not a dependency, so only the calls used here are typed.
  type Browser = { newPage(): Promise<{ goto(url: string): Promise<unknown> }>; close(): Promise<void> };
  let playwright: { chromium: { launch(): Promise<Browser> } };
  try {
    playwright = createRequire(import.meta.url)("playwright");
  } catch {
    console.error("Skipped: Playwright is not installed. Run `pnpm test:parity --app` to check the real app instead.");
    process.exit(0);
  }
  const vite = start("pnpm", ["dev"]);
  await new Promise<void>((ok, fail) => {
    vite.stdout!.on("data", (chunk: Buffer) => chunk.toString().includes("localhost:1420") && ok());
    vite.on("exit", (code) => fail(new Error(`The dev server exited early (code ${code}). Is port 1420 in use?`)));
  });
  const browser = await playwright.chromium.launch();
  closeBrowser = () => browser.close();
  await (await browser.newPage()).goto("http://localhost:1420");
}

// 3. Wait for the frontend's pixels.
const resultFile = join(dir, "result.json");
const deadline = Date.now() + Number(values.timeout) * 1000;
while (!existsSync(resultFile)) {
  if (Date.now() > deadline) {
    console.error(`Timed out after ${values.timeout}s waiting for the app to render.`);
    stop();
    process.exit(1);
  }
  await new Promise((r) => setTimeout(r, 250));
}
await new Promise((r) => setTimeout(r, 200)); // let the write finish
const { userAgent, results } = JSON.parse(await readFile(resultFile, "utf8")) as {
  userAgent: string;
  results: { name: string; width: number; height: number; pixels: string; warnings: string[] }[];
};
await closeBrowser();
stop();

// 4. Compare.
console.log(`Frontend: ${userAgent}`);
let failed = false;
samples.forEach((sample, i) => {
  const want = expected[i]!;
  const got = results.find((r) => r.name === sample.name);
  if (!got || got.width !== want.width || got.height !== want.height) {
    failed = true;
    return console.log(`✗ ${sample.name}: missing or wrong size`);
  }
  const pixels = Buffer.from(got.pixels, "base64");
  let max = 0;
  let differing = 0;
  for (let p = 0; p < want.pixels.length; p++) {
    const d = Math.abs(want.pixels[p]! - pixels[p]!);
    if (d) differing++;
    if (d > max) max = d;
  }
  const ok = max <= LIMIT && got.warnings.length === want.warnings.length;
  failed ||= !ok;
  console.log(`${ok ? "✓" : "✗"} ${sample.name}: max difference ${max}/255, ${differing} of ${want.pixels.length} channel values differ`);
});
process.exit(failed ? 1 : 0);
