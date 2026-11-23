import type { Doc, GroupLayer, Layer } from "@cardsmith/doc";
import type { Layer as PsdLayer, Psd } from "ag-psd";
import { describe, expect, it } from "vitest";
import { describeBindings, generateBindings } from "./bindings.js";
import { convertPsd } from "./convert.js";
import { convertEffects } from "./effects.js";
import { convertVectorPath } from "./paths.js";

/** A square subpath with corner knots (handles on the anchors), wound clockwise unless `ccw`. */
function square(x: number, y: number, size: number, operation: "combine" | "exclude" | "subtract" | "intersect", ccw = false) {
  const corners = [[x, y], [x + size, y], [x + size, y + size], [x, y + size]];
  if (ccw) corners.reverse();
  return { open: false, operation, fillRule: "even-odd" as const, knots: corners.map(([px, py]) => ({ linked: true, points: [px!, py!, px!, py!, px!, py!] })) };
}

function run(psd: Partial<Psd>) {
  const emitted = new Map<string, Uint8Array>();
  const result = convertPsd({ width: 100, height: 140, ...psd } as Psd, { emit: (path, png) => emitted.set(path, png) });
  return { ...result, emitted };
}
const only = (doc: Doc) => doc.root.children[0]!;
const rgba = (w: number, h: number, value: number[]) => ({ width: w, height: h, data: new Uint8ClampedArray(Array.from({ length: w * h }, () => value).flat()) });

describe("convertVectorPath", () => {
  const warn = () => {};

  it("writes cubic segments through the knots and closes the path", () => {
    const path = convertVectorPath({ paths: [{ open: false, operation: "combine", fillRule: "even-odd", knots: [
      { linked: false, points: [0, 5, 0, 0, 5, 0] },
      { linked: false, points: [15, 0, 20, 0, 20, 5] },
      { linked: false, points: [20, 15, 20, 20, 15, 20] },
    ] }] }, warn)!;
    // From each anchor: its outgoing handle, the next knot's incoming handle, the next anchor.
    expect(path.d).toBe("M0 0C5 0 15 0 20 0C20 5 20 15 20 20C15 20 0 5 0 0Z");
    expect(path).toMatchObject({ fillRule: "nonzero", inverted: false });
  });

  it("unions combined subpaths by winding them the same way", () => {
    const path = convertVectorPath({ paths: [square(0, 0, 10, "combine"), square(5, 5, 10, "combine", true)] }, warn)!;
    expect(path.fillRule).toBe("nonzero");
    // The counter-clockwise square was reversed: both now start at their top-left corner going right.
    expect(path.d.split("Z").filter(Boolean)).toEqual(["M0 0C0 0 10 0 10 0C10 0 10 10 10 10C10 10 0 10 0 10C0 10 0 0 0 0", "M5 5C5 5 15 5 15 5C15 5 15 15 15 15C15 15 5 15 5 15C5 15 5 5 5 5"]);
  });

  it("treats subtract-only paths as everything except their union", () => {
    expect(convertVectorPath({ paths: [square(0, 0, 10, "subtract")] }, warn)).toMatchObject({ inverted: true, fillRule: "nonzero" });
    expect(convertVectorPath({ paths: [square(0, 0, 10, "subtract"), square(20, 0, 10, "subtract")] }, warn)!.inverted).toBe(true);
  });

  it("uses even-odd for exclude, and inverts when the canvas starts filled", () => {
    expect(convertVectorPath({ paths: [square(0, 0, 10, "exclude")] }, warn)).toMatchObject({ fillRule: "evenodd", inverted: false });
    expect(convertVectorPath({ paths: [square(0, 0, 10, "exclude")], fillStartsWithAllPixels: true }, warn)!.inverted).toBe(true);
  });

  it("returns null for 'everything' and an empty path for 'nothing'", () => {
    expect(convertVectorPath({ paths: [square(0, 0, 10, "combine")], fillStartsWithAllPixels: true }, warn)).toBeNull();
    expect(convertVectorPath({ paths: [], fillStartsWithAllPixels: true }, warn)).toBeNull();
    expect(convertVectorPath({ paths: [] }, warn)).toEqual({ d: "", fillRule: "nonzero", inverted: false });
  });

  it("warns when operations are mixed", () => {
    const warnings: string[] = [];
    const path = convertVectorPath({ paths: [square(0, 0, 20, "combine"), square(5, 5, 5, "subtract")] }, (m) => warnings.push(m))!;
    expect(warnings.join()).toMatch(/mixes operations/);
    // The subtracted square winds against the added one, so non-zero filling cuts a hole.
    expect(path.d).toContain("M5 10C5 10 10 10 10 10");
  });
});

describe("convertEffects", () => {
  const global = { angle: 102, altitude: 21 };
  const convert = (fx: object) => {
    const warnings: string[] = [];
    return { ...convertEffects(fx as never, global, "/Layer", (m) => warnings.push(m)), warnings };
  };
  const px = (value: number) => ({ value, units: "Pixels" });

  it("keeps present effects, drops absent ones, and records the master switch", () => {
    const { effects, enabled } = convert({
      disabled: true,
      dropShadow: [{ present: false, enabled: false }, { present: true, enabled: true, blendMode: "multiply", color: { r: 0, g: 0, b: 0 }, opacity: 0.5, angle: 135, distance: px(8), choke: px(25), size: px(2), contour: { name: "Gaussian" } }],
      satin: { present: false },
    });
    expect(enabled).toBe(false);
    expect(effects).toEqual([{ type: "dropShadow", enabled: true, blend: "multiply", opacity: 0.5, color: [0, 0, 0, 1], angle: 135, distance: 8, spread: 0.25, size: 2, contour: "gaussian" }]);
  });

  it("uses the document's global light where an effect asks for it", () => {
    const { effects } = convert({
      innerShadow: [{ present: true, useGlobalLight: true, angle: 30, distance: px(3), choke: px(0), size: px(7) }],
      bevel: { present: true, style: "inner bevel", technique: "chisel hard", direction: "down", strength: 1, size: px(16), soften: px(0), useGlobalLight: true, angle: 45, altitude: 23,
        highlightBlendMode: "screen", highlightColor: { r: 255, g: 255, b: 255 }, highlightOpacity: 0.85, shadowBlendMode: "multiply", shadowColor: { r: 0, g: 0, b: 0 }, shadowOpacity: 0.85 },
    });
    expect(effects[0]).toMatchObject({ type: "innerShadow", angle: 102 });
    expect(effects[1]).toMatchObject({ type: "bevel", style: "inner", direction: "down", angle: 102, altitude: 21, size: 16, highlight: { blend: "screen", opacity: 0.85 } });
  });

  it("merges gradient color and opacity stops, and keeps offset, scale and reverse", () => {
    const { effects } = convert({ gradientOverlay: [{ present: true, blendMode: "luminosity", opacity: 0.2, type: "linear", angle: -20, scale: 0.5, reverse: true, offset: { x: -0.1, y: 0 },
      gradient: { type: "solid", colorStops: [{ color: { r: 255, g: 0, b: 0 }, location: 0, midpoint: 0.5 }, { color: { r: 0, g: 0, b: 255 }, location: 1, midpoint: 0.5 }],
        opacityStops: [{ opacity: 1, location: 0, midpoint: 0.5 }, { opacity: 0, location: 0.5, midpoint: 0.5 }, { opacity: 0, location: 1, midpoint: 0.5 }] } }] });
    expect(effects[0]).toMatchObject({ type: "gradientOverlay", blend: "luminosity", angle: -20, scale: 0.5, reverse: true, offset: { x: -0.1, y: 0 } });
    const stops = (effects[0] as { gradient: { stops: { offset: number; color: number[] }[] } }).gradient.stops;
    expect(stops.map((s) => s.offset)).toEqual([0, 0.5, 1]);
    expect(stops[1]!.color).toEqual([0.5, 0, 0.5, 0]); // halfway between the colors, at the opacity stop
    expect(stops[2]!.color).toEqual([0, 0, 1, 0]);
  });

  it("places an extra stop where a midpoint is off-center", () => {
    const { effects } = convert({ gradientOverlay: [{ present: true, type: "radial", gradient: { type: "solid",
      colorStops: [{ color: { r: 0, g: 0, b: 0 }, location: 0, midpoint: 0.5 }, { color: { r: 255, g: 255, b: 255 }, location: 1, midpoint: 0.25 }], opacityStops: [] } }] });
    const stops = (effects[0] as { gradient: { stops: { offset: number; color: number[] }[] } }).gradient.stops;
    expect(stops.map((s) => s.offset)).toEqual([0, 0.25, 1]);
    expect(stops[1]!.color[0]).toBeCloseTo(0.5); // half-way color reached a quarter of the way along
  });

  it("maps stroke, color overlay and outer glow, and warns about what it cannot represent", () => {
    const { effects, warnings } = convert({
      stroke: [{ present: true, position: "inside", fillType: "gradient", size: px(7), color: { r: 0, g: 0, b: 0 }, blendMode: "color dodge" }],
      solidFill: [{ present: true, blendMode: "normal", color: { r: 255, g: 255, b: 255 }, opacity: 1 }],
      outerGlow: { present: true, blendMode: "multiply", color: { r: 0, g: 0, b: 0 }, opacity: 0.7, choke: px(40), size: px(36) },
      innerGlow: { present: true },
    });
    expect(effects.map((e) => e.type)).toEqual(["outerGlow", "colorOverlay", "stroke"]);
    expect(effects[0]).toMatchObject({ spread: 0.4, size: 36 });
    expect(effects[2]).toMatchObject({ position: "inside", size: 7, blend: "normal" });
    expect(warnings).toEqual([
      "/Layer stroke: gradient strokes are not supported; using its color",
      '/Layer stroke: blend mode "color dodge" is not supported; using normal',
      "/Layer: inner glow is not supported; skipped",
    ]);
  });
});

describe("convertPsd", () => {
  it("converts document settings and keeps the paint order", () => {
    const { doc, warnings } = run({
      imageResources: { resolutionInfo: { horizontalResolution: 1200, horizontalResolutionUnit: "PPI", widthUnit: "Inches", verticalResolution: 1200, verticalResolutionUnit: "PPI", heightUnit: "Inches" } },
      children: [{ name: "bottom", id: 1 }, { name: "top", id: 2 }],
    });
    expect(doc).toMatchObject({ version: 1, width: 100, height: 140, dpi: 1200 });
    expect(doc.root.children.map((l) => [l.id, l.name])).toEqual([["l1", "bottom"], ["l2", "top"]]);
    expect(warnings).toEqual([]);
  });

  it("writes pixel layers as images placed at their bounds", () => {
    const { doc, emitted } = run({ children: [{ name: "Art", id: 7, left: 10, top: 20, right: 12, bottom: 21, opacity: 0.5, hidden: true, clipping: true, blendMode: "soft light", fillOpacity: 0.25, imageData: rgba(2, 1, [255, 0, 0, 255]) as never }] });
    expect(only(doc)).toEqual({ kind: "pixel", id: "l7", name: "Art", visible: false, opacity: 0.5, blend: "softLight", clip: true, fillOpacity: 0.25, asset: "l7", transform: [1, 0, 10, 0, 1, 20] });
    expect(doc.assets.l7).toEqual({ kind: "image", path: "assets/l7.png", width: 2, height: 1 });
    expect([...emitted.get("assets/l7.png")!.subarray(1, 4)]).toEqual([0x50, 0x4e, 0x47]);
  });

  it("gives layers without pixels a shared empty image, keeping their masks", () => {
    const mask = { left: 5, top: 6, right: 7, bottom: 7, defaultColor: 255, disabled: true, imageData: rgba(2, 1, [128, 128, 128, 255]) };
    const { doc, emitted } = run({ children: [{ name: "Textbox Reference", id: 1, mask: mask as never }, { name: "Layer 1", id: 2 }] });
    expect(doc.root.children.map((l) => (l as { asset: string }).asset)).toEqual(["empty", "empty"]);
    expect(only(doc).mask).toEqual({ asset: "l1.mask", offset: { x: 5, y: 6 }, outside: "reveal", enabled: false });
    expect([...emitted.keys()].sort()).toEqual(["assets/empty.png", "assets/l1.mask.png"]);
  });

  it("skips the raster copy Photoshop keeps of a vector mask", () => {
    const { doc, emitted } = run({ children: [{ name: "g", id: 1, children: [], blendMode: "pass through", mask: { fromVectorData: true, left: 0, top: 0, imageData: rgba(1, 1, [0, 0, 0, 255]) } as never,
      vectorMask: { paths: [square(0, 0, 10, "combine")], invert: true, disable: true } }] });
    const group = only(doc) as GroupLayer;
    expect(group.mask).toBeUndefined();
    expect(group.vectorMask).toMatchObject({ inverted: true, enabled: false });
    expect(group).toMatchObject({ kind: "group", passThrough: true, blend: "normal", children: [] });
    expect(emitted.size).toBe(0);
  });

  it("turns shape layers into paths, with the canvas as the default outline", () => {
    const black = { type: "color", color: { r: 0, g: 0, b: 0 } } as const;
    const { doc } = run({ children: [
      { name: "plain", id: 1, vectorFill: { type: "color", color: { r: 255, g: 128, b: 0 } }, vectorMask: { paths: [square(10, 10, 20, "combine")] } },
      { name: "whole canvas", id: 2, vectorFill: black },
      { name: "cut out", id: 3, vectorFill: black, vectorMask: { paths: [square(10, 10, 20, "subtract")] } },
      { name: "stroked", id: 4, vectorFill: black, vectorMask: { paths: [square(10, 10, 20, "combine")] },
        vectorStroke: { strokeEnabled: true, fillEnabled: false, lineWidth: { value: 3, units: "Pixels" }, lineAlignment: "inside", opacity: 0.5, content: { type: "color", color: { r: 255, g: 255, b: 255 } } } },
    ] });
    const [plain, whole, cut, stroked] = doc.root.children as Extract<Layer, { kind: "shape" }>[];
    expect(plain).toMatchObject({ kind: "shape", fill: { type: "solid", color: [1, 128 / 255, 0, 1] } });
    expect(plain!.path.d).toMatch(/^M10 10C/);
    expect(plain!.vectorMask).toBeUndefined(); // the path is the shape itself, not a mask on it
    expect(whole!.path).toEqual({ d: "M0 0H100V140H0Z" });
    expect(cut!.path.fillRule).toBe("evenodd");
    expect(cut!.path.d.startsWith("M0 0H100V140H0ZM10 10C")).toBe(true);
    expect(stroked).toMatchObject({ fill: { color: [0, 0, 0, 0] }, stroke: { color: [1, 1, 1, 0.5], width: 3, align: "inside" } });
  });

  it("converts text settings without rendering the text", () => {
    const { doc } = run({ children: [{ name: "Rules", id: 1, left: 40, top: 30, right: 90, bottom: 50, text: {
      text: "Flying\r(It flies.)", shapeType: "box", boxBounds: [0, 0, 60, 40], transform: [1, 0, 0, 2, 40, 30],
      style: { font: { name: "PlantinMTPro-Regular" }, fontSize: 9, autoLeading: false, leading: 10.8, tracking: -5, fillColor: { r: 255, g: 255, b: 255 } },
      styleRuns: [{ length: 7, style: {} }, { length: 11, style: { font: { name: "PlantinMTPro-Italic" } } }],
      paragraphStyle: { justification: "center" },
    } }] });
    expect(only(doc)).toMatchObject({
      kind: "text", box: { x: 40, y: 30, w: 60, h: 80 }, fit: "none",
      content: { runs: [{ text: "Flying\n" }, { text: "(It flies.)", font: "PlantinMTPro-Italic", italic: true }] },
      style: { font: "PlantinMTPro-Regular", size: 18, color: [1, 1, 1, 1], tracking: -5, align: "center" },
    });
    expect((only(doc) as { style: { leading: number } }).style.leading).toBeCloseTo(1.2);
  });

  it("positions point text by the layer's bounds", () => {
    const { doc } = run({ children: [{ name: "Name", id: 1, left: 10, top: 20, right: 60, bottom: 32, text: { text: "Card Name", shapeType: "point", transform: [1.5, 0, 0, 1.5, 12, 30], style: { font: { name: "Beleren" }, fontSize: 8 } } }] });
    expect(only(doc)).toMatchObject({ box: { x: 10, y: 20, w: 50, h: 12 }, style: { size: 12, align: "left" } });
  });

  it("converts effects and reports what it cannot import, with the layer's path", () => {
    const { doc, warnings } = run({ children: [{ name: "Frame", id: 1, blendMode: "pass through", children: [
      { name: "Odd", id: 2, blendMode: "color dodge", knockout: true, adjustment: { type: "levels" } as never,
        effects: { stroke: [{ present: true, enabled: true, position: "outside", size: { value: 7, units: "Pixels" }, color: { r: 0, g: 0, b: 0 }, blendMode: "normal", opacity: 1 }] } },
    ] }] });
    const odd = (only(doc) as GroupLayer).children[0]!;
    expect(odd.blend).toBe("normal");
    expect(odd.effects).toEqual([{ type: "stroke", enabled: true, blend: "normal", opacity: 1, color: [0, 0, 0, 1], size: 7, position: "outside" }]);
    expect(warnings).toEqual([
      '/Frame/Odd: blend mode "color dodge" is not supported; using normal',
      "/Frame/Odd: adjustment layer (levels) is not supported; it has no effect",
      "/Frame/Odd: knockout is not supported",
    ]);
  });

  it("keeps ids unique when a PSD repeats or omits them", () => {
    const { doc } = run({ children: [{ name: "a", id: 5 }, { name: "b", id: 5 }, { name: "c" }] as PsdLayer[] });
    expect(new Set(doc.root.children.map((l) => l.id)).size).toBe(3);
  });
});

describe("generateBindings", () => {
  const base = { visible: true, opacity: 1, blend: "normal" as const, clip: false };
  const leaf = (name: string): Layer => ({ ...base, kind: "shape", id: `id:${name}:${Math.random().toString(36).slice(2, 7)}`, name, path: { d: "" }, fill: { type: "solid", color: [0, 0, 0, 1] } });
  const group = (name: string, children: Layer[]): Layer => ({ ...base, kind: "group", id: `g:${name}`, name, passThrough: true, children });
  const docOf = (children: Layer[]): Doc => ({ version: 1, width: 10, height: 10, dpi: 72, assets: {}, root: { ...(group("Root", children) as GroupLayer), id: "root" } });
  const colors = (names: string[]) => names.map(leaf);

  const raster = () =>
    docOf([
      leaf("Layer 1"), leaf("Full Art Frame"), leaf("Art Frame"),
      group("Background", colors(["W", "U", "Gold"])), group("Land Pinlines & Textbox", colors(["W", "Land"])), group("Pinlines & Textbox", colors(["W", "WU"])),
      leaf("Hollow Crown Shadow"), group("Legendary Crown", colors(["W"])), group("Name & Title Boxes", colors(["W"])), group("PT Box", colors(["W", "Vehicle"])), group("Color Indicator", colors(["W"])),
      group("Text and Icons", ["Divider", "Textbox Reference", "PT Reference", "Expansion Reference", "Rules Text - Creature", "Rules Text - Noncreature", "Typeline Shift", "Typeline", "Power / Toughness", "Mana Cost", "Card Name"].map(leaf)),
      group("Legal", ["Collector", "Set", "Artist"].map(leaf)),
    ]);

  it("binds a raster template by Proxyshop's layer names", () => {
    const doc = raster();
    const b = generateBindings(doc);
    expect(b.kind).toBe("raster");
    expect(b.unresolved).toEqual([]);
    expect(b.art).toBe(doc.root.children[0]!.id);
    expect(Object.keys(b.artRefs).sort()).toEqual(["fullArt", "normal"]);
    expect(Object.keys(b.text).sort()).toEqual(["artist", "collector", "mana", "name", "pt", "rulesCreature", "rulesNoncreature", "set", "type", "typeShift"]);
    expect(Object.keys(b.textRefs).sort()).toEqual(["expansion", "pt", "textbox"]);
    expect(Object.keys(b.toggles.pinlines!)).toEqual(["W", "WU"]);
    expect(Object.keys(b.toggles.ptBox!)).toEqual(["W", "Vehicle"]);
    expect(b.toggles.background!.Gold).toBe((doc.root.children[3] as GroupLayer).children[2]!.id);
    expect(b.layers).toHaveProperty("crownShadow");
    expect(describeBindings(b)).toMatch(/nothing unresolved/);
  });

  it("lists what a template is missing instead of guessing", () => {
    const doc = raster();
    doc.root.children = doc.root.children.filter((l) => !["Layer 1", "Legendary Crown", "Legal"].includes(l.name));
    const b = generateBindings(doc);
    expect(b.unresolved.sort()).toEqual(["art", "groups.legal", "text.artist", "text.set", "toggles.crown"]);
    expect(describeBindings(b)).toMatch(/UNRESOLVED.*toggles\.crown/);
  });

  it("recognizes a vector template and binds its sized rules text and shape groups", () => {
    const sized = (size: string) => group(size, [leaf("Rules Text - Creature"), leaf("Rules Text - Noncreature")]);
    const b = generateBindings(docOf([
      leaf("Layer 1"), leaf("Full Art Frame"), leaf("Art Frame Normal"), leaf("Art Frame Tall"),
      group("Textbox", [group("Shape", [])]), group("Pinlines", [group("Shape", [])]), group("Name & Title Boxes", [group("Shape", [])]), group("PT Box", [group("Shape", [])]),
      group("Text and Icons", [...["Divider", "PT Reference", "Expansion Reference", "Textbox Reference", "Typeline", "Power / Toughness", "Mana Cost", "Card Name"].map(leaf), sized("Normal"), sized("Tall")]),
      group("Legal", ["Set", "Artist"].map(leaf)),
    ]));
    expect(b.kind).toBe("vector");
    expect(b.unresolved).toEqual([]);
    expect(Object.keys(b.artRefs).sort()).toEqual(["fullArt", "normal", "tall"]);
    expect(Object.keys(b.text)).toEqual(expect.arrayContaining(["rulesCreature.normal", "rulesNoncreature.tall"]));
    expect(Object.keys(b.groups)).toEqual(expect.arrayContaining(["pinlinesShapes", "textboxShapes", "twinsShapes", "ptBoxShapes"]));
  });
});
