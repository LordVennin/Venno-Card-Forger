/**
 * Pure array math behind layer effects: an exact Euclidean signed distance
 * field and a Gaussian-like blur. No CanvasKit and no DOM, so effects come out
 * bit-identical in Node and in the app.
 *
 * All arrays are row-major coverage in 0..1 over a w×h region.
 */

const INF = 1e20;

/**
 * 1D squared-distance transform (Felzenszwalb & Huttenlocher) along one line.
 * `f` holds 0 at seeds and INF elsewhere. Writes, for each position, the index
 * of the nearest seed along the line into `arg` (or -1 if the line has none).
 */
function edt1d(f: Float64Array, n: number, v: Int32Array, z: Float64Array, d: Float64Array, arg: Int32Array): void {
  let k = 0;
  v[0] = 0;
  z[0] = -INF;
  z[1] = INF;
  for (let q = 1; q < n; q++) {
    let s: number;
    do {
      const r = v[k]!;
      s = (f[q]! + q * q - (f[r]! + r * r)) / (2 * q - 2 * r);
    } while (s <= z[k]! && --k > -1);
    k++;
    v[k] = q;
    z[k] = s;
    z[k + 1] = INF;
  }
  k = 0;
  for (let q = 0; q < n; q++) {
    while (z[k + 1]! < q) k++;
    const r = v[k]!;
    d[q] = f[r]! + (q - r) * (q - r);
    arg[q] = f[r]! >= INF ? -1 : r;
  }
}

/**
 * For every pixel, the index of the nearest pixel where `seed` is true (or -1
 * if there are none) and the distance between the two pixel centers.
 */
function nearestSeed(seed: Uint8Array, w: number, h: number): { dist: Float32Array; near: Int32Array } {
  const n = w * h;
  const size = Math.max(w, h);
  const f = new Float64Array(size);
  const d = new Float64Array(size);
  const z = new Float64Array(size + 1);
  const v = new Int32Array(size);
  const arg = new Int32Array(size);
  const colDist = new Float64Array(n); // squared distance to the nearest seed in the same column
  const colRow = new Int32Array(n); // and that seed's row

  for (let x = 0; x < w; x++) {
    for (let y = 0; y < h; y++) f[y] = seed[y * w + x] ? 0 : INF;
    edt1d(f, h, v, z, d, arg);
    for (let y = 0; y < h; y++) {
      colDist[y * w + x] = d[y]!;
      colRow[y * w + x] = arg[y]!;
    }
  }

  const dist = new Float32Array(n);
  const near = new Int32Array(n);
  for (let y = 0; y < h; y++) {
    const row = y * w;
    for (let x = 0; x < w; x++) f[x] = colDist[row + x]!;
    edt1d(f, w, v, z, d, arg);
    for (let x = 0; x < w; x++) {
      const col = arg[x]!;
      const seedRow = col < 0 ? -1 : colRow[row + col]!;
      near[row + x] = seedRow < 0 ? -1 : seedRow * w + col;
      dist[row + x] = seedRow < 0 ? INF : Math.sqrt(d[x]!);
    }
  }
  return { dist, near };
}

/**
 * Signed distance, in pixels, from each pixel center to the shape's edge:
 * negative inside, positive outside. The edge is where coverage crosses 0.5,
 * and partially covered pixels place it with sub-pixel precision, so
 * anti-aliased outlines give smooth distances.
 */
export function signedDistance(alpha: Float32Array, w: number, h: number): Float32Array {
  const n = w * h;
  const empty = new Uint8Array(n);
  const full = new Uint8Array(n);
  const partial = new Uint8Array(n);
  for (let i = 0; i < n; i++) {
    const a = alpha[i]!;
    if (a <= 0) empty[i] = 1;
    else if (a >= 1) full[i] = 1;
    else partial[i] = 1;
  }
  const toEmpty = nearestSeed(empty, w, h);
  const toFull = nearestSeed(full, w, h);
  const toPartial = nearestSeed(partial, w, h);

  // Distance from pixel i to the edge as placed by the partial pixels around
  // seed q. `side` is +1 when i is outside the shape and -1 when inside. The
  // nearest partial pixel by center is not always the one whose edge is
  // nearest, so its partial neighbours are tried as well.
  const viaPartial = (i: number, q: number, side: number, best: number): number => {
    const px = i % w, py = (i - px) / w;
    const qx = q % w, qy = (q - qx) / w;
    for (let dy = -1; dy <= 1; dy++) {
      const y = qy + dy;
      if (y < 0 || y >= h) continue;
      for (let dx = -1; dx <= 1; dx++) {
        const x = qx + dx;
        if (x < 0 || x >= w) continue;
        const c = y * w + x;
        if (!partial[c]) continue;
        const d = Math.hypot(px - x, py - y) - side * (alpha[c]! - 0.5);
        if (d < best) best = d;
      }
    }
    return best;
  };

  const sd = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    const a = alpha[i]!;
    if (partial[i]) {
      sd[i] = 0.5 - a; // the edge passes through this pixel
      continue;
    }
    const inside = a >= 1;
    // A crisp edge lies half a pixel before the first pixel of the other kind.
    const crisp = inside ? toEmpty : toFull;
    let best = crisp.near[i]! < 0 ? INF : crisp.dist[i]! - 0.5;
    const q = toPartial.near[i]!;
    // A neighbour of q can be at most ~1.5 px nearer and shift the edge by 0.5.
    if (q >= 0 && toPartial.dist[i]! - 2 < best) best = viaPartial(i, q, inside ? -1 : 1, best);
    sd[i] = inside ? -best : best;
  }
  return sd;
}

/** Coverage of the half-plane `x <= 0` for a pixel whose center is at signed distance x. */
export function cover(x: number): number {
  return x <= -0.5 ? 1 : x >= 0.5 ? 0 : 0.5 - x;
}

function boxBlurLine(src: Float32Array, dst: Float32Array, start: number, step: number, n: number, r: number): void {
  // Edge values are zero: pixels outside the region are transparent.
  const scale = 1 / (2 * r + 1);
  let sum = 0;
  for (let i = 0; i <= r && i < n; i++) sum += src[start + i * step]!;
  for (let i = 0; i < n; i++) {
    dst[start + i * step] = sum * scale;
    const add = i + r + 1;
    const drop = i - r;
    if (add < n) sum += src[start + add * step]!;
    if (drop >= 0) sum -= src[start + drop * step]!;
  }
}

/** Box radii whose three successive blurs approximate a Gaussian of `sigma`. */
function boxRadii(sigma: number): number[] {
  const ideal = Math.sqrt((12 * sigma * sigma) / 3 + 1);
  let lower = Math.floor(ideal);
  if (lower % 2 === 0) lower--;
  const upper = lower + 2;
  const m = Math.round((12 * sigma * sigma - 3 * lower * lower - 12 * lower - 9) / (-4 * lower - 4));
  return [0, 1, 2].map((i) => ((i < m ? lower : upper) - 1) / 2);
}

/** Gaussian blur (three box passes), treating everything outside the region as zero. */
export function blur(src: Float32Array, w: number, h: number, sigma: number): Float32Array {
  if (!(sigma > 0.3)) return src;
  let a = Float32Array.from(src);
  let b = new Float32Array(src.length);
  for (const r of boxRadii(sigma)) {
    if (r < 1) continue;
    for (let y = 0; y < h; y++) boxBlurLine(a, b, y * w, 1, w, r);
    for (let x = 0; x < w; x++) boxBlurLine(b, a, x, w, h, r);
  }
  // Float32 running sums drift by a hair; keep the result a valid coverage.
  for (let i = 0; i < a.length; i++) a[i] = a[i]! < 0 ? 0 : a[i]! > 1 ? 1 : a[i]!;
  void b;
  return a;
}
