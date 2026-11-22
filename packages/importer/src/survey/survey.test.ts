import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { writePsdUint8Array, type Layer, type Psd } from "ag-psd";
import { describe, expect, it } from "vitest";
import { parseLayerNameEnums } from "./layerNames.js";
import { renderMarkdown } from "./markdown.js";
import { readPsdStructure, surveyDirectory } from "./run.js";
import { emptySurvey, finishSurvey, layerKind, surveyPsd } from "./survey.js";

const LAYERS_PY = `
class LAYERS (StrConstant):
    """Layer name definitions."""
    # Colors
    WHITE = 'W'
    PINLINES_TEXTBOX = 'Pinlines & Textbox'
    TEXT_AND_ICONS = "Text and Icons"  # trailing comment
    UNUSED = 'Nobody Uses This'

    @classmethod
    def something(cls): x = 'not a constant'

class OTHER:
    ALIAS = 'W'
`;

const pixel = (name: string, extra: Partial<Layer> = {}): Layer => ({
  name, top: 0, left: 0, bottom: 10, right: 10, blendMode: "normal", ...extra,
});

/** A hand-built PSD tree exercising every category the survey counts. */
const PSD: Psd = {
  width: 100,
  height: 140,
  bitsPerChannel: 8,
  colorMode: 3,
  imageResources: {
    resolutionInfo: {
      horizontalResolution: 800, horizontalResolutionUnit: "PPI", widthUnit: "Inches",
      verticalResolution: 800, verticalResolutionUnit: "PPI", heightUnit: "Inches",
    },
  },
  linkedFiles: [
    { id: "emb", name: "crown.psb", data: new Uint8Array(4) },
    { id: "ext", name: "art.png" },
  ],
  children: [
    {
      name: "Text and Icons",
      blendMode: "pass through",
      children: [
        {
          name: "Rules",
          blendMode: "normal",
          text: {
            text: "Flying (reminder)",
            shapeType: "box",
            antiAlias: "sharp",
            style: { font: { name: "PlantinMTPro-Regular" }, fontSize: 9, autoLeading: false, leading: 10, tracking: -5 },
            styleRuns: [
              { length: 7, style: { font: { name: "PlantinMTPro-Regular" } } },
              { length: 10, style: { font: { name: "PlantinMTPro-Italic" }, fauxItalic: true } },
            ],
            paragraphStyle: { justification: "left", spaceAfter: 4 },
          },
        },
        { name: "Name", blendMode: "normal", text: { text: "X", warp: { style: "arc" } as never } },
      ],
    },
    {
      name: "Pinlines & Textbox",
      blendMode: "normal",
      mask: { defaultColor: 0 },
      children: [
        pixel("W", {
          blendMode: "multiply",
          opacity: 0.5,
          clipping: true,
          mask: { defaultColor: 255, userMaskFeather: 2 },
          effects: {
            dropShadow: [{ enabled: true, blendMode: "multiply" }, { enabled: false }],
            stroke: [{ enabled: true, position: "outside", fillType: "color", blendMode: "normal" }],
            bevel: { enabled: true, style: "inner bevel", technique: "smooth", highlightBlendMode: "screen", shadowBlendMode: "multiply" },
            outerGlow: { enabled: false },
          },
        }),
        pixel("Custom Thing", { hidden: true, right: 0 }),
      ],
    },
    { name: "Levels 1", adjustment: { type: "levels" } as never },
    { name: "Shape", vectorFill: { type: "color", color: { r: 0, g: 0, b: 0 } }, vectorMask: { paths: [] }, vectorStroke: { strokeEnabled: true, fillEnabled: true } },
    { name: "Crown", placedLayer: { id: "emb", type: "raster", transform: [0, 0, 1, 0, 1, 1, 0, 1] } },
    { name: "Art", placedLayer: { id: "ext", type: "raster", external: true, transform: [0, 0, 1, 0, 1, 1, 0, 1] } },
  ],
};

describe("parseLayerNameEnums", () => {
  it("collects quoted constants per class and ignores everything else", () => {
    const names = parseLayerNameEnums(LAYERS_PY);
    expect([...names.keys()].sort()).toEqual(["Nobody Uses This", "Pinlines & Textbox", "Text and Icons", "W"]);
    expect(names.get("W")).toEqual(["LAYERS.WHITE", "OTHER.ALIAS"]);
  });
});

describe("surveyPsd", () => {
  const survey = emptySurvey();
  const names = parseLayerNameEnums(LAYERS_PY);
  surveyPsd(PSD, "test.psd", survey, names);
  finishSurvey(survey, names);

  it("summarizes the file", () => {
    expect(survey.files).toEqual([
      { file: "test.psd", width: 100, height: 140, dpi: 800, colorMode: "RGB", bitsPerChannel: 8, layers: 10, maxDepth: 2 },
    ]);
    expect(survey.totalLayers).toBe(10);
  });

  it("counts layer kinds and blend modes", () => {
    expect(survey.layerKinds).toEqual({
      group: 2, text: 2, pixel: 1, "pixel (empty)": 1, adjustment: 1, "shape (color fill)": 1, "smart object": 2,
    });
    expect(survey.blendModes).toEqual({ "pass through (group)": 1, "normal (group)": 1, normal: 7, multiply: 1 });
  });

  it("separates enabled effects from disabled ones and records their options", () => {
    expect(survey.effects).toEqual({ dropShadow: 1, stroke: 1, bevel: 1 });
    expect(survey.effectsDisabled).toEqual({ "dropShadow (effect off)": 1, "outerGlow (effect off)": 1 });
    expect(survey.effectBlendModes).toEqual({
      "dropShadow: multiply": 1, "stroke: normal": 1, "bevel: screen": 1, "bevel: multiply": 1,
    });
    expect(survey.effectOptions).toMatchObject({
      "bevel style: inner bevel": 1, "bevel technique: smooth": 1, "stroke position: outside": 1, "stroke fill: color": 1,
    });
  });

  it("treats effects.disabled as switching every effect off", () => {
    const s = emptySurvey();
    surveyPsd({ width: 1, height: 1, children: [pixel("a", { effects: { disabled: true, satin: { enabled: true } } })] }, "x", s);
    expect(s.effects).toEqual({});
    expect(s.effectsDisabled).toEqual({ "satin (layer style off)": 1 });
  });

  it("counts adjustments and smart objects (embedded vs linked)", () => {
    expect(survey.adjustments).toEqual({ levels: 1 });
    expect(survey.smartObjects).toEqual({ "embedded (raster, .psb)": 1, "linked (raster, .png)": 1 });
  });

  it("records text fonts and features", () => {
    expect(survey.text.layers).toBe(2);
    expect(survey.text.fonts).toEqual({ "PlantinMTPro-Regular": 1, "PlantinMTPro-Italic": 1 });
    expect(survey.text.features).toMatchObject({
      "box text": 1, "point text": 1, "fixed leading": 1, "tracking != 0": 1, "faux italic": 1,
      "multiple style runs": 1, "multiple fonts in one layer": 1, "paragraph spacing": 1,
    });
    expect(survey.text.justification).toEqual({ left: 1 });
    expect(survey.text.warp).toEqual({ none: 1, arc: 1 });
    expect(survey.text.antiAlias).toEqual({ sharp: 1 });
  });

  it("records masks, clipping and layer properties", () => {
    expect(survey.masks).toMatchObject({
      "raster mask on group": 1, "raster mask on layer": 1, "raster mask default white (reveal outside)": 1,
      "raster mask feather": 1, "vector mask on layer": 1, "clipping layer": 1, "clipping layer: pixel": 1,
    });
    expect(survey.layerProps).toEqual({ hidden: 1, "opacity < 100%": 1, "shape stroke": 1 });
  });

  it("matches layer names against the known constants", () => {
    expect(survey.layerNames.checked).toBe(true);
    expect(survey.layerNames.matched).toEqual({ "Text and Icons": 1, "Pinlines & Textbox": 1, W: 1 });
    expect(survey.layerNames.unmatched).toMatchObject({ Rules: 1, "Custom Thing": 1 });
    expect(survey.layerNames.unused).toEqual(["Nobody Uses This"]);
  });

  it("renders every section to Markdown", () => {
    const md = renderMarkdown(survey);
    for (const heading of ["## Files", "## Layer kinds", "## Blend modes", "## Layer effects", "## Adjustment layers",
      "## Smart objects", "## Text", "## Masks and clipping", "## Layer names"]) {
      expect(md).toContain(heading);
    }
    expect(md).toContain("| PlantinMTPro-Italic | 1 |");
    expect(md).toContain("| Pinlines & Textbox | 1 |");
    expect(md).toContain("`Nobody Uses This`");
  });
});

describe("layerKind", () => {
  it("prefers the most specific kind", () => {
    expect(layerKind({ children: [] })).toBe("group");
    expect(layerKind({ text: { text: "" }, placedLayer: undefined })).toBe("text");
    expect(layerKind({})).toBe("pixel (empty)");
  });
});

describe("surveyDirectory", () => {
  it("reads real PSD files from disk without decoding pixels", async () => {
    const dir = await mkdtemp(join(tmpdir(), "cardsmith-survey-"));
    const psd: Psd = {
      width: 4,
      height: 4,
      children: [
        { name: "Group", children: [{ name: "W", blendMode: "screen", opacity: 0.4, clipping: true }] },
        { name: "Hidden", hidden: true, blendMode: "overlay" },
      ],
    };
    await writeFile(join(dir, "tiny.psd"), writePsdUint8Array(psd));
    await writeFile(join(dir, "broken.psd"), "not a psd");
    await writeFile(join(dir, "ignored.txt"), "x");

    const survey = await surveyDirectory(dir);
    expect(survey.files.map((f) => f.file)).toEqual(["tiny.psd"]);
    expect(survey.failed.map((f) => f.file)).toEqual(["broken.psd"]);
    expect(survey.totalLayers).toBe(3);
    expect(survey.blendModes).toMatchObject({ screen: 1, overlay: 1, "pass through (group)": 1 });
    expect(survey.layerProps).toMatchObject({ hidden: 1, "opacity < 100%": 1 });
    expect(survey.masks).toMatchObject({ "clipping layer": 1 });
    expect(survey.layerNames.checked).toBe(false);

    // And the reader really skipped pixel data.
    const parsed = readPsdStructure(writePsdUint8Array(psd));
    expect(parsed.canvas).toBeUndefined();
    expect(parsed.imageData).toBeUndefined();
  });
});
