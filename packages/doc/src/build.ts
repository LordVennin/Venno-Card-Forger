/** Small constructors so hand-written documents and tests stay readable. */
import type { Doc, GroupLayer, Layer, LayerBase, PixelLayer, ShapeLayer, Color, Mat3 } from "./types.js";
import { IDENTITY } from "./types.js";

let counter = 0;

/** Unique within a process and unlikely to collide across sessions. */
export function newId(prefix = "l"): string {
  counter = (counter + 1) % 0xffffff;
  return `${prefix}_${Date.now().toString(36)}${counter.toString(36)}${Math.floor(Math.random() * 1296).toString(36)}`;
}

type Base = Partial<LayerBase> & { id: string };

function base(props: Base): LayerBase {
  return { name: props.id, visible: true, opacity: 1, blend: "normal", clip: false, ...props };
}

export function group(props: Base & { passThrough?: boolean }, children: Layer[]): GroupLayer {
  return { ...base(props), kind: "group", passThrough: props.passThrough ?? true, children };
}

export function pixel(props: Base & { asset: string; transform?: Mat3 }): PixelLayer {
  return { ...base(props), kind: "pixel", asset: props.asset, transform: props.transform ?? IDENTITY };
}

export function shape(props: Base & { d: string; color: Color } & Partial<Pick<ShapeLayer, "stroke">>): ShapeLayer {
  const { d, color, ...rest } = props;
  return { ...base(rest), kind: "shape", path: { d }, fill: { type: "solid", color }, ...(props.stroke ? { stroke: props.stroke } : {}) };
}

/** SVG path for an axis-aligned rectangle. */
export function rectPath(x: number, y: number, w: number, h: number): string {
  return `M${x} ${y}H${x + w}V${y + h}H${x}Z`;
}

export function emptyDoc(width: number, height: number, dpi = 72): Doc {
  return { version: 1, width, height, dpi, assets: {}, root: group({ id: "root", name: "Root" }, []) };
}
