/**
 * Photoshop vector paths -> our `Path` (an SVG path string plus a fill rule).
 *
 * A PSD vector mask is a list of closed subpaths, each with a boolean
 * operation. We have no boolean engine, so the combinations templates actually
 * use are expressed with fill rules and an "inverted" flag:
 *
 *   combine …            union: every subpath wound the same way, non-zero fill
 *   exclude …            xor: even-odd fill
 *   subtract …           everything except the union: inverted
 *   anything else        best effort, with a warning
 */
import type { BezierPath, LayerVectorMask } from "ag-psd";

export type ConvertedPath = {
  d: string;
  fillRule: "nonzero" | "evenodd";
  /** True when the path describes what to hide instead of what to show. */
  inverted: boolean;
};

type Point = [number, number];
/** [control before, anchor, control after] */
type Knot = [Point, Point, Point];

const round = (v: number) => Math.round(v * 1000) / 1000;

function knots(path: BezierPath): Knot[] {
  return path.knots.map(({ points: p }) => [[p[0]!, p[1]!], [p[2]!, p[3]!], [p[4]!, p[5]!]]);
}

/** Twice the signed area of the polygon through the anchors. Positive is clockwise in y-down pixels. */
function signedArea(k: Knot[]): number {
  let area = 0;
  for (let i = 0; i < k.length; i++) {
    const [x0, y0] = k[i]![1];
    const [x1, y1] = k[(i + 1) % k.length]![1];
    area += x0 * y1 - x1 * y0;
  }
  return area;
}

/** The same outline traversed the other way: reverse the knots and swap each one's handles. */
function reversed(k: Knot[]): Knot[] {
  return [...k].reverse().map(([before, anchor, after]) => [after, anchor, before]);
}

function subpath(k: Knot[]): string {
  if (k.length === 0) return "";
  const p = ([x, y]: Point) => `${round(x)} ${round(y)}`;
  let d = `M${p(k[0]![1])}`;
  for (let i = 0; i < k.length; i++) {
    const from = k[i]!;
    const to = k[(i + 1) % k.length]!;
    d += `C${p(from[2])} ${p(to[0])} ${p(to[1])}`;
  }
  return d + "Z";
}

/**
 * Convert a vector mask's subpaths. Returns null when the result is "the whole
 * canvas" (nothing to mask). `warn` is told about anything approximated.
 */
export function convertVectorPath(mask: Pick<LayerVectorMask, "paths" | "fillStartsWithAllPixels">, warn: (message: string) => void): ConvertedPath | null {
  const paths = mask.paths.filter((p) => p.knots.length > 0);
  if (paths.some((p) => p.open)) warn("an open vector path was closed");
  if (paths.length === 0) return mask.fillStartsWithAllPixels ? null : { d: "", fillRule: "nonzero", inverted: false };

  const ops = paths.map((p) => p.operation ?? "combine");
  // Photoshop starts from a filled canvas when the first shape is subtracted from it.
  const startsFull = mask.fillStartsWithAllPixels === true || ops[0] === "subtract";
  const all = (op: string) => ops.every((o) => o === op);
  /** Every subpath wound clockwise, so non-zero filling gives their union. */
  const union = () =>
    paths
      .map((p) => {
        const k = knots(p);
        return subpath(signedArea(k) < 0 ? reversed(k) : k);
      })
      .join("");

  if (all("subtract")) return { d: union(), fillRule: "nonzero", inverted: true };
  if (all("exclude")) return { d: paths.map((p) => subpath(knots(p))).join(""), fillRule: "evenodd", inverted: startsFull };
  if (all("combine")) return startsFull ? null : { d: union(), fillRule: "nonzero", inverted: false };

  // Mixed operations. Added shapes wind one way and subtracted ones the other, which is
  // right wherever added shapes do not overlap each other.
  warn(`vector path mixes operations (${ops.join(", ")}); overlapping shapes may differ from Photoshop`);
  const d = paths
    .map((p, i) => {
      const k = knots(p);
      const clockwise = signedArea(k) >= 0;
      const wantClockwise = ops[i] !== "subtract";
      return subpath(clockwise === wantClockwise ? k : reversed(k));
    })
    .join("");
  if (startsFull) warn("vector path starts from a filled canvas and mixes operations; treated as starting empty");
  return { d, fillRule: "nonzero", inverted: false };
}

/** A rectangle covering the whole document. */
export function canvasPath(width: number, height: number): string {
  return `M0 0H${width}V${height}H0Z`;
}
