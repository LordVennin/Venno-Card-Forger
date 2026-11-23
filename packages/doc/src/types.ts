/**
 * The document format (SPEC §2), as the Rust core reads and writes it
 * (`crates/core/src/doc.rs`). Keep the two in step: the core rejects anything
 * that does not match. Plain JSON all the way down; binary data lives in
 * assets and is referenced by id.
 */

export type LayerId = string;
export type AssetId = string;

export type Vec2 = { x: number; y: number };
export type Rect = { x: number; y: number; w: number; h: number };

/**
 * 2D affine transform as six numbers, row-major:
 *
 *   | a  c  tx |
 *   | b  d  ty |
 *
 * stored as [a, c, tx, b, d, ty], the same order Skia uses for the first two
 * rows of its 3×3 matrix. Maps layer-local pixels to document pixels.
 */
export type Mat3 = [number, number, number, number, number, number];

/** Straight (non-premultiplied) RGBA, each 0..1, in sRGB. */
export type Color = [number, number, number, number];

/** Vector outline as an SVG path string in document pixels. */
export type Path = { d: string; fillRule?: "nonzero" | "evenodd" };

/**
 * Blend modes. Only the modes found by the survey (docs/SURVEY.md): four on
 * layers, plus multiply, screen and luminosity used inside effects.
 */
export type BlendMode =
  | "normal"
  | "multiply"
  | "screen"
  | "overlay"
  | "softLight"
  | "linearLight"
  | "luminosity";

export const BLEND_MODES: readonly BlendMode[] = [
  "normal",
  "multiply",
  "screen",
  "overlay",
  "softLight",
  "linearLight",
  "luminosity",
];

export type GradientStop = { offset: number; color: Color };
export type Gradient = { stops: GradientStop[] };

/** Fields shared by effects that paint one color through a blend mode. */
type EffectPaint = { enabled: boolean; blend: BlendMode; opacity: number };

/**
 * Layer effects. Only the seven types found by the survey. Sizes and distances
 * are in document pixels (the importer applies Photoshop's effects scale).
 * Angles are in degrees, counter-clockwise from +x, pointing toward the light.
 */
export type Effect =
  | (EffectPaint & { type: "colorOverlay"; color: Color })
  | (EffectPaint & {
      type: "gradientOverlay";
      gradient: Gradient;
      style: "linear" | "radial";
      angle: number;
      /** 1 = the gradient spans the layer's bounds exactly. */
      scale: number;
      reverse?: boolean;
    })
  | (EffectPaint & {
      type: "stroke";
      color: Color;
      size: number;
      position: "outside" | "inside" | "center";
    })
  | (EffectPaint & {
      type: "dropShadow";
      color: Color;
      angle: number;
      distance: number;
      /** 0..1: the share of `size` that is a hard expansion instead of blur. */
      spread: number;
      size: number;
      /** Falloff curve. "gaussian" is Photoshop's Gaussian contour preset. Default "linear". */
      contour?: "linear" | "gaussian";
    })
  | (EffectPaint & {
      type: "innerShadow";
      color: Color;
      angle: number;
      distance: number;
      /** 0..1: the share of `size` that is a hard contraction instead of blur. */
      choke: number;
      size: number;
    })
  | (EffectPaint & { type: "outerGlow"; color: Color; spread: number; size: number })
  | {
      type: "bevel";
      enabled: boolean;
      style: "inner" | "outer";
      /** Only "chiselHard" appears in the survey. */
      technique: "chiselHard";
      direction: "up" | "down";
      /** 1 = 100%. */
      depth: number;
      size: number;
      soften: number;
      angle: number;
      altitude: number;
      highlight: { color: Color; blend: BlendMode; opacity: number };
      shadow: { color: Color; blend: BlendMode; opacity: number };
    };

export type EffectType = Effect["type"];

export type RasterMask = {
  /** Grayscale image asset: white reveals, black hides. */
  asset: AssetId;
  /** Document position of the mask image's top-left pixel. */
  offset: Vec2;
  /** What the mask does outside its image. Photoshop's "default color". Default "hide". */
  outside?: "hide" | "reveal";
  /** Masks can be switched off without being removed. Default true. */
  enabled?: boolean;
  inverted?: boolean;
  /** 0..1. At 0 the mask has no effect. Default 1. */
  density?: number;
  /** Blur radius in pixels. Default 0. */
  feather?: number;
};

export type VectorMask = { path: Path; enabled?: boolean; inverted?: boolean };

export type LayerBase = {
  id: LayerId;
  name: string;
  visible: boolean;
  /** 0..1. Scales the layer and all of its effects. */
  opacity: number;
  /** 0..1. Scales the layer's own pixels but not its effects. Default 1. */
  fillOpacity?: number;
  blend: BlendMode;
  /** Clipped to the nearest layer below that is not itself clipped. */
  clip: boolean;
  mask?: RasterMask;
  vectorMask?: VectorMask;
  effects?: Effect[];
  /** Master switch for the whole layer style. Default true. */
  effectsEnabled?: boolean;
  locked?: { pixels?: boolean; position?: boolean; all?: boolean };
  /** Semantic tag used by the card engine, e.g. "pinlines.W". */
  role?: string;
};

export type GroupLayer = LayerBase & {
  kind: "group";
  /** Paint order: children[0] is the bottom layer. */
  children: Layer[];
  /** True: children blend straight into what is below the group. False: the group is isolated. */
  passThrough: boolean;
};

export type PixelLayer = LayerBase & { kind: "pixel"; asset: AssetId; transform: Mat3 };

export type Fill = { type: "solid"; color: Color };
export type Stroke = { color: Color; width: number; align: "center" | "inside" };

export type ShapeLayer = LayerBase & { kind: "shape"; path: Path; fill: Fill; stroke?: Stroke };

export type TextRun = { text: string; italic?: boolean; bold?: boolean; font?: string; symbol?: string };
export type RichText = { runs: TextRun[] };
export type TextStyle = {
  font: string;
  size: number;
  color: Color;
  /** Line height as a multiple of size. Omit for the font's natural leading. */
  leading?: number;
  tracking?: number;
  align: "left" | "center" | "right";
};
export type FitMode = "none" | "shrinkHeight" | "shrinkWidth";

/** Declared now so documents can carry text; rendered from Phase 3. */
export type TextLayer = LayerBase & {
  kind: "text";
  box: Rect;
  content: RichText;
  style: TextStyle;
  fit: FitMode;
};

export type ArtFrameLayer = LayerBase & {
  kind: "art";
  frameRef: Rect;
  asset?: AssetId;
  transform: Mat3;
  fit: "cover" | "contain" | "manual";
};

/** Generated content (rule 4): always its own layer, never the user's art. */
export type FillLayer = LayerBase & {
  kind: "fill";
  source: string;
  params: Record<string, unknown>;
  asset: AssetId;
  transform: Mat3;
};

// No AdjustmentLayer: the survey found none.
export type Layer = GroupLayer | PixelLayer | ShapeLayer | TextLayer | ArtFrameLayer | FillLayer;
export type LayerKind = Layer["kind"];

export type AssetRef = {
  kind: "image" | "svg" | "font";
  /** Path inside the save file or template package, e.g. "assets/frame.png". */
  path: string;
  width?: number;
  height?: number;
};

/** Scryfall-shaped card JSON. Typed properly in packages/cards (Phase 4). */
export type CardData = Record<string, unknown>;

export type Doc = {
  version: 1;
  width: number;
  height: number;
  dpi: number;
  root: GroupLayer;
  assets: Record<AssetId, AssetRef>;
  card?: CardData;
  templateId?: string;
};

export const IDENTITY: Mat3 = [1, 0, 0, 0, 1, 0];
