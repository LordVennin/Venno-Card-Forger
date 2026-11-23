import { group, pixel, rectPath, shape, type BlendMode, type Color, type Layer } from "@cardsmith/doc";
import type { CanvasKit } from "canvaskit-wasm";
import { beforeAll, afterAll, describe, expect, it } from "vitest";
import { loadCanvasKit } from "./canvaskit.js";
import { Renderer } from "./render.js";
import { BLACK, BLUE, GREEN, RED, WHITE, pixelAt, png, scene } from "./testing.js";

let ck: CanvasKit;
let renderer: Renderer;
beforeAll(async () => {
  ck = await loadCanvasKit();
  renderer = new Renderer(ck);
});
afterAll(() => renderer.dispose());

const render = (w: number, h: number, layers: Layer[], images?: Record<string, Uint8Array>) => {
  const { doc, assets } = scene(w, h, layers, images);
  return renderer.render(doc, assets);
};
const full = (id: string, color: Color, extra: Partial<Layer> = {}) =>
  ({ ...shape({ id, d: rectPath(0, 0, 8, 8), color }), ...extra }) as Layer;
const box = (id: string, x: number, y: number, w: number, h: number, color: Color, extra: Partial<Layer> = {}) =>
  ({ ...shape({ id, d: rectPath(x, y, w, h), color }), ...extra }) as Layer;

/** Compare with a tolerance of one 8-bit step per channel. */
function near(actual: number[], expected: number[], tolerance = 1) {
  expect(actual.length).toBe(expected.length);
  actual.forEach((v, i) => expect(Math.abs(v - expected[i]!), `channel ${i}: got [${actual}] want [${expected}]`).toBeLessThanOrEqual(tolerance));
}

describe("basics", () => {
  it("draws a shape where its path is and leaves the rest transparent", () => {
    const r = render(8, 8, [box("a", 2, 2, 4, 4, RED)]);
    expect(pixelAt(r, 3, 3)).toEqual([255, 0, 0, 255]);
    expect(pixelAt(r, 1, 1)).toEqual([0, 0, 0, 0]);
    expect(pixelAt(r, 6, 6)).toEqual([0, 0, 0, 0]);
    expect(r.warnings).toEqual([]);
  });

  it("paints children[0] first, so later layers cover earlier ones", () => {
    const r = render(8, 8, [full("bottom", RED), box("top", 0, 0, 4, 8, BLUE)]);
    expect(pixelAt(r, 1, 1)).toEqual([0, 0, 255, 255]);
    expect(pixelAt(r, 6, 1)).toEqual([255, 0, 0, 255]);
  });

  it("skips invisible layers and applies opacity", () => {
    const r = render(8, 8, [full("bg", WHITE), full("hidden", BLUE, { visible: false }), full("half", RED, { opacity: 0.5 })]);
    near(pixelAt(r, 4, 4), [255, 128, 128, 255]);
  });

  it("treats fill opacity like opacity when there are no effects", () => {
    const r = render(8, 8, [full("bg", WHITE), full("a", BLACK, { fillOpacity: 0.25 })]);
    near(pixelAt(r, 4, 4), [191, 191, 191, 255]);
  });

  it("draws shape strokes centered or inside", () => {
    const stroke = (align: "center" | "inside") =>
      render(16, 16, [{ ...shape({ id: "s", d: rectPath(4, 4, 8, 8), color: RED, stroke: { color: BLUE, width: 2, align } }) }]);
    const center = stroke("center");
    expect(pixelAt(center, 3, 8)).toEqual([0, 0, 255, 255]); // 1 px outside the edge
    expect(pixelAt(center, 4, 8)).toEqual([0, 0, 255, 255]); // 1 px inside
    expect(pixelAt(center, 5, 8)).toEqual([255, 0, 0, 255]);
    const inside = stroke("inside");
    expect(pixelAt(inside, 3, 8)).toEqual([0, 0, 0, 0]);
    expect(pixelAt(inside, 5, 8)).toEqual([0, 0, 255, 255]);
    expect(pixelAt(inside, 6, 8)).toEqual([255, 0, 0, 255]);
  });

  it("draws pixel layers through their transform", () => {
    const art = png(ck, 2, 2, (x, y) => (x === 0 && y === 0 ? [255, 0, 0, 255] : [0, 255, 0, 255]));
    // Scale ×2, then move to (3, 1): the red texel covers (3..4, 1..2).
    const r = render(8, 8, [pixel({ id: "p", asset: "art", transform: [2, 0, 3, 0, 2, 1] })], { art });
    expect(pixelAt(r, 3, 1)).toEqual([255, 0, 0, 255]);
    expect(pixelAt(r, 6, 4)).toEqual([0, 255, 0, 255]);
    expect(pixelAt(r, 2, 1)).toEqual([0, 0, 0, 0]);
    expect(pixelAt(r, 7, 4)).toEqual([0, 0, 0, 0]);
  });

  it("reports text layers and missing images instead of failing", () => {
    const { doc, assets } = scene(8, 8, [
      full("bg", WHITE),
      { ...full("t", RED), kind: "text", box: { x: 0, y: 0, w: 8, h: 8 }, content: { runs: [] }, style: { font: "x", size: 1, color: RED, align: "left" }, fit: "none", name: "Rules" } as unknown as Layer,
      pixel({ id: "p", name: "Art", asset: "gone" }),
    ]);
    doc.assets.gone = { kind: "image", path: "assets/gone.png" };
    const r = renderer.render(doc, assets);
    expect(pixelAt(r, 4, 4)).toEqual([255, 255, 255, 255]);
    expect(r.warnings.join("\n")).toMatch(/Text layer "Rules" was skipped/);
    expect(r.warnings.join("\n")).toMatch(/Asset "gone" has no data/);
  });
});

describe("blend modes", () => {
  const S = [0.8, 0.4, 0.1];
  const D = [0.3, 0.6, 0.5];
  const lum = (c: number[]) => 0.3 * c[0]! + 0.59 * c[1]! + 0.11 * c[2]!;
  const softD = (d: number) => (d <= 0.25 ? ((16 * d - 12) * d + 4) * d : Math.sqrt(d));
  const formulas: Record<BlendMode, (s: number, d: number) => number> = {
    normal: (s) => s,
    multiply: (s, d) => s * d,
    screen: (s, d) => s + d - s * d,
    overlay: (s, d) => (d <= 0.5 ? 2 * s * d : 1 - 2 * (1 - s) * (1 - d)),
    softLight: (s, d) => (s <= 0.5 ? d - (1 - 2 * s) * d * (1 - d) : d + (2 * s - 1) * (softD(d) - d)),
    linearLight: (s, d) => Math.min(1, Math.max(0, d + 2 * s - 1)),
    luminosity: () => NaN,
  };
  const expected = (mode: BlendMode) =>
    mode === "luminosity"
      ? D.map((d) => d + (lum(S) - lum(D))) // stays in range for these colors, so no clipping
      : S.map((s, i) => formulas[mode](s, D[i]!));
  const to8 = (c: number[]) => [...c.map((v) => Math.round(v * 255)), 255];
  const modes = Object.keys(formulas) as BlendMode[];

  it.each(modes)("%s matches its formula over an opaque backdrop", (mode) => {
    const r = render(8, 8, [full("d", [...D, 1] as Color), full("s", [...S, 1] as Color, { blend: mode })]);
    near(pixelAt(r, 4, 4), to8(expected(mode)));
  });

  it.each(modes)("%s: the SkSL formula agrees with the path the renderer uses", (mode) => {
    const dst: Color = [...D, 1] as Color;
    for (const alpha of [1, 0.5]) {
      const src: Color = [...S, alpha] as Color;
      near(renderer.blendPixel(dst, src, mode, false, true), renderer.blendPixel(dst, src, mode, false, false), 1);
    }
  });

  it("blends normally where the backdrop is transparent", () => {
    const r = render(8, 8, [full("s", [...S, 1] as Color, { blend: "multiply" })]);
    near(pixelAt(r, 4, 4), to8(S));
  });
});

describe("groups", () => {
  const child = full("m", [0.5, 0.5, 0.5, 1], { blend: "multiply" });

  it("pass-through lets children blend with what is under the group", () => {
    const r = render(8, 8, [full("bg", RED), group({ id: "g", passThrough: true }, [child])]);
    near(pixelAt(r, 4, 4), [128, 0, 0, 255]);
  });

  it("an isolated group blends its children only with each other", () => {
    const r = render(8, 8, [full("bg", RED), group({ id: "g", passThrough: false }, [child])]);
    near(pixelAt(r, 4, 4), [128, 128, 128, 255]);
  });

  it("an isolated group applies its own blend mode and opacity as a unit", () => {
    const r = render(8, 8, [
      full("bg", WHITE),
      group({ id: "g", passThrough: false, opacity: 0.5 }, [full("a", RED), box("b", 0, 0, 4, 8, BLUE)]),
    ]);
    near(pixelAt(r, 1, 4), [128, 128, 255, 255]); // blue over red inside the group, then 50% over white
    near(pixelAt(r, 6, 4), [255, 128, 128, 255]);
  });

  it("pass-through opacity fades the group's whole contribution", () => {
    const r = render(8, 8, [full("bg", RED), group({ id: "g", passThrough: true, opacity: 0.5 }, [child])]);
    near(pixelAt(r, 4, 4), [191, 0, 0, 255]); // halfway between 255 and 128
  });

  it("hides a group's children when the group is hidden", () => {
    const r = render(8, 8, [full("bg", RED), group({ id: "g", visible: false }, [full("a", BLUE)])]);
    expect(pixelAt(r, 4, 4)).toEqual([255, 0, 0, 255]);
  });

  it("nests", () => {
    const r = render(8, 8, [group({ id: "a" }, [group({ id: "b", passThrough: false }, [group({ id: "c" }, [full("leaf", GREEN)])])])]);
    expect(pixelAt(r, 4, 4)).toEqual([0, 255, 0, 255]);
  });
});

describe("clipping", () => {
  it("shows a clipped layer only inside its base", () => {
    const r = render(8, 8, [full("bg", WHITE), box("base", 2, 2, 4, 4, RED), full("clip", BLUE, { clip: true })]);
    expect(pixelAt(r, 3, 3)).toEqual([0, 0, 255, 255]);
    expect(pixelAt(r, 0, 0)).toEqual([255, 255, 255, 255]);
  });

  it("keeps the base's alpha exactly, even where the base is translucent", () => {
    const r = render(8, 8, [full("base", [1, 0, 0, 0.5]), full("clip", BLUE, { clip: true })]);
    near(pixelAt(r, 4, 4), [0, 0, 255, 128]);
  });

  it("clips several layers to one base, and hiding the base hides them all", () => {
    const layers = (visible: boolean) => [
      full("bg", WHITE),
      box("base", 2, 2, 4, 4, RED, { visible }),
      box("c1", 0, 0, 4, 8, BLUE, { clip: true }),
      box("c2", 0, 0, 8, 3, GREEN, { clip: true }),
      box("next", 6, 6, 2, 2, BLACK),
    ];
    const shown = render(8, 8, layers(true));
    expect(pixelAt(shown, 2, 2)).toEqual([0, 255, 0, 255]);
    expect(pixelAt(shown, 3, 4)).toEqual([0, 0, 255, 255]);
    expect(pixelAt(shown, 5, 5)).toEqual([255, 0, 0, 255]);
    expect(pixelAt(shown, 7, 7)).toEqual([0, 0, 0, 255]); // the next unclipped layer is independent
    const hidden = render(8, 8, layers(false));
    expect(pixelAt(hidden, 3, 4)).toEqual([255, 255, 255, 255]);
    expect(pixelAt(hidden, 7, 7)).toEqual([0, 0, 0, 255]);
  });

  it("blends a clipped layer with its base, not with the backdrop", () => {
    const r = render(8, 8, [
      full("bg", BLUE),
      full("base", [1, 1, 0, 1]),
      full("clip", [0.5, 0.5, 0.5, 1], { clip: true, blend: "multiply", opacity: 1 }),
    ]);
    near(pixelAt(r, 4, 4), [128, 128, 0, 255]);
  });

  it("applies the base's opacity and blend mode to the clipped stack as one", () => {
    const r = render(8, 8, [full("bg", WHITE), full("base", RED, { opacity: 0.5 }), full("clip", BLACK, { clip: true })]);
    near(pixelAt(r, 4, 4), [128, 128, 128, 255]);
  });

  it("clips to a group", () => {
    const r = render(8, 8, [group({ id: "g", passThrough: false }, [box("a", 0, 0, 4, 8, RED)]), full("clip", GREEN, { clip: true })]);
    expect(pixelAt(r, 1, 4)).toEqual([0, 255, 0, 255]);
    expect(pixelAt(r, 6, 4)).toEqual([0, 0, 0, 0]);
  });
});

describe("masks", () => {
  // 4×4 mask: left half white, right half black.
  const half = () => png(ck, 4, 4, (x) => (x < 2 ? [255, 255, 255, 255] : [0, 0, 0, 255]));
  const masked = (mask: Record<string, unknown>) =>
    render(8, 8, [full("a", RED, { mask: { asset: "m", offset: { x: 2, y: 2 }, ...mask } })], { m: half() });

  it("reveals white, hides black, and hides outside the mask image by default", () => {
    const r = masked({});
    expect(pixelAt(r, 3, 3)).toEqual([255, 0, 0, 255]);
    expect(pixelAt(r, 5, 3)).toEqual([0, 0, 0, 0]);
    expect(pixelAt(r, 0, 0)).toEqual([0, 0, 0, 0]);
  });

  it("can reveal outside the mask image", () => {
    const r = masked({ outside: "reveal" });
    expect(pixelAt(r, 0, 0)).toEqual([255, 0, 0, 255]);
    expect(pixelAt(r, 5, 3)).toEqual([0, 0, 0, 0]);
  });

  it("inverts, including the outside", () => {
    const r = masked({ inverted: true });
    expect(pixelAt(r, 3, 3)).toEqual([0, 0, 0, 0]);
    expect(pixelAt(r, 5, 3)).toEqual([255, 0, 0, 255]);
    expect(pixelAt(r, 0, 0)).toEqual([255, 0, 0, 255]);
  });

  it("does nothing when disabled, and fades with density", () => {
    expect(pixelAt(masked({ enabled: false }), 5, 3)).toEqual([255, 0, 0, 255]);
    near(pixelAt(masked({ density: 0.5 }), 5, 3), [255, 0, 0, 128]);
    expect(pixelAt(masked({ density: 0.5 }), 3, 3)).toEqual([255, 0, 0, 255]);
  });

  it("uses gray levels as partial coverage", () => {
    const gray = png(ck, 8, 8, () => [64, 64, 64, 255]);
    const r = render(8, 8, [full("a", RED, { mask: { asset: "m", offset: { x: 0, y: 0 } } })], { m: gray });
    near(pixelAt(r, 4, 4), [255, 0, 0, 64]);
  });

  it("softens the mask edge with feather", () => {
    const r = masked({ feather: 2, outside: "hide" });
    const edge = pixelAt(r, 4, 3)[3]!; // just past the white/black boundary
    expect(edge).toBeGreaterThan(20);
    expect(edge).toBeLessThan(235);
  });

  it("applies vector masks, inverted vector masks and disabled ones", () => {
    const vm = (extra: Record<string, unknown>) =>
      render(8, 8, [full("a", RED, { vectorMask: { path: { d: rectPath(2, 2, 4, 4) }, ...extra } })]);
    expect(pixelAt(vm({}), 3, 3)).toEqual([255, 0, 0, 255]);
    expect(pixelAt(vm({}), 0, 0)).toEqual([0, 0, 0, 0]);
    expect(pixelAt(vm({ inverted: true }), 3, 3)).toEqual([0, 0, 0, 0]);
    expect(pixelAt(vm({ inverted: true }), 0, 0)).toEqual([255, 0, 0, 255]);
    expect(pixelAt(vm({ enabled: false }), 0, 0)).toEqual([255, 0, 0, 255]);
  });

  it("combines a raster mask and a vector mask on one layer", () => {
    const r = render(8, 8, [
      full("a", RED, { mask: { asset: "m", offset: { x: 2, y: 2 } }, vectorMask: { path: { d: rectPath(0, 0, 8, 4) } } }),
    ], { m: half() });
    expect(pixelAt(r, 3, 3)).toEqual([255, 0, 0, 255]);
    expect(pixelAt(r, 3, 5)).toEqual([0, 0, 0, 0]); // raster reveals, vector hides
    expect(pixelAt(r, 5, 3)).toEqual([0, 0, 0, 0]); // vector reveals, raster hides
  });

  it("masks groups, isolated or pass-through", () => {
    for (const passThrough of [true, false]) {
      const r = render(8, 8, [
        full("bg", WHITE),
        group({ id: "g", passThrough, vectorMask: { path: { d: rectPath(0, 0, 4, 8) } } }, [full("a", [0.5, 0.5, 0.5, 1], { blend: "multiply" })]),
      ]);
      near(pixelAt(r, 6, 4), [255, 255, 255, 255]);
      near(pixelAt(r, 1, 4), [128, 128, 128, 255]);
    }
  });

  it("clipped layers respect the base's mask", () => {
    const r = render(8, 8, [
      full("base", RED, { vectorMask: { path: { d: rectPath(0, 0, 4, 8) } } }),
      full("clip", BLUE, { clip: true }),
    ]);
    expect(pixelAt(r, 1, 4)).toEqual([0, 0, 255, 255]);
    expect(pixelAt(r, 6, 4)).toEqual([0, 0, 0, 0]);
  });
});
