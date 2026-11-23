import { describe, expect, it } from "vitest";
import { composite, copyRaster, crossFade, intersect, multiplyAlpha, newRaster, placeImage, resample, stamp, type Raster } from "./raster.js";

function filled(x: number, y: number, w: number, h: number, rgba: number[]): Raster {
  const r = newRaster({ x, y, w, h });
  for (let i = 0; i < r.data.length; i += 4) r.data.set(rgba, i);
  return r;
}
const at = (r: Raster, x: number, y: number) => [...r.data.slice(((y - r.y) * r.w + (x - r.x)) * 4, ((y - r.y) * r.w + (x - r.x)) * 4 + 4)];

describe("composite", () => {
  it("only touches the overlap, wherever the two rasters sit", () => {
    const dst = filled(10, 10, 4, 4, [0, 0, 0, 255]);
    composite(dst, filled(12, 8, 10, 4, [255, 0, 0, 255]), 1, "normal");
    expect(at(dst, 12, 10)).toEqual([255, 0, 0, 255]);
    expect(at(dst, 13, 11)).toEqual([255, 0, 0, 255]);
    expect(at(dst, 11, 10)).toEqual([0, 0, 0, 255]);
    expect(at(dst, 12, 12)).toEqual([0, 0, 0, 255]);
    composite(dst, filled(100, 100, 2, 2, [255, 0, 0, 255]), 1, "normal"); // no overlap: no error
  });

  it("normal over translucent uses straight-alpha source-over", () => {
    const dst = filled(0, 0, 1, 1, [0, 0, 255, 128]);
    composite(dst, filled(0, 0, 1, 1, [255, 0, 0, 128]), 1, "normal");
    // With a = 128/255: alpha = a + a·(1 − a) ≈ .752; red ≈ .667, blue ≈ .333
    const [r, g, b, a] = at(dst, 0, 0);
    expect([r, g, a]).toEqual([170, 0, 192]);
    expect(b).toBeGreaterThanOrEqual(84);
    expect(b).toBeLessThanOrEqual(85);
  });

  it("atop never changes destination alpha", () => {
    for (const mode of ["normal", "multiply", "luminosity"] as const) {
      const dst = filled(0, 0, 2, 1, [200, 100, 50, 77]);
      dst.data[7] = 0;
      composite(dst, filled(0, 0, 2, 1, [10, 20, 30, 255]), 0.6, mode, true);
      expect(at(dst, 0, 0)[3]).toBe(77);
      expect(at(dst, 1, 0)[3]).toBe(0);
    }
  });
});

describe("stamp", () => {
  it("paints a color through coverage, clipped to the destination", () => {
    const dst = filled(0, 0, 4, 1, [0, 0, 0, 255]);
    stamp(dst, { x: 2, y: 0, w: 4, h: 1 }, new Float32Array([1, 0.5, 1, 1]), { r: 255, g: 255, b: 255, a: 1 }, 1, "normal");
    expect(at(dst, 1, 0)).toEqual([0, 0, 0, 255]);
    expect(at(dst, 2, 0)).toEqual([255, 255, 255, 255]);
    expect(at(dst, 3, 0)).toEqual([128, 128, 128, 255]);
  });
});

describe("crossFade", () => {
  it("interpolates in premultiplied space", () => {
    const dst = filled(0, 0, 1, 1, [255, 0, 0, 255]);
    crossFade(dst, filled(0, 0, 1, 1, [0, 0, 255, 0]), 0.5, null);
    // Fading toward transparent keeps the color and halves the alpha.
    expect(at(dst, 0, 0)).toEqual([255, 0, 0, 128]);
  });

  it("respects coverage and leaves equal pixels alone", () => {
    const dst = filled(0, 0, 2, 1, [0, 0, 0, 255]);
    const src = copyRaster(dst);
    src.data.set([255, 255, 255, 255], 0);
    src.data.set([255, 255, 255, 255], 4);
    crossFade(dst, src, 1, new Uint8Array([255, 0]));
    expect(at(dst, 0, 0)).toEqual([255, 255, 255, 255]);
    expect(at(dst, 1, 0)).toEqual([0, 0, 0, 255]);
  });
});

describe("images", () => {
  const image = { w: 2, h: 2, data: new Uint8ClampedArray([255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255]) };
  const whole = { x: 0, y: 0, w: 16, h: 16 };

  it("placeImage crops to the region", () => {
    const r = placeImage(image, -1, 3, whole)!;
    expect([r.x, r.y, r.w, r.h]).toEqual([0, 3, 1, 2]);
    expect(at(r, 0, 3)).toEqual([0, 255, 0, 255]);
    expect(placeImage(image, 40, 40, whole)).toBeNull();
  });

  it("resample at a half-pixel offset blends neighbours and softens the border", () => {
    const r = resample(image, [1, 0, 4.5, 0, 1, 4], whole)!;
    expect(at(r, 4, 4)).toEqual([255, 0, 0, 128]); // half covered by the image's left edge
    const [red, green, , alpha] = at(r, 5, 4); // halfway between the red and green texels
    expect(alpha).toBe(255);
    expect(Math.abs(red! - 128)).toBeLessThanOrEqual(1);
    expect(Math.abs(green! - 128)).toBeLessThanOrEqual(1);
    expect(at(r, 6, 4)).toEqual([0, 255, 0, 128]);
  });

  it("resample does not let transparent texels tint their neighbours", () => {
    const fringe = { w: 2, h: 1, data: new Uint8ClampedArray([255, 0, 0, 255, 0, 255, 0, 0]) };
    const r = resample(fringe, [4, 0, 0, 0, 4, 0], whole)!;
    for (let x = 0; x < 8; x++) {
      const [red, green, , alpha] = at(r, x, 1);
      if (alpha! > 0) expect([red, green]).toEqual([255, 0]);
    }
  });

  it("resample rotates", () => {
    // 90° clockwise about the origin, then moved right by 8: image (u,v) -> (8 - v, u).
    const r = resample(image, [0, -1, 8, 1, 0, 0], whole)!;
    expect(at(r, 7, 0)).toEqual([255, 0, 0, 255]);
    expect(at(r, 7, 1)).toEqual([0, 255, 0, 255]);
    expect(at(r, 6, 0)).toEqual([0, 0, 255, 255]);
  });

  it("resample refuses a degenerate transform", () => {
    expect(resample(image, [0, 0, 0, 0, 0, 0], whole)).toBeNull();
  });
});

describe("helpers", () => {
  it("intersect", () => {
    expect(intersect({ x: 0, y: 0, w: 4, h: 4 }, { x: 2, y: 3, w: 10, h: 10 })).toEqual({ x: 2, y: 3, w: 2, h: 1 });
    expect(intersect({ x: 0, y: 0, w: 4, h: 4 }, { x: 4, y: 0, w: 1, h: 1 })).toBeNull();
  });

  it("multiplyAlpha scales alpha only", () => {
    const r = filled(0, 0, 2, 1, [9, 8, 7, 200]);
    multiplyAlpha(r, new Uint8Array([255, 51]));
    expect(at(r, 0, 0)).toEqual([9, 8, 7, 200]);
    expect(at(r, 1, 0)).toEqual([9, 8, 7, 40]);
  });
});
