/**
 * PSD layer tree -> our document. Pure: pixels go out through `emit`, and
 * nothing here touches the file system, so it can be tested on hand-built PSD
 * objects.
 *
 * The scope is what docs/SURVEY.md found. Anything else a PSD contains is
 * reported as a warning, never silently dropped.
 */
import type { Doc, GroupLayer, Layer, LayerBase, RasterMask, RichText, TextRun, VectorMask } from "@cardsmith/doc";
import type { Layer as PsdLayer, PixelData, Psd } from "ag-psd";
import { encodePng } from "../png.js";
import { blendMode, color, convertEffects, type Warn } from "./effects.js";
import { canvasPath, convertVectorPath } from "./paths.js";

export type ConvertOptions = {
  /** Called with each image the document needs, already encoded as PNG. */
  emit(path: string, png: Uint8Array): void;
  /** Make `layer.imageData` and `layer.mask.imageData` available (decode on demand). Default: assume they already are. */
  decode?(layer: PsdLayer): void;
  /** Free what `decode` produced, once the layer's images have been emitted. */
  release?(layer: PsdLayer): void;
};

export type ConvertResult = {
  doc: Doc;
  warnings: string[];
  /** Where Photoshop drew each text layer's ink, by layer id: [left, top, right, bottom]. For checking our layout. */
  textInk: Record<string, [number, number, number, number]>;
};

/** A 1×1 transparent image, for layers that have a mask or a role but no pixels of their own. */
const EMPTY = "empty";

function grayOf(data: PixelData): Uint8Array {
  const n = data.width * data.height;
  if (data.data.length === n) return new Uint8Array(data.data.buffer, data.data.byteOffset, n);
  // RGBA: the mask value is repeated in the color channels.
  const out = new Uint8Array(n);
  for (let i = 0; i < n; i++) out[i] = data.data[i * 4]!;
  return out;
}

export function convertPsd(psd: Psd, options: ConvertOptions): ConvertResult {
  const warnings: string[] = [];
  const textInk: ConvertResult["textInk"] = {};
  const doc: Doc = {
    version: 1,
    width: psd.width,
    height: psd.height,
    dpi: (() => {
      const r = psd.imageResources?.resolutionInfo;
      return r ? Math.round(r.horizontalResolution * (r.horizontalResolutionUnit === "PPCM" ? 2.54 : 1)) : 72;
    })(),
    assets: {},
    root: { kind: "group", id: "root", name: "Root", visible: true, opacity: 1, blend: "normal", clip: false, passThrough: true, children: [] },
  };
  const global = { angle: psd.imageResources?.globalAngle ?? 120, altitude: psd.imageResources?.globalAltitude ?? 30 };
  const used = new Set<string>(["root"]);
  let counter = 0;

  const newId = (layer: PsdLayer): string => {
    let id = layer.id !== undefined ? `l${layer.id}` : `n${++counter}`;
    while (used.has(id)) id = `n${++counter}`;
    used.add(id);
    return id;
  };

  const addImage = (id: string, path: string, png: () => Uint8Array, size?: { width: number; height: number }) => {
    if (doc.assets[id]) return;
    doc.assets[id] = { kind: "image", path, ...size };
    options.emit(path, png());
  };

  const convert = (layer: PsdLayer, trail: string): Layer => {
    const where = `${trail}/${layer.name ?? ""}`;
    const warn: Warn = (message) => warnings.push(message.startsWith(where) ? message : `${where}: ${message}`);
    const id = newId(layer);
    const isGroup = Array.isArray(layer.children);
    const passThrough = layer.blendMode === "pass through";

    const base: LayerBase = {
      id,
      name: layer.name ?? id,
      visible: !layer.hidden,
      opacity: layer.opacity ?? 1,
      blend: passThrough ? "normal" : blendMode(layer.blendMode, where, warnings.push.bind(warnings)),
      clip: !!layer.clipping,
    };
    if (layer.fillOpacity !== undefined && layer.fillOpacity < 1) base.fillOpacity = layer.fillOpacity;
    if (passThrough && !isGroup) warn("pass-through blending on a layer that is not a group; using normal");

    // Things outside the survey's scope that would change how the layer looks.
    if (layer.adjustment) warn(`adjustment layer (${layer.adjustment.type}) is not supported; it has no effect`);
    if (layer.knockout) warn("knockout is not supported");
    if (layer.blendClippendElements === false) warn('"blend clipped layers as group" is off, which is not supported');
    if (layer.blendInteriorElements) warn('"blend interior effects as group" is on, which is not supported');
    const full = [0, 0, 255, 255];
    if (layer.blendingRanges?.ranges.some((r) => [...r.sourceRange, ...r.destRange].some((v, i) => v !== full[i % 4]))) warn("blend-if ranges are not supported");

    options.decode?.(layer);

    // Masks. Photoshop stores a raster copy of a vector mask; when that is all the raster mask is, skip it.
    if (layer.mask && !layer.mask.fromVectorData) {
      const m = layer.mask;
      const data = m.imageData;
      const mask: RasterMask = { asset: `${id}.mask`, offset: { x: m.left ?? 0, y: m.top ?? 0 } };
      if (m.defaultColor === 255) mask.outside = "reveal";
      if (m.disabled) mask.enabled = false;
      if (m.userMaskDensity !== undefined && m.userMaskDensity < 1) mask.density = m.userMaskDensity;
      if (m.userMaskFeather) mask.feather = m.userMaskFeather;
      if (data && data.width > 0 && data.height > 0) {
        addImage(mask.asset, `assets/${id}.mask.png`, () => encodePng(data.width, data.height, grayOf(data), 1), { width: data.width, height: data.height });
        base.mask = mask;
      } else if (m.defaultColor !== 255) {
        // An empty mask that hides by default hides everything.
        addImage(mask.asset, `assets/${id}.mask.png`, () => encodePng(1, 1, new Uint8Array([0]), 1), { width: 1, height: 1 });
        base.mask = mask;
      }
    }
    if (layer.realMask) warn("a second raster mask (alongside a vector mask) is not supported; ignored");

    const vector = layer.vectorMask ? convertVectorPath(layer.vectorMask, warn) : undefined;
    const isShape = !!layer.vectorFill && !isGroup && !layer.text;
    if (layer.vectorMask && vector && !isShape) {
      const vm: VectorMask = { path: { d: vector.d, ...(vector.fillRule === "evenodd" ? { fillRule: "evenodd" as const } : {}) } };
      if (vector.inverted !== !!layer.vectorMask.invert) vm.inverted = true;
      if (layer.vectorMask.disable) vm.enabled = false;
      if ((layer.mask?.vectorMaskDensity ?? 1) < 1 || layer.mask?.vectorMaskFeather) warn("vector mask density and feather are not supported");
      base.vectorMask = vm;
    } else if (layer.vectorMask && !vector && !isShape && layer.vectorMask.invert) {
      // "Everything", inverted: hides the whole layer.
      base.vectorMask = { path: { d: "" } };
    }

    if (layer.effects) {
      const { effects, enabled } = convertEffects(layer.effects, global, where, warnings.push.bind(warnings));
      if (effects.length) {
        base.effects = effects;
        if (!enabled) base.effectsEnabled = false;
      }
    }

    let result: Layer;
    if (isGroup) {
      const children = layer.children!.map((child) => convert(child, where));
      result = { ...base, kind: "group", passThrough, children } satisfies GroupLayer;
    } else if (layer.text) {
      result = { ...base, kind: "text", ...convertText(layer, warn) };
      textInk[id] = [layer.left ?? 0, layer.top ?? 0, layer.right ?? 0, layer.bottom ?? 0];
    } else if (isShape) {
      const fill = layer.vectorFill!;
      if (fill.type !== "color") warn(`${fill.type} shape fills are not supported; using black`);
      const fillColor = fill.type === "color" ? color(fill.color) : color(undefined);
      if (layer.vectorStroke?.fillEnabled === false) fillColor[3] = 0;
      // What the shape covers: its vector path, or the whole canvas when it has none or the path is switched off.
      let path = { d: canvasPath(psd.width, psd.height) } as { d: string; fillRule?: "evenodd" };
      if (layer.vectorMask && !layer.vectorMask.disable && vector) {
        const flip = vector.inverted !== !!layer.vectorMask.invert;
        if (!flip) path = { d: vector.d, ...(vector.fillRule === "evenodd" ? { fillRule: "evenodd" as const } : {}) };
        else {
          // Everything except the path: the canvas outline plus the path, filled even-odd.
          if (vector.fillRule === "nonzero" && (layer.vectorMask.paths.length ?? 0) > 1) warn("an inverted shape with overlapping subpaths may differ from Photoshop");
          path = { d: canvasPath(psd.width, psd.height) + vector.d, fillRule: "evenodd" };
        }
      }
      const stroke = layer.vectorStroke;
      result = { ...base, kind: "shape", path, fill: { type: "solid", color: fillColor } };
      if (stroke?.strokeEnabled && (stroke.lineWidth?.value ?? 0) > 0) {
        if (stroke.content?.type !== "color") warn("only solid-color shape strokes are supported; using black");
        if (stroke.lineAlignment === "outside") warn("outside-aligned shape strokes are not supported; drawn centered");
        if (stroke.lineDashSet?.length) warn("dashed shape strokes are not supported; drawn solid");
        result.stroke = {
          color: color(stroke.content?.type === "color" ? stroke.content.color : undefined, stroke.opacity ?? 1),
          width: stroke.lineWidth!.value,
          align: stroke.lineAlignment === "inside" ? "inside" : "center",
        };
      }
    } else {
      // Pixel layers, including rasterized smart objects.
      const data = layer.imageData;
      if (data && data.width > 0 && data.height > 0) {
        const bytes = new Uint8Array(data.data.buffer, data.data.byteOffset, data.data.byteLength);
        addImage(id, `assets/${id}.png`, () => encodePng(data.width, data.height, bytes), { width: data.width, height: data.height });
        result = { ...base, kind: "pixel", asset: id, transform: [1, 0, layer.left ?? 0, 0, 1, layer.top ?? 0] };
      } else {
        addImage(EMPTY, "assets/empty.png", () => encodePng(1, 1, new Uint8Array(4)), { width: 1, height: 1 });
        result = { ...base, kind: "pixel", asset: EMPTY, transform: [1, 0, 0, 0, 1, 0] };
      }
    }
    options.release?.(layer);
    return result;
  };

  doc.root.children = (psd.children ?? []).map((layer) => convert(layer, ""));
  return { doc, warnings, textInk };
}

/** The text-specific fields of a text layer. Photoshop's text data is converted, not rendered. */
function convertText(layer: PsdLayer, warn: Warn) {
  const text = layer.text!;
  const t = text.transform ?? [1, 0, 0, 1, 0, 0];
  if (Math.abs(t[1]!) > 1e-6 || Math.abs(t[2]!) > 1e-6) warn("rotated or skewed text is not supported; drawn upright");
  if (text.warp && text.warp.style !== "none") warn(`text warp (${text.warp.style}) is not supported`);
  if (text.orientation === "vertical") warn("vertical text is not supported");
  // The text's own scale multiplies every size in it.
  const sx = Math.abs(t[0] ?? 1);
  const sy = Math.abs(t[3] ?? 1);

  // The style of each run is the layer's style with the run's differences on top.
  // Either may be missing a field, so the base style is the first run's.
  const styled = (text.styleRuns?.length ? text.styleRuns : [{ length: text.text.length, style: {} }]).map((run) => ({ length: run.length, style: { ...(text.style ?? {}), ...run.style } }));
  const first = styled.find((run) => run.length > 0)?.style ?? text.style ?? {};
  const baseFont = first.font?.name ?? "";
  const baseSize = (first.fontSize ?? 12) * sy;
  const baseColor = color(first.fillColor);
  if (!baseFont) warn("the text has no font");

  const runs: TextRun[] = [];
  let at = 0;
  for (const run of styled) {
    const part = text.text.slice(at, at + run.length);
    at += run.length;
    if (!part) continue;
    const font = run.style.font?.name ?? baseFont;
    const size = (run.style.fontSize ?? 12) * sy;
    const fill = color(run.style.fillColor);
    const out: TextRun = { text: part.replace(/\r/g, "\n") };
    if (font !== baseFont) out.font = font;
    if (Math.abs(size - baseSize) > 1e-6) out.size = size;
    if (fill.some((v, i) => v !== baseColor[i])) out.color = fill;
    if ((run.style.tracking ?? 0) !== (first.tracking ?? 0)) out.tracking = run.style.tracking ?? 0;
    if (/Italic|It$|Obl/i.test(font) || run.style.fauxItalic) out.italic = true;
    if (/Bold|Semibold/i.test(font) || run.style.fauxBold) out.bold = true;
    if (run.style.fauxItalic || run.style.fauxBold) warn("faux bold and faux italic are not drawn");
    if (run.style.baselineShift) warn("baseline shift is not supported");
    if (run.style.fontCaps) warn("all caps and small caps are not supported");
    const last = runs[runs.length - 1];
    const same = last && last.font === out.font && last.size === out.size && last.tracking === out.tracking && last.italic === out.italic && last.bold === out.bold && JSON.stringify(last.color) === JSON.stringify(out.color);
    if (same) last.text += out.text;
    else runs.push(out);
  }
  // Photoshop ends the text with a paragraph mark that draws nothing.
  const tail = runs[runs.length - 1];
  if (tail) {
    tail.text = tail.text.replace(/\n$/, "");
    if (!tail.text) runs.pop();
  }
  const content: RichText = { runs };
  const paragraph = { ...(text.paragraphStyle ?? {}), ...(text.paragraphStyleRuns?.[0]?.style ?? {}) };
  const justification = paragraph.justification ?? "left";
  if (!["left", "center", "right"].includes(justification)) warn(`paragraph justification "${justification}" is not supported; using left`);
  const spacing = ((paragraph.spaceBefore ?? 0) + (paragraph.spaceAfter ?? 0)) * sy;
  const stretch = ((first.horizontalScale ?? 1) * sx) / sy;

  // Box text carries its box. Point text hangs from the start of its first baseline.
  const ink = { x: layer.left ?? 0, y: layer.top ?? 0, w: (layer.right ?? 0) - (layer.left ?? 0), h: (layer.bottom ?? 0) - (layer.top ?? 0) };
  const isBox = text.shapeType === "box" && !!text.boxBounds;
  const box = isBox ? { x: t[4]!, y: t[5]!, w: (text.boxBounds![2]! - text.boxBounds![0]!) * sx, h: (text.boxBounds![3]! - text.boxBounds![1]!) * sy } : ink;

  return {
    box,
    ...(isBox ? {} : { origin: { x: t[4]!, y: t[5]! } }),
    content,
    style: {
      font: baseFont,
      size: baseSize,
      color: baseColor,
      ...(first.autoLeading === false && first.leading && first.fontSize ? { leading: first.leading / first.fontSize } : {}),
      ...(first.tracking ? { tracking: first.tracking } : {}),
      ...(Math.abs(stretch - 1) > 1e-6 ? { scaleX: stretch } : {}),
      ...(spacing ? { paragraphSpacing: spacing } : {}),
      align: (justification === "center" || justification === "right" ? justification : "left") as "left" | "center" | "right",
    },
    fit: "none" as const,
  };
}
