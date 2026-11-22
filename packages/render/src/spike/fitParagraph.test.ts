import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it } from "vitest";
import type { CanvasKit } from "canvaskit-wasm";
import { loadCanvasKit } from "../canvaskit.js";
import { fitParagraph, makeFontProvider, renderSpike, type FontSpec } from "./fitParagraph.js";
import { SAMPLE_OPTIONS, SAMPLE_RUNS, loadSampleFont } from "./sample.js";

let ck: CanvasKit;
let font: FontSpec;

beforeAll(async () => {
  ck = await loadCanvasKit();
  font = await loadSampleFont();
});

/** Total ink (non-white) pixels in a decoded PNG, optionally inside one rect. */
function inkPixels(png: Uint8Array, inside?: { x: number; y: number; w: number; h: number }) {
  const image = ck.MakeImageFromEncoded(png)!;
  const w = image.width();
  const h = image.height();
  const px = image.readPixels(0, 0, {
    width: w,
    height: h,
    colorType: ck.ColorType.RGBA_8888,
    alphaType: ck.AlphaType.Unpremul,
    colorSpace: ck.ColorSpace.SRGB,
  }) as Uint8Array;
  image.delete();
  let ink = 0;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      if (inside && (x < inside.x || x >= inside.x + inside.w || y < inside.y || y >= inside.y + inside.h)) continue;
      const i = (y * w + x) * 4;
      if (px[i]! < 128 && px[i + 1]! < 128 && px[i + 2]! < 128) ink++;
    }
  }
  return { ink, width: w, height: h };
}

describe("CanvasKit paragraph spike (headless)", () => {
  it("shrinks rules text with inline placeholders until it fits the box", async () => {
    const { png, result, box } = renderSpike(ck, font, SAMPLE_RUNS, SAMPLE_OPTIONS);

    // The sample must overflow at the starting size, or the test proves nothing.
    expect(result.initialHeight).toBeGreaterThan(SAMPLE_OPTIONS.height);
    expect(result.fits).toBe(true);
    expect(result.height).toBeLessThanOrEqual(SAMPLE_OPTIONS.height);
    expect(result.fontSize).toBeLessThan(SAMPLE_OPTIONS.fontSize);
    expect(result.fontSize).toBeGreaterThanOrEqual(SAMPLE_OPTIONS.minFontSize!);
    // One step larger must not fit: we stopped at the largest size that does.
    const fonts = makeFontProvider(ck, [font]);
    const bigger = fitParagraph(ck, fonts, font.family, SAMPLE_RUNS, {
      ...SAMPLE_OPTIONS,
      fontSize: result.fontSize + SAMPLE_OPTIONS.step!,
      minFontSize: result.fontSize + SAMPLE_OPTIONS.step!,
    });
    expect(bigger.result.fits).toBe(false);
    bigger.paragraph.delete();
    fonts.delete();

    // Every symbol run got a placeholder rect, inside the box, sized from the font.
    const symbols = SAMPLE_RUNS.filter((r) => r.kind === "symbol");
    expect(result.symbolRects).toHaveLength(symbols.length);
    for (const rect of result.symbolRects) {
      expect(rect.w).toBeCloseTo(result.fontSize * 0.85, 1);
      expect(rect.x).toBeGreaterThanOrEqual(0);
      expect(rect.x + rect.w).toBeLessThanOrEqual(SAMPLE_OPTIONS.width + 0.5);
      expect(rect.y + rect.h).toBeLessThanOrEqual(result.height + 0.5);
    }

    // The PNG is real and has ink inside the box.
    expect([...png.slice(0, 8)]).toEqual([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
    const { ink, width, height } = inkPixels(png, box);
    expect(width).toBe(SAMPLE_OPTIONS.width + 48);
    expect(height).toBe(SAMPLE_OPTIONS.height + 48);
    expect(ink).toBeGreaterThan(5000);
    // Nothing is drawn below the box: the fitted text did not spill.
    const below = inkPixels(png, { x: 0, y: box.y + box.h + 2, w: width, h: height });
    expect(below.ink).toBe(0);

    const out = fileURLToPath(new URL("../../out/spike-text-fit.png", import.meta.url));
    await mkdir(fileURLToPath(new URL("../../out/", import.meta.url)), { recursive: true });
    await writeFile(out, png);
  });

  it("uses the italic face for italic runs", () => {
    const opts = { width: 700, height: 80, fontSize: 40 };
    const render = (italic: boolean, f: FontSpec) =>
      renderSpike(ck, f, [{ kind: "text", text: "Reminder text looks like this.", italic }], opts).png;
    const upright = render(false, font);
    const italic = render(true, font);
    // DejaVu Serif Italic shares its advance widths with the upright face, so
    // compare pixels, not measurements.
    expect(Buffer.from(italic).equals(Buffer.from(upright))).toBe(false);
    // Finding: SkParagraph synthesizes a slant when the family has no italic
    // face. The faux result differs from the real face, which is how we know the
    // real one was matched by style above. (The real engine must detect a
    // missing face itself; Skia will not tell us.)
    const uprightOnly: FontSpec = { family: font.family, faces: [font.faces[0]!] };
    const faux = render(true, uprightOnly);
    expect(Buffer.from(faux).equals(Buffer.from(upright))).toBe(false);
    expect(Buffer.from(faux).equals(Buffer.from(italic))).toBe(false);
  });

  it("reports fits=false instead of looping when the minimum size is too big", () => {
    const fonts = makeFontProvider(ck, [font]);
    const { paragraph, result } = fitParagraph(ck, fonts, font.family, SAMPLE_RUNS, {
      ...SAMPLE_OPTIONS,
      height: 40,
      minFontSize: 20,
    });
    expect(result.fits).toBe(false);
    expect(result.fontSize).toBe(20);
    paragraph.delete();
    fonts.delete();
  });
});
