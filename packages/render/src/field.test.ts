import { describe, expect, it } from "vitest";
import { blur, cover, signedDistance } from "./field.js";

/** Coverage grid from a function of pixel-center coordinates. */
function grid(w: number, h: number, f: (x: number, y: number) => number): Float32Array {
  const a = new Float32Array(w * h);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) a[y * w + x] = f(x, y);
  return a;
}

describe("signedDistance", () => {
  const w = 20, h = 20;
  // Crisp square covering pixels 5..14 in both axes: edges at 5 and 15.
  const square = grid(w, h, (x, y) => (x >= 5 && x < 15 && y >= 5 && y < 15 ? 1 : 0));
  const sd = signedDistance(square, w, h);
  const at = (x: number, y: number) => sd[y * w + x]!;

  it("measures from the pixel edge, not the pixel center", () => {
    expect(at(4, 10)).toBeCloseTo(0.5); // first pixel outside the left edge
    expect(at(2, 10)).toBeCloseTo(2.5);
    expect(at(5, 10)).toBeCloseTo(-0.5); // first pixel inside
    expect(at(9, 10)).toBeCloseTo(-4.5);
    expect(at(10, 17)).toBeCloseTo(2.5);
  });

  it("is Euclidean around corners", () => {
    // 3 px left and 3 px up from the corner pixel (5,5): centers differ by (3,3).
    expect(at(2, 2)).toBeCloseTo(Math.hypot(3, 3) - 0.5, 4);
    expect(at(17, 18)).toBeCloseTo(Math.hypot(3, 4) - 0.5, 4);
  });

  it("uses partial coverage to place the edge inside a pixel", () => {
    // Edge at x = 5.25: pixel 5 is 75% covered.
    const soft = grid(w, h, (x) => (x > 5 ? 1 : x === 5 ? 0.75 : 0));
    const s = signedDistance(soft, w, h);
    expect(s[10 * w + 5]).toBeCloseTo(-0.25);
    expect(s[10 * w + 4]).toBeCloseTo(0.75);
    expect(s[10 * w + 3]).toBeCloseTo(1.75);
    expect(s[10 * w + 6]).toBeCloseTo(-1.25);
  });

  it("picks the nearest edge when a crisp edge and a nearly-full pixel are equally close", () => {
    // Row 0 is empty. In row 1, pixel 3 is 96% covered and the rest are full.
    // Pixel (4,1) is one step from both; its edge is the crisp one above, 0.5 away.
    const a = grid(8, 6, (x, y) => (y === 0 ? 0 : y === 1 && x === 3 ? 0.96 : 1));
    const s = signedDistance(a, 8, 6);
    expect(s[1 * 8 + 4]).toBeCloseTo(-0.5);
    expect(s[2 * 8 + 4]).toBeCloseTo(-1.5);
    expect(s[1 * 8 + 3]).toBeCloseTo(-0.46);
    expect(s[2 * 8 + 3]).toBeCloseTo(-1.46);
  });

  it("handles empty and full regions", () => {
    expect(signedDistance(new Float32Array(16), 4, 4).every((v) => v > 1e9)).toBe(true);
    expect(signedDistance(new Float32Array(16).fill(1), 4, 4).every((v) => v < -1e9)).toBe(true);
  });
});

describe("cover", () => {
  it("ramps across one pixel", () => {
    expect([cover(-2), cover(-0.5), cover(0), cover(0.25), cover(0.5), cover(3)]).toEqual([1, 1, 0.5, 0.25, 0, 0]);
  });
});

describe("blur", () => {
  it("returns the input untouched for a negligible sigma", () => {
    const a = new Float32Array([0, 1, 0]);
    expect(blur(a, 3, 1, 0)).toBe(a);
  });

  it("spreads a point symmetrically and conserves its mass away from edges", () => {
    const w = 41, h = 41;
    const a = grid(w, h, (x, y) => (x === 20 && y === 20 ? 1 : 0));
    const b = blur(a, w, h, 3);
    const at = (x: number, y: number) => b[y * w + x]!;
    expect(b.reduce((s, v) => s + v, 0)).toBeCloseTo(1, 3);
    expect(at(20, 20)).toBeGreaterThan(at(23, 20));
    expect(at(23, 20)).toBeGreaterThan(at(27, 20));
    expect(at(17, 20)).toBeCloseTo(at(23, 20), 6);
    expect(at(20, 17)).toBeCloseTo(at(23, 20), 6);
    // A Gaussian of sigma 3 has its peak at 1 / (2π·9).
    expect(at(20, 20)).toBeCloseTo(1 / (2 * Math.PI * 9), 2);
  });

  it("keeps a flat field flat in the interior", () => {
    const b = blur(new Float32Array(61 * 61).fill(1), 61, 61, 2);
    expect(b[30 * 61 + 30]).toBeCloseTo(1, 5);
    expect(b[0]).toBeLessThan(0.5); // outside the region counts as empty
  });
});
