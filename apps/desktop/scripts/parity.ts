/**
 * pnpm test:parity [--app] [--timeout <seconds>]
 *
 * SPEC Phase 1R acceptance: native and WebAssembly builds of the core render
 * the samples to identical bytes. Compares frame hashes from three places:
 *
 *   1. the native CLI (`cardsmith hashes`)
 *   2. the WASM module loaded in Node
 *   3. the WASM module inside the frontend: headless Chromium by default
 *      (needs Playwright), or the real Tauri window with --app (needs a display)
 */
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { Core } from "../src/lib/core";

const appDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repo = resolve(appDir, "../..");
const { values } = parseArgs({ options: { app: { type: "boolean" }, timeout: { type: "string", default: "900" } } });

type Row = { name: string; size: string; hash: string };
const sh = (command: string, args: string[]) => execFileSync(command, args, { cwd: repo, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });

// 1. Native.
sh("pnpm", ["build:wasm"]);
const native: Row[] = sh("cargo", ["run", "-q", "--release", "-p", "cardsmith-cli", "--", "hashes"])
  .trim()
  .split("\n")
  .map((line) => {
    const [name, size, hash] = line.split(" ");
    return { name: name!, size: size!, hash: hash! };
  });

// 2. WASM in Node.
const core = await Core.load(await readFile(join(appDir, "src/wasm/cardsmith.wasm")));
const inNode: Row[] = core.sampleNames().map((name, index) => {
  core.openSample(index);
  const doc = core.doc();
  return { name, size: `${doc.width}x${doc.height}`, hash: core.frameHash() };
});

// 3. WASM in the frontend.
const dir = await mkdtemp(join(tmpdir(), "cardsmith-parity-"));
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

let frontend: { userAgent: string; results: (Row & { ms: number })[]; bench: string[] } | undefined;
let closeBrowser = async () => {};
let skipped = "";
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
  let playwright: { chromium: { launch(): Promise<Browser> } } | undefined;
  try {
    playwright = createRequire(import.meta.url)("playwright");
  } catch {
    skipped = "Playwright is not installed; run `pnpm test:parity --app` to check the real app";
  }
  if (playwright) {
    const vite = start("pnpm", ["exec", "vite"]);
    await new Promise<void>((ok, fail) => {
      vite.stdout!.on("data", (chunk: Buffer) => chunk.toString().includes("localhost:1420") && ok());
      vite.on("exit", (code) => fail(new Error(`The dev server exited early (code ${code}). Is port 1420 in use?`)));
    });
    const browser = await playwright.chromium.launch();
    closeBrowser = () => browser.close();
    await (await browser.newPage()).goto("http://localhost:1420");
  }
}

if (!skipped) {
  const resultFile = join(dir, "result.json");
  const deadline = Date.now() + Number(values.timeout) * 1000;
  while (!existsSync(resultFile)) {
    if (Date.now() > deadline) {
      console.error(`Timed out after ${values.timeout}s waiting for the frontend to render.`);
      stop();
      process.exit(1);
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  await new Promise((r) => setTimeout(r, 200)); // let the write finish
  frontend = JSON.parse(await readFile(resultFile, "utf8"));
}
await closeBrowser();
stop();

// Compare.
let failed = false;
if (frontend) console.log(`Frontend: ${frontend.userAgent}`);
for (const want of native) {
  const node = inNode.find((r) => r.name === want.name);
  const web = frontend?.results.find((r) => r.name === want.name);
  const same = (r?: Row) => r?.hash === want.hash && r.size === want.size;
  const ok = same(node) && (skipped !== "" || same(web));
  failed ||= !ok;
  const webText = skipped ? "frontend skipped" : `frontend ${web?.hash ?? "missing"}${web ? ` (${web.ms} ms)` : ""}`;
  console.log(`${ok ? "✓" : "✗"} ${want.name} ${want.size}: native ${want.hash}, wasm/node ${node?.hash ?? "missing"}, ${webText}`);
}
if (skipped) console.log(`Note: ${skipped}.`);
if (frontend?.bench.length) console.log(`\nBrush timings in the frontend (card sample, 1300×900 canvas):\n${frontend.bench.join("\n")}`);
process.exit(failed ? 1 : 0);
