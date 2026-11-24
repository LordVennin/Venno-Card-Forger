//! PatchMatch inpainting: fill a hole in an image with patches copied from
//! the rest of it, so the fill carries on the picture's own textures.
//!
//! This is the method behind Photoshop's Content-Aware Fill (Barnes et al.,
//! "PatchMatch", 2009, used as in Wexler et al.'s space-time completion). The
//! image is shrunk into a pyramid. At each size, every patch that touches the
//! hole looks for the most similar patch that lies wholly outside it; then
//! each hole pixel becomes the average of what those matches say it should
//! be. The matches found at one size start the search at the next.
//!
//! It is deterministic for a given seed; a different seed gives a different,
//! equally plausible fill.

/// Patches are 7 x 7.
const R: i32 = 3;

/// An RGB image, three bytes a pixel, with a matching "this pixel is missing" map.
#[derive(Clone)]
struct Level {
    w: i32,
    h: i32,
    rgb: Vec<u8>,
    hole: Vec<bool>,
    /// Whether the patch centered here lies wholly inside the image and outside the hole.
    source: Vec<bool>,
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// A whole number in `lo..hi` (`hi` if the range is empty).
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next() >> 33) as i32 % (hi - lo)
    }
}

impl Level {
    fn new(w: i32, h: i32, rgb: Vec<u8>, hole: Vec<bool>) -> Level {
        // A patch is a source if no pixel of it is missing: count missing pixels with a summed-area table.
        let stride = (w + 1) as usize;
        let mut sum = vec![0u32; stride * (h + 1) as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                sum[(y + 1) * stride + x + 1] = hole[y * w as usize + x] as u32 + sum[y * stride + x + 1] + sum[(y + 1) * stride + x] - sum[y * stride + x];
            }
        }
        let mut source = vec![false; (w * h) as usize];
        for y in R..h - R {
            for x in R..w - R {
                let (x0, y0, x1, y1) = ((x - R) as usize, (y - R) as usize, (x + R + 1) as usize, (y + R + 1) as usize);
                let missing = sum[y1 * stride + x1] + sum[y0 * stride + x0] - sum[y0 * stride + x1] - sum[y1 * stride + x0];
                source[(y * w + x) as usize] = missing == 0;
            }
        }
        Level { w, h, rgb, hole, source }
    }

    fn at(&self, x: i32, y: i32) -> usize {
        (y.clamp(0, self.h - 1) * self.w + x.clamp(0, self.w - 1)) as usize
    }

    /// Half the size: each pixel the average of the known pixels of its 2 x 2
    /// block, and missing only if all four are.
    fn half(&self) -> Level {
        let (w, h) = ((self.w + 1) / 2, (self.h + 1) / 2);
        let mut rgb = vec![0u8; (w * h * 3) as usize];
        let mut hole = vec![false; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let (mut sum, mut known) = ([0u32; 3], 0u32);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let i = self.at(x * 2 + dx, y * 2 + dy);
                    if !self.hole[i] {
                        known += 1;
                        for c in 0..3 {
                            sum[c] += self.rgb[i * 3 + c] as u32;
                        }
                    }
                }
                let o = (y * w + x) as usize;
                // A pixel is only trusted as known when its whole block is, so a hole's edge never leaks half-known colors.
                hole[o] = known < 4;
                if known > 0 {
                    for c in 0..3 {
                        rgb[o * 3 + c] = (sum[c] / known) as u8;
                    }
                }
            }
        }
        Level::new(w, h, rgb, hole)
    }

    /// How different the patch around `(px, py)` is from the one around `(qx, qy)`, giving up past `limit`.
    fn distance(&self, px: i32, py: i32, qx: i32, qy: i32, limit: u32) -> u32 {
        let mut total = 0u32;
        for dy in -R..=R {
            for dx in -R..=R {
                let (a, b) = (self.at(px + dx, py + dy) * 3, self.at(qx + dx, qy + dy) * 3);
                for c in 0..3 {
                    let d = self.rgb[a + c] as i32 - self.rgb[b + c] as i32;
                    total += (d * d) as u32;
                }
            }
            if total >= limit {
                return total;
            }
        }
        total
    }
}

/// Give every missing pixel the color of known pixels near it, spreading inwards ring by ring.
fn spread(level: &mut Level) {
    let (w, h) = (level.w, level.h);
    let mut known: Vec<bool> = level.hole.iter().map(|m| !m).collect();
    if !known.iter().any(|k| *k) {
        return;
    }
    loop {
        let mut ring = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                if known[i] {
                    continue;
                }
                let (mut sum, mut n) = ([0u32; 3], 0u32);
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && nx < w && ny < h && known[(ny * w + nx) as usize] {
                        n += 1;
                        for c in 0..3 {
                            sum[c] += level.rgb[(ny * w + nx) as usize * 3 + c] as u32;
                        }
                    }
                }
                if n > 0 {
                    ring.push((i, [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]));
                }
            }
        }
        if ring.is_empty() {
            break;
        }
        for (i, color) in ring {
            level.rgb[i * 3..i * 3 + 3].copy_from_slice(&color);
            known[i] = true;
        }
    }
}

/// One size of the pyramid: improve the matches, then repaint the hole from them, a few times over.
fn refine(level: &mut Level, matches: &mut [(i32, i32)], targets: &[(i32, i32)], sources: &[(i32, i32)], passes: usize, rng: &mut Rng) {
    let (w, h) = (level.w, level.h);
    let reach = w.max(h);
    for pass in 0..passes {
        // Search: each target patch tries its neighbours' matches, shifted along, then random patches ever closer to its best.
        let backwards = pass % 2 == 1;
        let step = if backwards { 1 } else { -1 };
        let order: Box<dyn Iterator<Item = &(i32, i32)>> = if backwards { Box::new(targets.iter().rev()) } else { Box::new(targets.iter()) };
        for &(x, y) in order {
            let i = (y * w + x) as usize;
            let (mut best, mut best_d) = (matches[i], u32::MAX);
            best_d = level.distance(x, y, best.0, best.1, best_d);
            let consider = |qx: i32, qy: i32, best: &mut (i32, i32), best_d: &mut u32| {
                if qx < R || qy < R || qx >= w - R || qy >= h - R || !level.source[(qy * w + qx) as usize] {
                    return;
                }
                let d = level.distance(x, y, qx, qy, *best_d);
                if d < *best_d {
                    (*best, *best_d) = ((qx, qy), d);
                }
            };
            for (dx, dy) in [(step, 0), (0, step)] {
                let (nx, ny) = (x + dx, y + dy);
                if nx >= 0 && ny >= 0 && nx < w && ny < h {
                    let m = matches[(ny * w + nx) as usize];
                    if m.0 >= 0 {
                        consider(m.0 - dx, m.1 - dy, &mut best, &mut best_d);
                    }
                }
            }
            let mut radius = reach;
            while radius >= 1 {
                consider(best.0 + rng.range(-radius, radius + 1), best.1 + rng.range(-radius, radius + 1), &mut best, &mut best_d);
                radius /= 2;
            }
            // Once in a while, anywhere at all: keeps a bad start from sticking.
            let anywhere = sources[rng.range(0, sources.len() as i32) as usize];
            consider(anywhere.0, anywhere.1, &mut best, &mut best_d);
            matches[i] = best;
        }

        // Vote: every missing pixel is the average of what each patch over it would put there.
        let mut sum = vec![[0u32; 4]; (w * h) as usize];
        for &(x, y) in targets {
            let (qx, qy) = matches[(y * w + x) as usize];
            for dy in -R..=R {
                for dx in -R..=R {
                    let (tx, ty) = (x + dx, y + dy);
                    if tx < 0 || ty < 0 || tx >= w || ty >= h || !level.hole[(ty * w + tx) as usize] {
                        continue;
                    }
                    let s = level.at(qx + dx, qy + dy) * 3;
                    let cell = &mut sum[(ty * w + tx) as usize];
                    for c in 0..3 {
                        cell[c] += level.rgb[s + c] as u32;
                    }
                    cell[3] += 1;
                }
            }
        }
        for (i, cell) in sum.iter().enumerate() {
            if cell[3] > 0 {
                for c in 0..3 {
                    level.rgb[i * 3 + c] = ((cell[c] + cell[3] / 2) / cell[3]) as u8;
                }
            }
        }
    }
}

/// Fill the missing pixels of an RGB image (three bytes a pixel) in place.
/// `hole[i]` says pixel `i` is missing. With nothing known, or nothing
/// missing, the image is left as it is.
pub fn inpaint(width: u32, height: u32, rgb: &mut [u8], hole: &[bool], seed: u64) {
    assert_eq!(rgb.len(), (width * height * 3) as usize);
    assert_eq!(hole.len(), (width * height) as usize);
    if !hole.iter().any(|m| *m) || hole.iter().all(|m| *m) {
        return;
    }
    let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);

    // The pyramid, largest first. It stops shrinking when the image gets tiny or would have no whole patch left to copy from.
    let mut levels = vec![Level::new(width as i32, height as i32, rgb.to_vec(), hole.to_vec())];
    loop {
        let last = levels.last().expect("there is always one level");
        if last.w.min(last.h) <= 4 * R + 2 {
            break;
        }
        let next = last.half();
        if !next.source.iter().any(|s| *s) {
            break;
        }
        levels.push(next);
    }

    let mut previous: Option<(i32, Vec<(i32, i32)>)> = None;
    let mut smaller: Option<Level> = None;
    for (depth, mut level) in levels.into_iter().enumerate().rev() {
        let (w, h) = (level.w, level.h);
        // Start from the smaller size's picture, or from colors spread in from the hole's edge.
        match &smaller {
            Some(small) => {
                for y in 0..h {
                    for x in 0..w {
                        let i = (y * w + x) as usize;
                        if level.hole[i] {
                            let s = small.at(x / 2, y / 2) * 3;
                            level.rgb[i * 3..i * 3 + 3].copy_from_slice(&small.rgb[s..s + 3]);
                        }
                    }
                }
            }
            None => spread(&mut level),
        }
        let sources: Vec<(i32, i32)> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|&(x, y)| level.source[(y * w + x) as usize]).collect();
        if sources.is_empty() {
            // Nowhere to copy from at this size: the spread colors are the fill.
            smaller = Some(level);
            continue;
        }
        // Patches to fill: every one that covers a missing pixel.
        let mut near = vec![false; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                if level.hole[(y * w + x) as usize] {
                    for ty in (y - R).max(0)..=(y + R).min(h - 1) {
                        for tx in (x - R).max(0)..=(x + R).min(w - 1) {
                            near[(ty * w + tx) as usize] = true;
                        }
                    }
                }
            }
        }
        let targets: Vec<(i32, i32)> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|&(x, y)| near[(y * w + x) as usize]).collect();
        // Matches start where the smaller size found them, twice as far out; otherwise anywhere.
        let mut matches = vec![(-1, -1); (w * h) as usize];
        for &(x, y) in &targets {
            let guess = previous.as_ref().and_then(|(pw, m)| {
                let (sx, sy) = (x / 2, y / 2);
                let from = *m.get((sy * pw + sx) as usize)?;
                let q = (from.0 * 2 + x % 2, from.1 * 2 + y % 2);
                (from.0 >= 0 && q.0 >= R && q.1 >= R && q.0 < w - R && q.1 < h - R && level.source[(q.1 * w + q.0) as usize]).then_some(q)
            });
            matches[(y * w + x) as usize] = guess.unwrap_or_else(|| sources[rng.range(0, sources.len() as i32) as usize]);
        }
        // More passes at the small sizes, where they are cheap and settle the large shapes.
        let passes = if depth == 0 { 2 } else if depth == 1 { 3 } else { 5 };
        refine(&mut level, &mut matches, &targets, &sources, passes, &mut rng);
        previous = Some((w, matches));
        smaller = Some(level);
    }
    if let Some(done) = smaller {
        rgb.copy_from_slice(&done.rgb);
    }
}
