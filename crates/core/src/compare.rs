//! Comparing a render with a reference image, for golden tests.
//!
//! Both images are compared premultiplied, so a fully transparent pixel
//! matches another whatever color it nominally has.

use crate::raster::Region;

/// Side of the blocks used to find where the images disagree most.
const BLOCK: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    /// Peak signal-to-noise ratio in dB over the compared pixels. Infinite when they are identical.
    pub psnr: f64,
    /// Largest difference in any channel, 0..255.
    pub max_diff: u8,
    /// Pixels that differ by more than 1/255 in some channel.
    pub differing: usize,
    /// Pixels compared (everything outside the ignored regions).
    pub compared: usize,
    /// The 64 px block with the highest mean error, and that error (0..255).
    pub worst_block: Region,
    pub worst_block_error: f64,
}

/// Compare two straight-alpha RGBA images of the same size, skipping the
/// pixels inside `ignore`. Returns the metrics and a heatmap (straight RGBA):
/// black where the images agree, through red to yellow where they differ most,
/// and dark blue where pixels were ignored.
pub fn compare(width: u32, height: u32, actual: &[u8], reference: &[u8], ignore: &[Region]) -> (Comparison, Vec<u8>) {
    let (w, h) = (width as usize, height as usize);
    assert_eq!(actual.len(), w * h * 4, "the render has the wrong size");
    assert_eq!(reference.len(), w * h * 4, "the reference has the wrong size");

    let mut skip = vec![false; w * h];
    let whole = Region::new(0, 0, width as i32, height as i32);
    for region in ignore {
        if let Some(r) = region.intersect(&whole) {
            for y in r.y..r.y + r.h {
                let start = whole.index(r.x, y);
                skip[start..start + r.w as usize].fill(true);
            }
        }
    }

    let (blocks_x, blocks_y) = (w.div_ceil(BLOCK), h.div_ceil(BLOCK));
    let mut block_error = vec![0f64; blocks_x * blocks_y];
    let mut block_count = vec![0usize; blocks_x * blocks_y];
    let mut heatmap = vec![0u8; w * h * 4];
    let (mut squared, mut compared, mut differing, mut max_diff) = (0f64, 0usize, 0usize, 0u8);

    for p in 0..w * h {
        let out = &mut heatmap[p * 4..p * 4 + 4];
        if skip[p] {
            out.copy_from_slice(&[20, 30, 70, 255]);
            continue;
        }
        let (a, r) = (&actual[p * 4..p * 4 + 4], &reference[p * 4..p * 4 + 4]);
        let premultiplied = |px: &[u8]| {
            let alpha = px[3] as f64 / 255.0;
            [px[0] as f64 * alpha, px[1] as f64 * alpha, px[2] as f64 * alpha, px[3] as f64]
        };
        let (pa, pr) = (premultiplied(a), premultiplied(r));
        let mut worst = 0f64;
        let mut sum = 0f64;
        for c in 0..4 {
            let d = (pa[c] - pr[c]).abs();
            squared += d * d;
            sum += d;
            worst = worst.max(d);
        }
        compared += 1;
        if worst > 1.0 {
            differing += 1;
        }
        max_diff = max_diff.max(worst.round() as u8);
        let block = (p / w / BLOCK) * blocks_x + (p % w) / BLOCK;
        block_error[block] += sum / 4.0;
        block_count[block] += 1;
        // Black -> red -> yellow as the error grows; small errors are amplified so they show.
        let t = (worst / 64.0).min(1.0).sqrt();
        out.copy_from_slice(&[(t * 2.0).min(1.0).mul_add(255.0, 0.0) as u8, ((t - 0.5).max(0.0) * 2.0 * 255.0) as u8, 0, 255]);
    }

    let (mut worst_block, mut worst_block_error) = (Region::new(0, 0, 0, 0), 0f64);
    for (i, (&error, &count)) in block_error.iter().zip(&block_count).enumerate() {
        // Blocks that are mostly ignored say little.
        if count * 4 < BLOCK * BLOCK {
            continue;
        }
        let mean = error / count as f64;
        if mean > worst_block_error {
            worst_block_error = mean;
            let (bx, by) = ((i % blocks_x) * BLOCK, (i / blocks_x) * BLOCK);
            worst_block = Region::new(bx as i32, by as i32, BLOCK.min(w - bx) as i32, BLOCK.min(h - by) as i32);
        }
    }
    let mse = if compared > 0 { squared / (compared * 4) as f64 } else { 0.0 };
    let psnr = if mse > 0.0 { 10.0 * (255.0f64 * 255.0 / mse).log10() } else { f64::INFINITY };
    (Comparison { psnr, max_diff, differing, compared, worst_block, worst_block_error }, heatmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        rgba.repeat((w * h) as usize)
    }

    #[test]
    fn identical_images_have_infinite_psnr() {
        let a = solid(100, 80, [10, 20, 30, 255]);
        let (c, heat) = compare(100, 80, &a, &a, &[]);
        assert!(c.psnr.is_infinite());
        assert_eq!((c.max_diff, c.differing, c.compared, c.worst_block_error), (0, 0, 8000, 0.0));
        assert!(heat.chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
    }

    #[test]
    fn transparent_pixels_match_whatever_their_color() {
        let (c, _) = compare(4, 4, &solid(4, 4, [255, 0, 0, 0]), &solid(4, 4, [0, 0, 255, 0]), &[]);
        assert!(c.psnr.is_infinite());
    }

    #[test]
    fn a_uniform_error_gives_the_textbook_psnr() {
        // Every channel off by 10: MSE over RGB is 100, and alpha is equal, so MSE = 75.
        let (c, _) = compare(64, 64, &solid(64, 64, [100, 100, 100, 255]), &solid(64, 64, [110, 110, 110, 255]), &[]);
        assert!((c.psnr - 10.0 * (255.0f64 * 255.0 / 75.0).log10()).abs() < 1e-9);
        assert_eq!((c.max_diff, c.differing), (10, 64 * 64));
        assert!((c.worst_block_error - 7.5).abs() < 1e-9);
    }

    #[test]
    fn the_worst_block_is_found_and_ignored_regions_are_skipped() {
        let mut actual = solid(200, 200, [50, 50, 50, 255]);
        let reference = actual.clone();
        // A bright patch at (130..150, 70..90): inside the block at (128, 64).
        for y in 70..90 {
            for x in 130..150 {
                let i = (y * 200 + x) * 4;
                actual[i..i + 3].copy_from_slice(&[250, 250, 250]);
            }
        }
        let (c, heat) = compare(200, 200, &actual, &reference, &[]);
        assert_eq!(c.worst_block, Region::new(128, 64, 64, 64));
        assert_eq!((c.differing, c.max_diff), (400, 200));
        assert_eq!(&heat[(80 * 200 + 140) * 4..(80 * 200 + 140) * 4 + 3], [255, 255, 0]);

        let (ignored, heat) = compare(200, 200, &actual, &reference, &[Region::new(125, 65, 30, 30)]);
        assert!(ignored.psnr.is_infinite());
        assert_eq!(ignored.compared, 200 * 200 - 900);
        assert_eq!(heat[(80 * 200 + 140) * 4..(80 * 200 + 140) * 4 + 4], [20, 30, 70, 255]);
    }
}
