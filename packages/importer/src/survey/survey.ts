/**
 * PSD feature survey (Phase 0, task 3).
 *
 * Walks parsed PSDs and counts every feature the renderer and importer would
 * have to support. The resulting list is the renderer's whole scope: anything
 * not seen here does not get built.
 */
import type { Layer, LayerEffectsInfo, LayerMaskData, Psd, TextStyle } from "ag-psd";
import type { KnownLayerNames } from "./layerNames.js";

/** A counter: key -> number of occurrences. */
export type Counts = Record<string, number>;

export type FileSummary = {
  file: string;
  width: number;
  height: number;
  dpi?: number;
  colorMode: string;
  bitsPerChannel?: number;
  layers: number;
  maxDepth: number;
};

export type Survey = {
  files: FileSummary[];
  /** Files that could not be parsed, with the error message. */
  failed: { file: string; error: string }[];
  totalLayers: number;
  layerKinds: Counts;
  blendModes: Counts;
  /** Layer effects that are present AND enabled. */
  effects: Counts;
  /** Layer effects that exist on a layer but are switched off. */
  effectsDisabled: Counts;
  /** Blend modes used inside effects, keyed "effect: mode". */
  effectBlendModes: Counts;
  /** Non-default effect options that change how an effect renders. */
  effectOptions: Counts;
  adjustments: Counts;
  smartObjects: Counts;
  text: {
    layers: number;
    fonts: Counts;
    features: Counts;
    justification: Counts;
    antiAlias: Counts;
    warp: Counts;
  };
  masks: Counts;
  /** Other per-layer properties that affect compositing. */
  layerProps: Counts;
  layerNames: {
    /** False when no layers.py was supplied; the lists below are then empty. */
    checked: boolean;
    matched: Counts;
    unmatched: Counts;
    /** Known names that no surveyed PSD uses. */
    unused: string[];
  };
};

export function emptySurvey(): Survey {
  return {
    files: [],
    failed: [],
    totalLayers: 0,
    layerKinds: {},
    blendModes: {},
    effects: {},
    effectsDisabled: {},
    effectBlendModes: {},
    effectOptions: {},
    adjustments: {},
    smartObjects: {},
    text: { layers: 0, fonts: {}, features: {}, justification: {}, antiAlias: {}, warp: {} },
    masks: {},
    layerProps: {},
    layerNames: { checked: false, matched: {}, unmatched: {}, unused: [] },
  };
}

function bump(counts: Counts, key: string, by = 1): void {
  counts[key] = (counts[key] ?? 0) + by;
}

const COLOR_MODES = ["Bitmap", "Grayscale", "Indexed", "RGB", "CMYK", "", "", "Multichannel", "Duotone", "Lab"];

export function layerKind(layer: Layer): string {
  if (layer.children) return layer.sectionDivider?.key === "artb" ? "artboard" : "group";
  if (layer.text) return "text";
  if (layer.placedLayer) return "smart object";
  if (layer.adjustment) return "adjustment";
  if (layer.vectorFill) return `shape (${layer.vectorFill.type} fill)`;
  if (layer.vectorStroke || layer.vectorOrigination) return "shape";
  const empty = (layer.right ?? 0) - (layer.left ?? 0) <= 0 || (layer.bottom ?? 0) - (layer.top ?? 0) <= 0;
  return empty ? "pixel (empty)" : "pixel";
}

const EFFECT_KEYS = [
  "dropShadow",
  "innerShadow",
  "outerGlow",
  "innerGlow",
  "bevel",
  "solidFill",
  "satin",
  "stroke",
  "gradientOverlay",
  "patternOverlay",
] as const;

type AnyEffect = {
  enabled?: boolean;
  present?: boolean;
  blendMode?: string;
  highlightBlendMode?: string;
  shadowBlendMode?: string;
  style?: string;
  technique?: string;
  position?: string;
  fillType?: string;
  source?: string;
  gradient?: { type?: string; style?: string };
  type?: string;
  contour?: { name?: string };
  noise?: number;
  choke?: unknown;
  overprint?: boolean;
  dither?: boolean;
  reverse?: boolean;
  align?: boolean;
};

function surveyEffects(effects: LayerEffectsInfo, out: Survey): void {
  const allOff = effects.disabled === true;
  if (effects.scale !== undefined && effects.scale !== 1) bump(out.effectOptions, "effects scale != 100%");
  for (const key of EFFECT_KEYS) {
    const raw = effects[key] as AnyEffect | AnyEffect[] | undefined;
    if (!raw) continue;
    const list = Array.isArray(raw) ? raw : [raw];
    let enabledOnLayer = 0;
    for (const fx of list) {
      if (fx.present === false) continue;
      if (allOff || fx.enabled === false) {
        bump(out.effectsDisabled, key);
        continue;
      }
      enabledOnLayer++;
      bump(out.effects, key);
      for (const mode of [fx.blendMode, fx.highlightBlendMode, fx.shadowBlendMode]) {
        if (mode) bump(out.effectBlendModes, `${key}: ${mode}`);
      }
      if (key === "bevel") {
        if (fx.style) bump(out.effectOptions, `bevel style: ${fx.style}`);
        if (fx.technique) bump(out.effectOptions, `bevel technique: ${fx.technique}`);
      }
      if (key === "stroke") {
        if (fx.position) bump(out.effectOptions, `stroke position: ${fx.position}`);
        if (fx.fillType) bump(out.effectOptions, `stroke fill: ${fx.fillType}`);
      }
      if (key === "innerGlow" && fx.source) bump(out.effectOptions, `innerGlow source: ${fx.source}`);
      if (key === "gradientOverlay") {
        if (fx.type) bump(out.effectOptions, `gradientOverlay type: ${fx.type}`);
        if (fx.gradient?.type) bump(out.effectOptions, `gradientOverlay gradient: ${fx.gradient.type}`);
      }
      if (fx.contour?.name && fx.contour.name !== "Linear") {
        bump(out.effectOptions, `${key} contour: ${fx.contour.name}`);
      }
      if (fx.noise) bump(out.effectOptions, `${key} noise > 0`);
    }
    if (enabledOnLayer > 1) bump(out.effectOptions, `multiple ${key} on one layer`);
  }
}

function surveyStyle(style: TextStyle, out: Survey["text"], seenFonts: Set<string>): void {
  if (style.font?.name) seenFonts.add(style.font.name);
  const f = out.features;
  const flag = (cond: unknown, name: string) => {
    if (cond) bump(f, name);
  };
  flag(style.fauxBold, "faux bold");
  flag(style.fauxItalic, "faux italic");
  flag(style.tracking, "tracking != 0");
  flag(style.autoLeading === false && style.leading !== undefined, "fixed leading");
  flag(style.autoLeading === true, "auto leading");
  flag(style.horizontalScale !== undefined && style.horizontalScale !== 1, "horizontal scale != 100%");
  flag(style.verticalScale !== undefined && style.verticalScale !== 1, "vertical scale != 100%");
  flag(style.baselineShift, "baseline shift");
  flag(style.kerning, "manual kerning");
  flag(style.autoKerning === false, "auto kerning off");
  flag(style.fontCaps === 1, "small caps");
  flag(style.fontCaps === 2, "all caps");
  flag(style.fontBaseline === 1, "superscript");
  flag(style.fontBaseline === 2, "subscript");
  flag(style.underline, "underline");
  flag(style.strikethrough, "strikethrough");
  flag(style.ligatures === false, "ligatures off");
  flag(style.dLigatures, "discretionary ligatures");
  flag(style.strokeFlag, "text stroke");
  flag(style.noBreak, "no-break");
}

function surveyText(layer: Layer, out: Survey["text"]): void {
  const text = layer.text!;
  out.layers++;
  const f = out.features;
  // Style flags are counted once per run; fonts once per layer.
  const fonts = new Set<string>();
  if (text.style) surveyStyle(text.style, out, fonts);
  for (const run of text.styleRuns ?? []) surveyStyle(run.style, out, fonts);
  for (const font of fonts) bump(out.fonts, font);

  const runs = text.styleRuns?.length ?? 0;
  if (runs > 1) bump(f, "multiple style runs");
  if (fonts.size > 1) bump(f, "multiple fonts in one layer");
  if ((text.paragraphStyleRuns?.length ?? 0) > 1) bump(f, "multiple paragraph runs");

  bump(f, text.shapeType === "box" ? "box text" : "point text");
  if (text.orientation === "vertical") bump(f, "vertical orientation");
  if (text.textOnPath) bump(f, "text on path");
  if (text.antiAlias) bump(out.antiAlias, text.antiAlias);
  bump(out.warp, text.warp?.style ?? "none");

  const t = text.transform;
  if (t && (Math.abs(t[1]!) > 1e-6 || Math.abs(t[2]!) > 1e-6)) bump(f, "rotated/skewed transform");
  else if (t && (Math.abs(t[0]! - 1) > 1e-6 || Math.abs(t[3]! - 1) > 1e-6)) bump(f, "scaled transform");

  const paragraphs = [text.paragraphStyle, ...(text.paragraphStyleRuns ?? []).map((r) => r.style)];
  const seenJust = new Set<string>();
  const seenPara = new Set<string>();
  for (const p of paragraphs) {
    if (!p) continue;
    if (p.justification) seenJust.add(p.justification);
    if (p.firstLineIndent) seenPara.add("first-line indent");
    if (p.startIndent || p.endIndent) seenPara.add("paragraph indent");
    if (p.spaceBefore || p.spaceAfter) seenPara.add("paragraph spacing");
    if (p.autoHyphenate) seenPara.add("auto hyphenate");
    if (p.hanging) seenPara.add("hanging punctuation");
    if (p.everyLineComposer) seenPara.add("every-line composer");
  }
  for (const j of seenJust) bump(out.justification, j);
  for (const p of seenPara) bump(f, p);
}

function surveyMask(layer: Layer, out: Survey): void {
  const kind = layer.children ? "group" : "layer";
  const mask: LayerMaskData | undefined = layer.mask;
  if (mask) {
    // ag-psd reports the vector mask's rasterized cache as `mask` with fromVectorData.
    if (mask.fromVectorData) bump(out.masks, "vector mask raster cache");
    else {
      bump(out.masks, `raster mask on ${kind}`);
      if (mask.disabled) bump(out.masks, "raster mask disabled");
      if (mask.defaultColor === 255) bump(out.masks, "raster mask default white (reveal outside)");
      if (mask.positionRelativeToLayer === false) bump(out.masks, "raster mask unlinked from layer");
    }
    if (mask.userMaskDensity !== undefined && mask.userMaskDensity < 1) bump(out.masks, "raster mask density < 100%");
    if (mask.userMaskFeather) bump(out.masks, "raster mask feather");
    if (mask.vectorMaskDensity !== undefined && mask.vectorMaskDensity < 1) bump(out.masks, "vector mask density < 100%");
    if (mask.vectorMaskFeather) bump(out.masks, "vector mask feather");
  }
  if (layer.realMask) bump(out.masks, "raster + vector mask on same layer");
  if (layer.vectorMask) {
    bump(out.masks, `vector mask on ${kind}`);
    if (layer.vectorMask.invert) bump(out.masks, "vector mask inverted");
    if (layer.vectorMask.disable) bump(out.masks, "vector mask disabled");
    if (layer.vectorMask.notLink) bump(out.masks, "vector mask unlinked from layer");
    if (layer.vectorMask.paths.length > 1) bump(out.masks, "vector mask with multiple subpaths");
  }
  if (layer.clipping) {
    bump(out.masks, "clipping layer");
    bump(out.masks, `clipping layer: ${layerKind(layer)}`);
  }
  if (layer.filterMask) bump(out.masks, "smart filter mask");
}

function surveyProps(layer: Layer, out: Survey): void {
  const p = out.layerProps;
  if (layer.hidden) bump(p, "hidden");
  if (layer.opacity !== undefined && layer.opacity < 1) bump(p, "opacity < 100%");
  if (layer.fillOpacity !== undefined && layer.fillOpacity < 1) bump(p, "fill opacity < 100%");
  if (layer.blendClippendElements === false) bump(p, "blend clipped layers as group: off");
  if (layer.blendInteriorElements) bump(p, "blend interior effects as group: on");
  if (layer.knockout) bump(p, "knockout");
  if (layer.layerMaskAsGlobalMask) bump(p, "layer mask hides effects");
  if (layer.vectorMaskAsGlobalMask) bump(p, "vector mask hides effects");
  if (layer.transparencyProtected || layer.protected?.transparency) bump(p, "lock: transparency");
  if (layer.protected?.position) bump(p, "lock: position");
  if (layer.protected?.composite) bump(p, "lock: pixels");
  if (layer.protected?.all) bump(p, "lock: all");
  if (layer.linkGroup) bump(p, "linked layers");
  const full = [0, 0, 255, 255];
  const narrowed = (range: number[]) => range.some((v, i) => v !== full[i]);
  if (layer.blendingRanges?.ranges.some((r) => narrowed(r.sourceRange) || narrowed(r.destRange))) {
    bump(p, "blend-if ranges");
  }
}

/** Survey one parsed PSD into `out`. `names` enables layer-name matching. */
export function surveyPsd(psd: Psd, file: string, out: Survey, names?: KnownLayerNames): void {
  const embedded = new Map<string, { hasData: boolean; name: string }>();
  for (const link of psd.linkedFiles ?? []) {
    embedded.set(link.id, { hasData: !!link.data?.byteLength, name: link.name });
  }

  let layers = 0;
  let maxDepth = 0;
  const walk = (list: Layer[], depth: number): void => {
    maxDepth = Math.max(maxDepth, depth);
    for (const layer of list) {
      layers++;
      const kind = layerKind(layer);
      bump(out.layerKinds, kind);
      bump(out.blendModes, `${layer.blendMode ?? "normal"}${layer.children ? " (group)" : ""}`);
      if (layer.effects) surveyEffects(layer.effects, out);
      if (layer.adjustment) bump(out.adjustments, layer.adjustment.type);
      if (layer.text) surveyText(layer, out.text);
      surveyMask(layer, out);
      surveyProps(layer, out);

      if (layer.placedLayer) {
        const placed = layer.placedLayer;
        const link = embedded.get(placed.id);
        const ext = /\.([a-z0-9]+)$/i.exec(link?.name ?? "")?.[1]?.toLowerCase() ?? "unknown";
        // `external` is ag-psd's flag for a linked (not embedded) smart object.
        const where = placed.external || (link && !link.hasData) ? "linked" : link ? "embedded" : "missing data";
        bump(out.smartObjects, `${where} (${placed.type}, .${ext})`);
        if (placed.warp && placed.warp.style !== "none") bump(out.smartObjects, `warp: ${placed.warp.style}`);
        if (placed.nonAffineTransform && isPerspective(placed.transform, placed.nonAffineTransform)) {
          bump(out.smartObjects, "non-affine (perspective) transform");
        }
        if (placed.filter) {
          bump(out.smartObjects, "smart filters");
          for (const fx of placed.filter.list) bump(out.smartObjects, `smart filter: ${fx.type}`);
        }
      }

      if (names) {
        out.layerNames.checked = true;
        const name = layer.name ?? "";
        bump(names.has(name) ? out.layerNames.matched : out.layerNames.unmatched, name);
      }
      if (layer.children) walk(layer.children, depth + 1);
    }
  };
  walk(psd.children ?? [], 1);

  out.totalLayers += layers;
  const res = psd.imageResources?.resolutionInfo;
  out.files.push({
    file,
    width: psd.width,
    height: psd.height,
    dpi: res ? Math.round(res.horizontalResolution * (res.horizontalResolutionUnit === "PPCM" ? 2.54 : 1)) : undefined,
    colorMode: COLOR_MODES[psd.colorMode ?? 3] || `mode ${psd.colorMode}`,
    bitsPerChannel: psd.bitsPerChannel,
    layers,
    maxDepth,
  });
}

function isPerspective(affine: number[], nonAffine: number[]): boolean {
  return nonAffine.some((v, i) => Math.abs(v - (affine[i] ?? v)) > 0.01);
}

/** Fill in `layerNames.unused` once every file has been surveyed. */
export function finishSurvey(out: Survey, names?: KnownLayerNames): Survey {
  if (names) {
    out.layerNames.checked = true;
    out.layerNames.unused = [...names.keys()].filter((n) => !(n in out.layerNames.matched)).sort();
  }
  return out;
}
