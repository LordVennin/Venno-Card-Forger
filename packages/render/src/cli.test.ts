import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { stringifyDoc } from "@cardsmith/doc";
import { describe, expect, it } from "vitest";
import { loadCanvasKit } from "./canvaskit.js";
import { main } from "./cli.js";
import { Renderer } from "./render.js";
import { buildSamples } from "./samples.js";
import { maxDiff } from "./testing.js";

const PNG = [0x89, 0x50, 0x4e, 0x47];

async function decode(file: string) {
  const ck = await loadCanvasKit();
  const image = ck.MakeImageFromEncoded(await readFile(file))!;
  const width = image.width(), height = image.height();
  const pixels = image.readPixels(0, 0, { width, height, colorType: ck.ColorType.RGBA_8888, alphaType: ck.AlphaType.Unpremul, colorSpace: ck.ColorSpace.SRGB }) as Uint8Array;
  image.delete();
  return { width, height, pixels: Uint8Array.from(pixels), warnings: [] };
}

describe("pnpm render", () => {
  it("renders samples, then re-renders them from .cardsmith and from doc.json identically", async () => {
    const dir = await mkdtemp(join(tmpdir(), "cardsmith-render-"));
    const log: string[] = [];
    expect(await main(["--samples", dir], (l) => log.push(l))).toBe(0);

    const ck = await loadCanvasKit();
    const renderer = new Renderer(ck);
    for (const sample of buildSamples(ck)) {
      const direct = renderer.render(sample.doc, sample.assets);
      expect(direct.warnings).toEqual([]);
      const written = await decode(join(dir, `${sample.name}.png`));
      expect(maxDiff(direct, written)).toBe(0);

      // From the save file.
      const fromZip = join(dir, `${sample.name}.zip.png`);
      expect(await main([join(dir, `${sample.name}.cardsmith`), "-o", fromZip], () => {})).toBe(0);
      expect(maxDiff(direct, await decode(fromZip))).toBe(0);

      // From a loose doc.json with assets beside it.
      const loose = join(dir, `${sample.name}-loose`);
      await mkdir(join(loose, "assets"), { recursive: true });
      await writeFile(join(loose, "doc.json"), stringifyDoc(sample.doc));
      for (const [id, ref] of Object.entries(sample.doc.assets)) await writeFile(join(loose, ref.path), sample.assets[id]!);
      const fromJson = join(loose, "out.png");
      expect(await main([join(loose, "doc.json"), "-o", fromJson], () => {})).toBe(0);
      expect(maxDiff(direct, await decode(fromJson))).toBe(0);
      expect([...(await readFile(fromJson)).subarray(0, 4)]).toEqual(PNG);
    }
    renderer.dispose();
  });

  it("explains bad input instead of crashing", async () => {
    const dir = await mkdtemp(join(tmpdir(), "cardsmith-render-"));
    const log: string[] = [];
    expect(await main([], (l) => log.push(l))).toBe(2);
    expect(log.join("\n")).toMatch(/usage/);

    await writeFile(join(dir, "bad.json"), '{"version": 7}');
    log.length = 0;
    expect(await main([join(dir, "bad.json"), "-o", join(dir, "x.png")], (l) => log.push(l))).toBe(1);
    expect(log.join("\n")).toMatch(/unsupported version 7/);

    log.length = 0;
    expect(await main([join(dir, "missing.json"), "-o", join(dir, "x.png")], (l) => log.push(l))).toBe(1);
    expect(log.join("\n")).toMatch(/error: .*missing\.json/);
  });

  it("warns about assets missing beside a doc.json but still renders", async () => {
    const dir = await mkdtemp(join(tmpdir(), "cardsmith-render-"));
    const ck = await loadCanvasKit();
    const sample = buildSamples(ck)[0]!;
    await writeFile(join(dir, "doc.json"), stringifyDoc(sample.doc));
    const log: string[] = [];
    expect(await main([join(dir, "doc.json"), "-o", join(dir, "out.png")], (l) => log.push(l))).toBe(0);
    expect(log.join("\n")).toMatch(/warning: Asset "backdrop" has no data/);
  });
});
