//! PatchMatch on pictures whose right answer is known.

use cardsmith_fill::patchmatch::inpaint;
use cardsmith_fill::{fill_edges, Placed};

fn picture(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).flat_map(|(x, y)| f(x, y)).collect()
}

fn px(rgb: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * w + x) * 3) as usize;
    [rgb[i], rgb[i + 1], rgb[i + 2]]
}

/// A hole in the middle third, both ways.
fn middle(w: u32, h: u32) -> Vec<bool> {
    (0..h).flat_map(|y| (0..w).map(move |x| x >= w / 3 && x < 2 * w / 3 && y >= h / 3 && y < 2 * h / 3)).collect()
}

#[test]
fn a_flat_color_is_carried_across_the_hole_and_known_pixels_are_left_alone() {
    let (w, h) = (96, 96);
    let hole = middle(w, h);
    let mut rgb = picture(w, h, |x, y| if hole[(y * w + x) as usize] { [255, 0, 255] } else { [40, 90, 160] });
    inpaint(w, h, &mut rgb, &hole, 1);
    assert!(rgb.chunks_exact(3).all(|p| p == [40, 90, 160]));
}

#[test]
fn stripes_carry_on_through_the_hole() {
    // Vertical stripes 8 px wide. The fill should put the right stripe at (nearly) every missing pixel.
    let (w, h) = (128, 128);
    let stripe = |x: u32| if (x / 8) % 2 == 0 { [230, 230, 230] } else { [30, 30, 30] };
    let hole = middle(w, h);
    let mut rgb = picture(w, h, |x, y| if hole[(y * w + x) as usize] { [255, 0, 0] } else { stripe(x) });
    inpaint(w, h, &mut rgb, &hole, 7);
    let (mut missing, mut right) = (0, 0);
    for y in 0..h {
        for x in 0..w {
            if hole[(y * w + x) as usize] {
                missing += 1;
                let (got, want) = (px(&rgb, w, x, y), stripe(x));
                right += u32::from((got[0] as i32 - want[0] as i32).abs() < 60);
            } else {
                assert_eq!(px(&rgb, w, x, y), stripe(x));
            }
        }
    }
    assert!(right * 100 >= missing * 95, "{right} of {missing} missing pixels got the right stripe");
}

#[test]
fn the_same_seed_gives_the_same_fill_and_another_seed_another() {
    let (w, h) = (96, 80);
    let hole = middle(w, h);
    // A busy picture, so there is more than one plausible fill.
    let base = picture(w, h, |x, y| [((x * 37 + y * 11) % 251) as u8, ((x * 5 + y * 71) % 241) as u8, ((x * y) % 233) as u8]);
    let run = |seed: u64| {
        let mut rgb = base.clone();
        inpaint(w, h, &mut rgb, &hole, seed);
        rgb
    };
    assert_eq!(run(3), run(3));
    assert_ne!(run(3), run(4));
    // Nothing to do, or nothing to go on: left as it is.
    let mut same = base.clone();
    inpaint(w, h, &mut same, &vec![false; (w * h) as usize], 1);
    assert_eq!(same, base);
    inpaint(w, h, &mut same, &vec![true; (w * h) as usize], 1);
    assert_eq!(same, base);
}

#[test]
fn art_in_the_middle_of_a_card_is_extended_to_its_edges() {
    // A 100 x 60 picture of a sky-to-ground gradient, placed at twice its size in the middle of a 400 x 300 card.
    let art: Vec<u8> = (0..60u32).flat_map(|y| (0..100u32).map(move |_| y)).flat_map(|y| [(y * 4) as u8, 100, 255 - (y * 4) as u8, 255]).collect();
    let placed = Placed { width: 100, height: 60, rgba: &art, x: 100.0, y: 90.0, scale: 2.0 };
    let fill = fill_edges(&placed, (400, 300), [110.0, 100.0, 290.0, 200.0], 200, 1).unwrap();
    assert_eq!((fill.width, fill.height, fill.scale), (200, 150, 2.0));
    let at = |x: u32, y: u32| { let i = ((y * fill.width + x) * 4) as usize; [fill.rgba[i], fill.rgba[i + 1], fill.rgba[i + 2], fill.rgba[i + 3]] };
    // Inside the kept area: the art itself (card 200,150 is art row 30).
    let inside = at(100, 75);
    assert!((inside[0] as i32 - 120).abs() <= 6 && inside[3] == 255, "{inside:?}");
    // Left and right of it, at the same height, the same colors carry on.
    for x in [10, 190] {
        let beside = at(x, 75);
        assert!((beside[0] as i32 - 120).abs() <= 40 && beside[1] == 100, "{beside:?} at x {x}");
    }
    // Everything is opaque: the fill covers the whole card.
    assert!(fill.rgba.chunks_exact(4).all(|p| p[3] == 255));
    // Art that covers the whole card leaves nothing to fill.
    let whole = Placed { width: 100, height: 60, rgba: &art, x: 0.0, y: 0.0, scale: 5.0 };
    assert!(fill_edges(&whole, (400, 300), [0.0, 0.0, 400.0, 300.0], 200, 1).is_none());
}

#[test]
fn providers_are_tried_in_order_and_the_first_that_works_is_used() {
    use cardsmith_fill::{fill_with_fallback, Availability, FillProvider, FillRequest, Filled, PatchMatch};
    struct Missing;
    impl FillProvider for Missing {
        fn id(&self) -> &'static str { "lama" }
        fn available(&self) -> Availability { Availability { ok: false, reason: Some("its model has not been downloaded".into()), vram_mb: None } }
        fn fill(&mut self, _: &FillRequest) -> Result<Option<Filled>, String> { unreachable!() }
    }
    struct Broken;
    impl FillProvider for Broken {
        fn id(&self) -> &'static str { "sdcpp" }
        fn available(&self) -> Availability { Availability { ok: true, reason: None, vram_mb: Some(6000) } }
        fn fill(&mut self, _: &FillRequest) -> Result<Option<Filled>, String> { Err("out of memory".into()) }
    }
    let art: Vec<u8> = vec![90; 40 * 40 * 4].chunks_exact(4).flat_map(|_| [90, 120, 150, 255]).collect();
    let request = || FillRequest { art: Placed { width: 40, height: 40, rgba: &art, x: 30.0, y: 30.0, scale: 1.0 }, canvas: (100, 100), keep: [32.0, 32.0, 68.0, 68.0], seed: 1, prompt: None };
    let (mut missing, mut broken, mut patchmatch) = (Missing, Broken, PatchMatch { work_width: 100 });
    let outcome = fill_with_fallback(&mut [&mut missing, &mut broken, &mut patchmatch], &request()).unwrap();
    assert_eq!(outcome.provider, "patchmatch");
    assert_eq!(outcome.notices, ["lama is not available: its model has not been downloaded", "sdcpp failed: out of memory"]);
    let filled = outcome.filled.unwrap();
    assert_eq!((filled.width, filled.height), (100, 100));
    assert!(filled.rgba.chunks_exact(4).all(|p| p == [90, 120, 150, 255]));
    // With nothing left to fall back on, the reasons are the error.
    let error = fill_with_fallback(&mut [&mut Missing, &mut Broken], &request()).err().unwrap();
    assert!(error.contains("not been downloaded") && error.contains("out of memory"), "{error}");
}
