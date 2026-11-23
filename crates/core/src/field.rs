//! Array math behind layer effects: an exact Euclidean signed distance field
//! and a Gaussian-like blur. All buffers are row-major coverage in 0..1 over a
//! w×h region.

const INF: f64 = 1e20;

/// Scratch buffers for the 1D transform, sized for the longer side.
struct Scratch {
    f: Vec<f64>,
    d: Vec<f64>,
    z: Vec<f64>,
    v: Vec<usize>,
    arg: Vec<i32>,
}

/// 1D squared-distance transform (Felzenszwalb & Huttenlocher) along one line.
/// `f` holds 0 at seeds and INF elsewhere. For each position, writes the
/// squared distance to `d` and the index of the nearest seed to `arg` (-1 if
/// the line has none).
fn edt1d(s: &mut Scratch, n: usize) {
    let (f, d, z, v, arg) = (&s.f, &mut s.d, &mut s.z, &mut s.v, &mut s.arg);
    let mut k = 0usize;
    v[0] = 0;
    z[0] = -INF;
    z[1] = INF;
    for q in 1..n {
        let fq = f[q] + (q * q) as f64;
        loop {
            let r = v[k];
            let x = (fq - (f[r] + (r * r) as f64)) / (2.0 * q as f64 - 2.0 * r as f64);
            if x <= z[k] && k > 0 {
                k -= 1;
                continue;
            }
            if x <= z[k] {
                // k == 0: the new parabola replaces the first one.
                v[0] = q;
                z[0] = -INF;
                z[1] = INF;
            } else {
                k += 1;
                v[k] = q;
                z[k] = x;
                z[k + 1] = INF;
            }
            break;
        }
    }
    k = 0;
    for q in 0..n {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let r = v[k];
        let dist = q as f64 - r as f64;
        d[q] = f[r] + dist * dist;
        arg[q] = if f[r] >= INF { -1 } else { r as i32 };
    }
}

/// For every pixel, the index of the nearest pixel where `seed` is set (-1 if
/// there are none) and the distance between the two pixel centers.
fn nearest_seed(seed: &[bool], w: usize, h: usize) -> (Vec<f32>, Vec<i32>) {
    let n = w * h;
    let size = w.max(h);
    let mut s = Scratch { f: vec![0.0; size], d: vec![0.0; size], z: vec![0.0; size + 1], v: vec![0; size], arg: vec![0; size] };
    let mut col_dist = vec![0f64; n]; // squared distance to the nearest seed in the same column
    let mut col_row = vec![0i32; n]; // and that seed's row

    for x in 0..w {
        for y in 0..h {
            s.f[y] = if seed[y * w + x] { 0.0 } else { INF };
        }
        edt1d(&mut s, h);
        for y in 0..h {
            col_dist[y * w + x] = s.d[y];
            col_row[y * w + x] = s.arg[y];
        }
    }

    let mut dist = vec![0f32; n];
    let mut near = vec![0i32; n];
    for y in 0..h {
        let row = y * w;
        s.f[..w].copy_from_slice(&col_dist[row..row + w]);
        edt1d(&mut s, w);
        for x in 0..w {
            let col = s.arg[x];
            let seed_row = if col < 0 { -1 } else { col_row[row + col as usize] };
            if seed_row < 0 {
                near[row + x] = -1;
                dist[row + x] = INF as f32;
            } else {
                near[row + x] = seed_row * w as i32 + col;
                dist[row + x] = s.d[x].sqrt() as f32;
            }
        }
    }
    (dist, near)
}

/// Signed distance, in pixels, from each pixel center to the shape's edge:
/// negative inside, positive outside. The edge is where coverage crosses 0.5,
/// and partially covered pixels place it with sub-pixel precision, so
/// anti-aliased outlines give smooth distances.
pub fn signed_distance(alpha: &[f32], w: usize, h: usize) -> Vec<f32> {
    let empty: Vec<bool> = alpha.iter().map(|&a| a <= 0.0).collect();
    let full: Vec<bool> = alpha.iter().map(|&a| a >= 1.0).collect();
    let partial: Vec<bool> = alpha.iter().map(|&a| a > 0.0 && a < 1.0).collect();
    let (empty_dist, empty_near) = nearest_seed(&empty, w, h);
    let (full_dist, full_near) = nearest_seed(&full, w, h);
    let (partial_dist, partial_near) = nearest_seed(&partial, w, h);

    // Distance from pixel i to the edge as placed by the partial pixels around
    // seed q. `side` is +1 when i is outside the shape and -1 when inside. The
    // nearest partial pixel by center is not always the one whose edge is
    // nearest, so its partial neighbours are tried as well.
    let via_partial = |i: usize, q: usize, side: f32, mut best: f32| -> f32 {
        let (px, py) = ((i % w) as i32, (i / w) as i32);
        let (qx, qy) = ((q % w) as i32, (q / w) as i32);
        for y in (qy - 1).max(0)..=(qy + 1).min(h as i32 - 1) {
            for x in (qx - 1).max(0)..=(qx + 1).min(w as i32 - 1) {
                let c = y as usize * w + x as usize;
                if partial[c] {
                    let d = ((px - x) as f32).hypot((py - y) as f32) - side * (alpha[c] - 0.5);
                    best = best.min(d);
                }
            }
        }
        best
    };

    let mut sd = vec![0f32; w * h];
    for i in 0..w * h {
        let a = alpha[i];
        if partial[i] {
            sd[i] = 0.5 - a; // the edge passes through this pixel
            continue;
        }
        let inside = a >= 1.0;
        // A crisp edge lies half a pixel before the first pixel of the other kind.
        let (dist, near) = if inside { (&empty_dist, &empty_near) } else { (&full_dist, &full_near) };
        let mut best = if near[i] < 0 { INF as f32 } else { dist[i] - 0.5 };
        let q = partial_near[i];
        // A neighbour of q can be at most ~1.5 px nearer and shift the edge by 0.5.
        if q >= 0 && partial_dist[i] - 2.0 < best {
            best = via_partial(i, q as usize, if inside { -1.0 } else { 1.0 }, best);
        }
        sd[i] = if inside { -best } else { best };
    }
    sd
}

/// Coverage of the half-plane `x <= 0` for a pixel whose center is at signed distance x.
#[inline]
pub fn cover(x: f32) -> f32 {
    (0.5 - x).clamp(0.0, 1.0)
}

/// One box-blur pass along a line of `n` values spaced `step` apart. Values
/// outside the line count as zero: pixels outside the region are transparent.
fn box_blur_line(src: &[f32], dst: &mut [f32], start: usize, step: usize, n: usize, r: usize) {
    let scale = 1.0 / (2 * r + 1) as f32;
    let mut sum = 0f32;
    for i in 0..=r.min(n - 1) {
        sum += src[start + i * step];
    }
    for i in 0..n {
        dst[start + i * step] = sum * scale;
        if i + r + 1 < n {
            sum += src[start + (i + r + 1) * step];
        }
        if i >= r {
            sum -= src[start + (i - r) * step];
        }
    }
}

/// Box radii whose three successive blurs approximate a Gaussian of `sigma`.
fn box_radii(sigma: f32) -> [usize; 3] {
    let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    let mut lower = ideal.floor() as i32;
    if lower % 2 == 0 {
        lower -= 1;
    }
    let upper = lower + 2;
    let m = ((12.0 * sigma * sigma - (3 * lower * lower + 12 * lower + 9) as f32) / (-4 * lower - 4) as f32).round() as i32;
    [0, 1, 2].map(|i| ((if i < m { lower } else { upper } - 1) / 2).max(0) as usize)
}

/// Gaussian blur (three box passes), treating everything outside the region as zero.
pub fn blur(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let mut a = src.to_vec();
    if !(sigma > 0.3) || w == 0 || h == 0 {
        return a;
    }
    let mut b = vec![0f32; src.len()];
    for r in box_radii(sigma) {
        if r < 1 {
            continue;
        }
        for y in 0..h {
            box_blur_line(&a, &mut b, y * w, 1, w, r);
        }
        for x in 0..w {
            box_blur_line(&b, &mut a, x, w, h, r);
        }
    }
    // Running sums drift by a hair; keep the result a valid coverage.
    a.iter_mut().for_each(|v| *v = v.clamp(0.0, 1.0));
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(w: usize, h: usize, f: impl Fn(usize, usize) -> f32) -> Vec<f32> {
        (0..w * h).map(|i| f(i % w, i / w)).collect()
    }
    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 0.005, "{a} is not close to {b}");
    }

    #[test]
    fn distance_measures_from_the_pixel_edge() {
        // Crisp square covering pixels 5..14 in both axes: edges at 5 and 15.
        let (w, h) = (20, 20);
        let sd = signed_distance(&grid(w, h, |x, y| if (5..15).contains(&x) && (5..15).contains(&y) { 1.0 } else { 0.0 }), w, h);
        let at = |x: usize, y: usize| sd[y * w + x];
        close(at(4, 10), 0.5);
        close(at(2, 10), 2.5);
        close(at(5, 10), -0.5);
        close(at(9, 10), -4.5);
        close(at(10, 17), 2.5);
        // Euclidean around corners: centers differ by (3,3) and (3,4) from the corner pixels.
        close(at(2, 2), 3f32.hypot(3.0) - 0.5);
        close(at(17, 18), 3f32.hypot(4.0) - 0.5);
    }

    #[test]
    fn distance_uses_partial_coverage_for_sub_pixel_edges() {
        // Edge at x = 5.25: pixel 5 is 75% covered.
        let (w, h) = (20, 20);
        let s = signed_distance(&grid(w, h, |x, _| if x > 5 { 1.0 } else if x == 5 { 0.75 } else { 0.0 }), w, h);
        close(s[10 * w + 5], -0.25);
        close(s[10 * w + 4], 0.75);
        close(s[10 * w + 3], 1.75);
        close(s[10 * w + 6], -1.25);
    }

    #[test]
    fn distance_picks_the_nearest_edge_on_ties() {
        // Row 0 is empty. In row 1, pixel 3 is 96% covered and the rest are full.
        // Pixel (4,1) is one step from both; its edge is the crisp one above, 0.5 away.
        let s = signed_distance(&grid(8, 6, |x, y| if y == 0 { 0.0 } else if y == 1 && x == 3 { 0.96 } else { 1.0 }), 8, 6);
        close(s[8 + 4], -0.5);
        close(s[2 * 8 + 4], -1.5);
        close(s[8 + 3], -0.46);
        close(s[2 * 8 + 3], -1.46);
    }

    #[test]
    fn distance_handles_empty_and_full_regions() {
        assert!(signed_distance(&[0.0; 16], 4, 4).iter().all(|&v| v > 1e9));
        assert!(signed_distance(&[1.0; 16], 4, 4).iter().all(|&v| v < -1e9));
    }

    #[test]
    fn cover_ramps_across_one_pixel() {
        assert_eq!([cover(-2.0), cover(-0.5), cover(0.0), cover(0.25), cover(0.5), cover(3.0)], [1.0, 1.0, 0.5, 0.25, 0.0, 0.0]);
    }

    #[test]
    fn blur_spreads_a_point_symmetrically_and_conserves_mass() {
        let (w, h) = (41, 41);
        let b = blur(&grid(w, h, |x, y| if x == 20 && y == 20 { 1.0 } else { 0.0 }), w, h, 3.0);
        let at = |x: usize, y: usize| b[y * w + x];
        assert!((b.iter().sum::<f32>() - 1.0).abs() < 1e-3);
        assert!(at(20, 20) > at(23, 20) && at(23, 20) > at(27, 20));
        assert!((at(17, 20) - at(23, 20)).abs() < 1e-6);
        assert!((at(20, 17) - at(23, 20)).abs() < 1e-6);
        // A Gaussian of sigma 3 has its peak at 1 / (2π·9).
        close(at(20, 20), 1.0 / (2.0 * std::f32::consts::PI * 9.0));
    }

    #[test]
    fn blur_keeps_flat_fields_flat_and_ignores_tiny_sigmas() {
        let b = blur(&vec![1.0; 61 * 61], 61, 61, 2.0);
        assert!((b[30 * 61 + 30] - 1.0).abs() < 1e-5);
        assert!(b[0] < 0.5); // outside the region counts as empty
        assert_eq!(blur(&[0.0, 1.0, 0.0], 3, 1, 0.0), [0.0, 1.0, 0.0]);
    }
}
