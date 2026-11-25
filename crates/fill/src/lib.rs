//! Filling the empty space around a card's art.
//!
//! Art is rarely the shape of the card. On a full-art or extended frame the
//! picture stops short and the rest has to be made up so it looks as if the
//! picture simply carried on. Proxyshop hands that to Photoshop's
//! Content-Aware Fill; this crate does the same job itself.
//!
//! The fill never touches the art (rule 4): it is a separate image, made at a
//! reduced size, that the card engine puts on its own layer underneath.

pub mod lama;
pub mod patchmatch;
pub mod sdcpp;
#[cfg(all(feature = "onnx", feature = "sdcpp"))]
pub mod tiers;

/// Art as it sits on a card: straight-alpha RGBA pixels and where they go.
pub struct Placed<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
    /// Document position of the art's top-left corner and its scale.
    pub x: f64,
    pub y: f64,
    pub scale: f64,
}

/// A finished fill: an RGB picture of the whole card area at a reduced size.
pub struct Filled {
    pub width: u32,
    pub height: u32,
    /// Straight-alpha RGBA, fully opaque.
    pub rgba: Vec<u8>,
    /// How many document pixels one pixel of the fill covers.
    pub scale: f64,
}

/// The card area at working size: the trusted art, shrunk, and a map of what is missing.
pub(crate) struct Working {
    pub width: u32,
    pub height: u32,
    /// Three bytes a pixel.
    pub rgb: Vec<u8>,
    pub hole: Vec<bool>,
    /// How many document pixels one working pixel covers.
    pub scale: f64,
}

/// Shrink the card area to about `work_width` across. Pixels inside `keep`
/// that the art fully covers are known; everything else is missing.
pub(crate) fn working(art: &Placed, canvas: (u32, u32), keep: [f64; 4], work_width: u32) -> Working {
    let scale = (canvas.0 as f64 / work_width.max(16) as f64).max(1.0);
    let (w, h) = ((canvas.0 as f64 / scale).ceil() as u32, (canvas.1 as f64 / scale).ceil() as u32);
    let mut rgb = vec![0u8; (w * h * 3) as usize];
    let mut hole = vec![true; (w * h) as usize];
    // Each working pixel is the average of the art under it, where it lies inside the kept area and the art is opaque there.
    let per_art = scale / art.scale;
    let taps = (per_art.ceil() as i32).clamp(1, 8);
    for y in 0..h {
        for x in 0..w {
            let (cx, cy) = ((x as f64 + 0.5) * scale, (y as f64 + 0.5) * scale);
            if cx < keep[0] || cy < keep[1] || cx >= keep[2] || cy >= keep[3] {
                continue;
            }
            let (mut sum, mut n, mut clear) = ([0u32; 3], 0u32, false);
            for ty in 0..taps {
                for tx in 0..taps {
                    // Sample points spread evenly over the working pixel's footprint.
                    let dx = ((x as f64 + (tx as f64 + 0.5) / taps as f64) * scale - art.x) / art.scale;
                    let dy = ((y as f64 + (ty as f64 + 0.5) / taps as f64) * scale - art.y) / art.scale;
                    if dx < 0.0 || dy < 0.0 || dx >= art.width as f64 || dy >= art.height as f64 {
                        clear = true;
                        continue;
                    }
                    let i = ((dy as u32 * art.width + dx as u32) * 4) as usize;
                    if art.rgba[i + 3] < 250 {
                        clear = true;
                        continue;
                    }
                    for c in 0..3 {
                        sum[c] += art.rgba[i + c] as u32;
                    }
                    n += 1;
                }
            }
            // A pixel the art only partly covers is not trusted.
            if n > 0 && !clear {
                let o = (y * w + x) as usize;
                hole[o] = false;
                for c in 0..3 {
                    rgb[o * 3 + c] = (sum[c] / n) as u8;
                }
            }
        }
    }
    Working { width: w, height: h, rgb, hole, scale }
}

impl Working {
    /// Nothing is missing, or nothing is known: there is no fill to make.
    pub fn nothing_to_do(&self) -> bool {
        !self.hole.iter().any(|m| *m) || self.hole.iter().all(|m| *m)
    }

    pub fn finish(self) -> Filled {
        Filled { width: self.width, height: self.height, rgba: self.rgb.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(), scale: self.scale }
    }
}

/// Fill everything on a `canvas` (width, height in document pixels) outside
/// `keep` (left, top, right, bottom: the part of the art that is trusted) by
/// PatchMatch, working at about `work_width` pixels across.
///
/// The result covers the whole canvas: the art, shrunk, inside `keep`, and
/// made-up picture everywhere else. Returns None when there is nothing to
/// fill or nothing to fill from.
pub fn fill_edges(art: &Placed, canvas: (u32, u32), keep: [f64; 4], work_width: u32, seed: u64) -> Option<Filled> {
    let mut work = working(art, canvas, keep, work_width);
    if work.nothing_to_do() {
        return None;
    }
    patchmatch::inpaint(work.width, work.height, &mut work.rgb, &work.hole, seed, None);
    Some(work.finish())
}

// ---- providers and falling back ---------------------------------------------------

/// What a provider is asked to do.
pub struct FillRequest<'a> {
    pub art: Placed<'a>,
    /// Document size in pixels.
    pub canvas: (u32, u32),
    /// The trusted part of the art (left, top, right, bottom); everything outside it is filled.
    pub keep: [f64; 4],
    /// The same seed gives the same fill; another seed gives a variation.
    pub seed: u64,
    /// What should be there, for providers that take a description.
    pub prompt: Option<&'a str>,
}

/// Whether a provider can run here, and if not, why (in words for the user).
#[derive(Clone, Debug, PartialEq)]
pub struct Availability {
    pub ok: bool,
    pub reason: Option<String>,
    /// Graphics memory it wants, when it wants any.
    pub vram_mb: Option<u32>,
}

/// One way of making a fill. Every provider returns the same thing, a picture
/// of the whole card area, so the layer made from it is the same whichever
/// was used.
pub trait FillProvider {
    /// "patchmatch", "lama", "sdcpp", "comfyui".
    fn id(&self) -> &'static str;
    fn available(&self) -> Availability;
    /// None means there was nothing to fill.
    fn fill(&mut self, request: &FillRequest) -> Result<Option<Filled>, String>;
}

/// Tier 1: PatchMatch on the CPU. Always available.
pub struct PatchMatch {
    /// Width of the picture it works on. The fill is that size, stretched over the card.
    pub work_width: u32,
}

impl FillProvider for PatchMatch {
    fn id(&self) -> &'static str {
        "patchmatch"
    }

    fn available(&self) -> Availability {
        Availability { ok: true, reason: None, vram_mb: None }
    }

    fn fill(&mut self, request: &FillRequest) -> Result<Option<Filled>, String> {
        Ok(fill_edges(&request.art, request.canvas, request.keep, self.work_width, request.seed))
    }
}

/// The outcome of trying providers in order.
pub struct Outcome {
    /// None when there was nothing to fill.
    pub filled: Option<Filled>,
    /// The provider that made it.
    pub provider: &'static str,
    /// One line for each provider that was passed over or failed, saying why.
    pub notices: Vec<String>,
}

/// Try each provider in turn, best first, and use the first that works. A
/// provider that is not available, or fails, is passed over with a notice,
/// as Proxyshop falls back from Generative Fill to Content-Aware Fill.
pub fn fill_with_fallback(providers: &mut [&mut dyn FillProvider], request: &FillRequest) -> Result<Outcome, String> {
    let mut notices = Vec::new();
    for provider in providers.iter_mut() {
        let state = provider.available();
        if !state.ok {
            notices.push(format!("{} is not available: {}", provider.id(), state.reason.unwrap_or_else(|| "no reason given".into())));
            continue;
        }
        match provider.fill(request) {
            Ok(filled) => return Ok(Outcome { filled, provider: provider.id(), notices }),
            Err(why) => notices.push(format!("{} failed: {why}", provider.id())),
        }
    }
    Err(if notices.is_empty() { "There is no fill provider to try.".into() } else { notices.join("; ") })
}

/// A patch that paints over part of a picture.
pub struct Patch {
    /// Where its top-left corner goes in the picture, in the picture's pixels.
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Straight-alpha RGBA, fully opaque.
    pub rgba: Vec<u8>,
    /// How many of the picture's pixels one pixel of the patch covers.
    pub scale: f64,
}

/// The surroundings of a rectangle of a picture, with the rectangle itself
/// cut out, ready to be filled: the picture to give a provider, where it was
/// cut from, and how wide to work. The surroundings reach as far out as the
/// rectangle is big, so there is as much to copy from as there is to fill.
pub struct Cutout {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// The surroundings' top-left corner in the picture.
    pub origin: (u32, u32),
    /// The rectangle, within the surroundings: left, top, right, bottom.
    pub hole: [u32; 4],
}

/// Cut `hole` (left, top, right, bottom, in the picture's pixels) out of a
/// picture. None if the rectangle is empty or covers the whole picture.
pub fn cut_out(width: u32, height: u32, rgba: &[u8], hole: [f64; 4]) -> Option<Cutout> {
    let clamp = |v: f64, max: u32| v.round().clamp(0.0, max as f64) as u32;
    let (x0, y0, x1, y1) = (clamp(hole[0], width), clamp(hole[1], height), clamp(hole[2], width), clamp(hole[3], height));
    if x1 <= x0 || y1 <= y0 || (x0 == 0 && y0 == 0 && x1 == width && y1 == height) {
        return None;
    }
    let margin = (x1 - x0).max(y1 - y0).max(64);
    let (cx0, cy0) = (x0.saturating_sub(margin), y0.saturating_sub(margin));
    let (cx1, cy1) = ((x1 + margin).min(width), (y1 + margin).min(height));
    let (w, h) = (cx1 - cx0, cy1 - cy0);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in cy0..cy1 {
        let row = ((y * width + cx0) * 4) as usize;
        out.extend_from_slice(&rgba[row..row + (w * 4) as usize]);
    }
    for y in y0..y1 {
        for x in x0..x1 {
            out[(((y - cy0) * w + (x - cx0)) * 4 + 3) as usize] = 0;
        }
    }
    Some(Cutout { rgba: out, width: w, height: h, origin: (cx0, cy0), hole: [x0 - cx0, y0 - cy0, x1 - cx0, y1 - cy0] })
}

impl Cutout {
    /// The request that fills the rectangle from its surroundings.
    pub fn request<'a>(&'a self, seed: u64, prompt: Option<&'a str>) -> FillRequest<'a> {
        let art = Placed { width: self.width, height: self.height, rgba: &self.rgba, x: 0.0, y: 0.0, scale: 1.0 };
        FillRequest { art, canvas: (self.width, self.height), keep: [0.0, 0.0, self.width as f64, self.height as f64], seed, prompt }
    }

    /// How wide to work: the surroundings' own size, up to a limit that keeps it quick.
    pub fn work_width(&self) -> u32 {
        self.width.min(1024)
    }

    /// The part of a finished fill that covers the rectangle, as a patch for the picture.
    pub fn patch(&self, filled: &Filled) -> Patch {
        let s = filled.scale;
        let (x0, y0) = ((self.hole[0] as f64 / s).floor() as u32, (self.hole[1] as f64 / s).floor() as u32);
        let (x1, y1) = (((self.hole[2] as f64 / s).ceil() as u32).min(filled.width), ((self.hole[3] as f64 / s).ceil() as u32).min(filled.height));
        let (w, h) = (x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1));
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in y0..y0 + h {
            let row = ((y.min(filled.height - 1) * filled.width + x0) * 4) as usize;
            rgba.extend_from_slice(&filled.rgba[row..row + (w * 4) as usize]);
        }
        Patch { x: self.origin.0 + (x0 as f64 * s).round() as u32, y: self.origin.1 + (y0 as f64 * s).round() as u32, width: w, height: h, rgba, scale: s }
    }
}
