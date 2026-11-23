import { execFileSync } from "node:child_process";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { encodePng } from "./png.js";

describe("encodePng", () => {
  it("writes files the Rust core decodes back to the same pixels", async () => {
    // A 3×2 RGBA image with distinct values, including transparency, and a doc that shows it unscaled.
    const pixels = new Uint8Array([255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255]);
    const dir = await mkdtemp(join(tmpdir(), "cardsmith-png-"));
    await writeFile(join(dir, "a.png"), encodePng(3, 2, pixels));
    const base = { visible: true, opacity: 1, blend: "normal", clip: false };
    const doc = {
      version: 1, width: 3, height: 2, dpi: 72, assets: { a: { kind: "image", path: "a.png" } },
      root: { kind: "group", id: "root", name: "Root", ...base, passThrough: true, children: [{ kind: "pixel", id: "p", name: "P", ...base, asset: "a", transform: [1, 0, 0, 0, 1, 0] }] },
    };
    await writeFile(join(dir, "doc.json"), JSON.stringify(doc));
    // Round-trip through the real renderer: its output PNG must be byte-identical in pixels.
    execFileSync("cargo", ["run", "-q", "--release", "-p", "cardsmith-cli", "--", "render", join(dir, "doc.json"), "-o", join(dir, "out.png"), "--raw", join(dir, "out.rgba")], { stdio: "pipe" });
    const { readFile } = await import("node:fs/promises");
    expect([...(await readFile(join(dir, "out.rgba")))]).toEqual([...pixels].map((v, i) => (i % 4 !== 3 && pixels[i - (i % 4) + 3] === 0 ? 0 : v)));
  }, 120_000);

  it("rejects a buffer of the wrong size and writes grayscale", () => {
    expect(() => encodePng(2, 2, new Uint8Array(3))).toThrow(/expected 16 bytes/);
    const gray = encodePng(2, 1, new Uint8Array([0, 255]), 1);
    expect([...gray.subarray(0, 4)]).toEqual([0x89, 0x50, 0x4e, 0x47]);
    expect(gray[25]).toBe(0); // color type: grayscale
  });
});
