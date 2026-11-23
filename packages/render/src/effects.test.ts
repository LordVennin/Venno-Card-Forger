import { group, rectPath, shape, type Color, type Effect, type Layer } from "@cardsmith/doc";
import type { CanvasKit } from "canvaskit-wasm";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { loadCanvasKit } from "./canvaskit.js";
import { Renderer } from "./render.js";
import { BLACK, BLUE, GREEN, RED, WHITE, pixelAt, scene } from "./testing.js";

let ck: CanvasKit;
let renderer: Renderer;
beforeAll(async () => {
  ck = await loadCanvasKit();
  renderer = new Renderer(ck);
});
afterAll(() => renderer.dispose());

const GRAY: Color = [0.5, 0.5, 0.5, 1];
const paint = { enabled: true, blend: "normal", opacity: 1 } as const;

/** A 40×40 document with a 20×20 square at (10,10): edges at 10 and 30. */
function square(effects: Effect[], extra: Partial<Layer> = {}, under: Layer[] = []) {
  const layer = { ...shape({ id: "sq", d: rectPath(10, 10, 20, 20), color: GRAY }), effects, ...extra } as Layer;
  const { doc, assets } = scene(40, 40, [...under, layer]);
  const r = renderer.render(doc, assets);
  return (x: number, y: number) => pixelAt(r, x, y);
}
const backdrop = (color: Color) => shape({ id: "bg", d: rectPath(0, 0, 40, 40), color });
const CLEAR = [0, 0, 0, 0];
const near = (actual: number[], expected: number[], tolerance = 1) =>
  actual.forEach((v, i) => expect(Math.abs(v - expected[i]!), `got [${actual}] want [${expected}]`).toBeLessThanOrEqual(tolerance));

describe("color overlay", () => {
  it("recolors the layer and keeps its outline", () => {
    const at = square([{ ...paint, type: "colorOverlay", color: RED }]);
    expect(at(20, 20)).toEqual([255, 0, 0, 255]);
    expect(at(5, 5)).toEqual(CLEAR);
  });

  it("still shows at fill opacity 0, and then blends with the backdrop", () => {
    const at = square([{ ...paint, type: "colorOverlay", color: GRAY, blend: "multiply" }], { fillOpacity: 0 }, [backdrop(RED)]);
    near(at(20, 20), [128, 0, 0, 255]);
    expect(at(5, 5)).toEqual([255, 0, 0, 255]);
  });

  it("respects its own opacity", () => {
    const at = square([{ ...paint, type: "colorOverlay", color: WHITE, opacity: 0.5 }]);
    near(at(20, 20), [191, 191, 191, 255]);
  });
});

describe("gradient overlay", () => {
  const gradient = { stops: [{ offset: 0, color: RED }, { offset: 1, color: BLUE }] };
  const overlay = (extra: Partial<Extract<Effect, { type: "gradientOverlay" }>>) =>
    square([{ ...paint, type: "gradientOverlay", gradient, style: "linear", angle: 0, scale: 1, ...extra }]);

  it("runs along its angle across the layer's bounds", () => {
    const right = overlay({ angle: 0 });
    expect(right(10, 20)[0]).toBeGreaterThan(240); // starts red on the left
    expect(right(29, 20)[2]).toBeGreaterThan(240); // ends blue on the right
    near(right(20, 12), right(20, 27)); // constant across the gradient
    const up = overlay({ angle: 90 });
    expect(up(20, 29)[0]).toBeGreaterThan(240); // 90° points up: starts at the bottom
    expect(up(20, 10)[2]).toBeGreaterThan(240);
  });

  it("reverses", () => {
    const at = overlay({ reverse: true });
    expect(at(10, 20)[2]).toBeGreaterThan(240);
    expect(at(29, 20)[0]).toBeGreaterThan(240);
  });

  it("scale squeezes the gradient around the center", () => {
    const at = overlay({ scale: 0.2 }); // spans x = 18..22
    expect(at(14, 20)).toEqual([255, 0, 0, 255]);
    expect(at(26, 20)).toEqual([0, 0, 255, 255]);
  });

  it("radial starts at the center", () => {
    const at = overlay({ style: "radial" });
    expect(at(20, 20)[0]).toBeGreaterThan(230);
    expect(at(10, 20)[2]).toBeGreaterThan(230);
    near(at(20, 11), at(11, 20), 3);
  });

  it("stays inside the layer", () => {
    expect(overlay({})(5, 20)).toEqual(CLEAR);
  });
});

describe("stroke", () => {
  const stroke = (position: "outside" | "inside" | "center", size = 3, extra: Partial<Layer> = {}) =>
    square([{ ...paint, type: "stroke", color: RED, size, position }], extra);

  it("outside: a band of `size` pixels around the layer", () => {
    const at = stroke("outside");
    expect(at(9, 20)).toEqual([255, 0, 0, 255]);
    expect(at(7, 20)).toEqual([255, 0, 0, 255]);
    expect(at(6, 20)).toEqual(CLEAR);
    expect(at(10, 20)).toEqual([128, 128, 128, 255]);
  });

  it("outside: rounds convex corners", () => {
    const at = stroke("outside", 6);
    expect(at(4, 20)).toEqual([255, 0, 0, 255]); // 6 px straight out from the edge
    expect(at(5, 5)[3]).toBe(0); // the square corner of the bounding box is beyond the radius
    expect(at(6, 6)[3]).toBeGreaterThan(200); // but just inside the arc is painted
  });

  it("inside: covers the layer's own edge pixels", () => {
    const at = stroke("inside");
    expect(at(9, 20)).toEqual(CLEAR);
    expect(at(10, 20)).toEqual([255, 0, 0, 255]);
    expect(at(12, 20)).toEqual([255, 0, 0, 255]);
    expect(at(13, 20)).toEqual([128, 128, 128, 255]);
  });

  it("center: straddles the edge", () => {
    const at = stroke("center", 4);
    expect(at(8, 20)).toEqual([255, 0, 0, 255]);
    expect(at(11, 20)).toEqual([255, 0, 0, 255]);
    expect(at(7, 20)).toEqual(CLEAR);
    expect(at(12, 20)).toEqual([128, 128, 128, 255]);
  });

  it("is not faded by fill opacity", () => {
    const at = stroke("outside", 3, { fillOpacity: 0 });
    expect(at(8, 20)).toEqual([255, 0, 0, 255]);
    expect(at(20, 20)).toEqual(CLEAR);
  });

  it("paints the first of two strokes on top", () => {
    const at = square([
      { ...paint, type: "stroke", color: RED, size: 2, position: "outside" },
      { ...paint, type: "stroke", color: BLUE, size: 5, position: "outside" },
    ]);
    expect(at(9, 20)).toEqual([255, 0, 0, 255]);
    expect(at(6, 20)).toEqual([0, 0, 255, 255]);
  });

  it("follows the masked outline, and works at the document edge", () => {
    const at = stroke("outside", 2, { vectorMask: { path: { d: rectPath(0, 0, 20, 40) } } });
    expect(at(20, 20)).toEqual([255, 0, 0, 255]); // the mask's cut edge gets a stroke
    expect(at(25, 20)).toEqual(CLEAR);

    const layer = { ...shape({ id: "edge", d: rectPath(0, 0, 40, 10), color: GRAY }), effects: [{ ...paint, type: "stroke", color: RED, size: 3, position: "outside" }] } as Layer;
    const { doc, assets } = scene(40, 40, [layer]);
    const r = renderer.render(doc, assets);
    expect(pixelAt(r, 20, 11)).toEqual([255, 0, 0, 255]);
    expect(pixelAt(r, 0, 0)).toEqual([128, 128, 128, 255]);
  });
});

describe("drop shadow", () => {
  const shadow = (extra: Partial<Extract<Effect, { type: "dropShadow" }>>, layer: Partial<Layer> = {}) =>
    square([{ ...paint, type: "dropShadow", color: BLACK, angle: 90, distance: 4, spread: 0, size: 0, ...extra }], layer);

  it("falls away from the light by `distance`", () => {
    const at = shadow({}); // light from above, so the shadow drops down
    expect(at(20, 32)).toEqual([0, 0, 0, 255]);
    expect(at(20, 33)).toEqual([0, 0, 0, 255]);
    expect(at(20, 34)).toEqual(CLEAR);
    expect(at(20, 8)).toEqual(CLEAR);
    expect(at(8, 20)).toEqual(CLEAR);
    const side = shadow({ angle: 180 }); // light from the left
    expect(side(32, 20)).toEqual([0, 0, 0, 255]);
    expect(side(8, 20)).toEqual(CLEAR);
  });

  it("is knocked out by the layer, even at fill opacity 0", () => {
    const at = shadow({}, { fillOpacity: 0 });
    expect(at(20, 25)).toEqual(CLEAR);
    expect(at(20, 32)).toEqual([0, 0, 0, 255]);
  });

  it("softens with size and hardens with spread", () => {
    const soft = shadow({ distance: 0, size: 8 });
    const a = [31, 33, 35, 37].map((y) => soft(20, y)[3]!);
    expect(a[0]).toBeGreaterThan(a[1]!);
    expect(a[1]).toBeGreaterThan(a[2]!);
    expect(a[2]).toBeGreaterThan(a[3]!);
    expect(a[0]).toBeLessThan(160);
    const hard = shadow({ distance: 0, size: 6, spread: 1 });
    expect(hard(20, 34)).toEqual([0, 0, 0, 255]);
    expect(hard(20, 37)).toEqual(CLEAR);
  });

  it("the Gaussian contour thins the outer falloff", () => {
    const linear = shadow({ distance: 0, size: 10 });
    const curved = shadow({ distance: 0, size: 10, contour: "gaussian" });
    // Outside the layer the blurred matte is below one half, where the S-curve pulls values down.
    const pairs = [31, 33, 35].map((y) => [linear(20, y)[3]!, curved(20, y)[3]!]);
    for (const [lin, cur] of pairs) {
      expect(lin).toBeGreaterThan(0);
      expect(cur).toBeLessThan(lin!);
    }
  });

  it("blends with the backdrop through its own mode and opacity", () => {
    const at = square(
      [{ ...paint, type: "dropShadow", color: GRAY, blend: "multiply", opacity: 0.5, angle: 90, distance: 4, spread: 0, size: 0 }],
      {},
      [backdrop(RED)],
    );
    near(at(20, 32), [191, 0, 0, 255]);
  });
});

describe("outer glow", () => {
  it("surrounds the layer evenly", () => {
    const at = square([{ ...paint, type: "outerGlow", color: GREEN, spread: 0, size: 8 }]);
    const left = at(7, 20), top = at(20, 7), right = at(32, 20), bottom = at(20, 32);
    expect(left[3]).toBeGreaterThan(20);
    near(left, top);
    near(left, right);
    near(left, bottom);
    expect(at(1, 20)[3]).toBeLessThan(left[3]!);
    expect(at(20, 20)).toEqual([128, 128, 128, 255]);
  });
});

describe("inner shadow", () => {
  const inner = (extra: Partial<Extract<Effect, { type: "innerShadow" }>>) =>
    square([{ ...paint, type: "innerShadow", color: BLACK, angle: 90, distance: 3, choke: 0, size: 0, ...extra }]);

  it("darkens the inside edge nearest the light", () => {
    const at = inner({});
    expect(at(20, 10)).toEqual([0, 0, 0, 255]);
    expect(at(20, 12)).toEqual([0, 0, 0, 255]);
    expect(at(20, 13)).toEqual([128, 128, 128, 255]);
    expect(at(20, 29)).toEqual([128, 128, 128, 255]);
    expect(at(20, 9)).toEqual(CLEAR); // never outside the layer
  });

  it("softens with size and rings the whole edge with choke", () => {
    const soft = inner({ distance: 0, size: 6 });
    expect(soft(20, 10)[0]).toBeLessThan(soft(20, 13)[0]!);
    expect(soft(20, 20)).toEqual([128, 128, 128, 255]);
    const ring = inner({ distance: 0, size: 4, choke: 1 });
    for (const [x, y] of [[20, 11], [20, 28], [11, 20], [28, 20]]) expect(ring(x!, y!)).toEqual([0, 0, 0, 255]);
    expect(ring(20, 15)).toEqual([128, 128, 128, 255]);
  });
});

describe("bevel", () => {
  const bevel = (extra: Partial<Extract<Effect, { type: "bevel" }>>) =>
    square([{
      type: "bevel", enabled: true, style: "inner", technique: "chiselHard", direction: "up", depth: 1, size: 4, soften: 0,
      angle: 90, altitude: 30,
      highlight: { color: WHITE, blend: "normal", opacity: 1 },
      shadow: { color: BLACK, blend: "normal", opacity: 1 },
      ...extra,
    }]);

  it("inner, up: lights the edge facing the light and shades the opposite one", () => {
    const at = bevel({});
    expect(at(20, 11)[0]).toBeGreaterThan(200); // top edge faces the light
    expect(at(20, 28)[0]).toBeLessThan(60); // bottom edge faces away
    expect(at(20, 20)).toEqual([128, 128, 128, 255]); // flat top is untouched
    expect(at(20, 8)).toEqual(CLEAR);
  });

  it("is flat-shaded: constant along a straight chiselled face", () => {
    const at = bevel({});
    near(at(16, 11), at(24, 11), 0);
    near(at(20, 11), at(20, 12), 0);
  });

  it("direction down swaps highlight and shadow", () => {
    const at = bevel({ direction: "down" });
    expect(at(20, 11)[0]).toBeLessThan(60);
    expect(at(20, 28)[0]).toBeGreaterThan(200);
  });

  it("follows the light angle", () => {
    const at = bevel({ angle: 180 }); // light from the left
    expect(at(11, 20)[0]).toBeGreaterThan(200);
    expect(at(28, 20)[0]).toBeLessThan(60);
  });

  it("outer: shades the band outside the layer and leaves the layer alone", () => {
    const at = bevel({ style: "outer" });
    expect(at(20, 8)).toEqual([255, 255, 255, at(20, 8)[3]!]);
    expect(at(20, 8)[3]).toBeGreaterThan(200);
    expect(at(20, 31)[3]).toBeGreaterThan(200);
    expect(at(20, 31)[0]).toBe(0);
    expect(at(20, 11)).toEqual([128, 128, 128, 255]);
    expect(at(20, 3)).toEqual(CLEAR);
  });

  it("soften blurs the facets", () => {
    const hard = bevel({})(20, 14)[0]!; // just past the 4 px face
    const soft = bevel({ soften: 6 })(20, 14)[0]!;
    expect(hard).toBe(128);
    expect(soft).toBeGreaterThan(128);
  });
});

describe("effect switches and layer opacity", () => {
  const overlay: Effect = { ...paint, type: "colorOverlay", color: RED };

  it("ignores disabled effects and a switched-off layer style", () => {
    expect(square([{ ...overlay, enabled: false }])(20, 20)).toEqual([128, 128, 128, 255]);
    expect(square([overlay], { effectsEnabled: false })(20, 20)).toEqual([128, 128, 128, 255]);
  });

  it("layer opacity fades the layer and its effects together", () => {
    const at = square(
      [overlay, { ...paint, type: "stroke", color: BLUE, size: 3, position: "outside" }],
      { opacity: 0.5 },
      [backdrop(WHITE)],
    );
    near(at(20, 20), [255, 128, 128, 255]);
    near(at(8, 20), [128, 128, 255, 255]);
    expect(at(2, 2)).toEqual([255, 255, 255, 255]);
  });

  it("works on groups, using the group's merged outline", () => {
    const g = {
      ...group({ id: "g", passThrough: true }, [
        shape({ id: "a", d: rectPath(10, 10, 10, 20), color: GRAY }),
        shape({ id: "b", d: rectPath(20, 10, 10, 20), color: WHITE }),
      ]),
      effects: [{ ...paint, type: "stroke", color: RED, size: 2, position: "outside" }],
    } as Layer;
    const { doc, assets } = scene(40, 40, [g]);
    const r = renderer.render(doc, assets);
    expect(pixelAt(r, 9, 20)).toEqual([255, 0, 0, 255]);
    expect(pixelAt(r, 20, 20)).toEqual([255, 255, 255, 255]); // no stroke on the seam between the children
    expect(pixelAt(r, 19, 20)).toEqual([128, 128, 128, 255]);
  });

  it("an empty layer with effects draws nothing", () => {
    const layer = { ...shape({ id: "none", d: rectPath(50, 50, 5, 5), color: GRAY }), effects: [overlay] } as Layer;
    const { doc, assets } = scene(40, 40, [layer]);
    expect(renderer.render(doc, assets).pixels.every((v) => v === 0)).toBe(true);
  });

  it("clipped layers sit under the base's overlay effects", () => {
    const at = square([{ ...overlay, opacity: 0.5 }], {}, []);
    const base = { ...shape({ id: "sq", d: rectPath(10, 10, 20, 20), color: GRAY }), effects: [{ ...overlay, opacity: 0.5 }] } as Layer;
    const clip = { ...shape({ id: "c", d: rectPath(0, 0, 40, 40), color: BLUE }), clip: true } as Layer;
    const { doc, assets } = scene(40, 40, [base, clip]);
    const r = renderer.render(doc, assets);
    near(pixelAt(r, 20, 20), [128, 0, 128, 255]); // blue from the clipped layer, then 50% red on top
    near(at(20, 20), [191, 64, 64, 255]);
    expect(pixelAt(r, 5, 5)).toEqual(CLEAR);
  });
});
