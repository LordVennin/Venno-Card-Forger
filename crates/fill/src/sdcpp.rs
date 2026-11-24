//! Tier 3: Stable Diffusion inpainting, through stable-diffusion.cpp.
//!
//! This is the equivalent of Photoshop's Generative Fill: a diffusion model
//! paints the missing area, optionally following a description. It needs a
//! graphics card (about 4 GB of memory for SD 1.5 at 512 x 512), a 4 GB model
//! file, and the `sd-cli` program from the stable-diffusion.cpp project, none
//! of which ship with the app. The program is run as a separate process, so a
//! crash or an out-of-memory failure there cannot take the app down: this tier
//! reports the failure and the next tier is used.
//!
//! `sd-cli` does inpainting by running a script in its own small language
//! (DGL) that names the model, the picture, the mask and the settings; it has
//! no command-line options for those. So for each fill a script is written
//! beside the picture and the mask, and `sd-cli -M dgl --script <file>` is
//! run. The script is the project's own SD 1.5 inpainting example
//! (docs/dgl/sd/sd1-5-inpainting.dgl, MIT license) with our values filled in.
//!
//! Like the LaMa tier, the model works on a 512 x 512 picture, so its answer
//! guides PatchMatch, which fills at the working size.
//!
//! Run once against the real program, on the owner's machine: a borderless
//! card filled in 18.5 seconds. The tests here use a stand-in program, since
//! they cannot assume a graphics card. Its memory use, and what `sd-cli` does
//! when it runs out, are still not known. See `docs/TIER3.md`.

use crate::{Availability, FillProvider, FillRequest, Filled};
use std::path::PathBuf;

/// The model this tier was written for, and where it is published.
pub const MODEL_URL: &str = "https://huggingface.co/stable-diffusion-v1-5/stable-diffusion-inpainting/resolve/main/sd-v1-5-inpainting.ckpt";
pub const MODEL_BYTES: u64 = 4_265_437_280;
pub const MODEL_FILE: &str = "sd-v1-5-inpainting.ckpt";

/// The side of the square picture the model is run on.
#[cfg(feature = "sdcpp")]
const SIDE: u32 = 512;

/// What is asked for when the user gives no description.
pub const DEFAULT_PROMPT: &str = "seamless continuation of the picture, background, same style and lighting";
pub const NEGATIVE_PROMPT: &str = "text, watermark, signature, frame, border, logo";

pub struct SdCpp {
    /// The `sd-cli` program.
    pub program: PathBuf,
    /// An SD 1.5 inpainting checkpoint.
    pub model: PathBuf,
    /// A folder for the files of a run. They are left there afterwards, so a failed run can be looked at.
    pub scratch: PathBuf,
    /// Width of the picture PatchMatch finishes at.
    pub work_width: u32,
    pub steps: u32,
    pub cfg_scale: f64,
    /// Extra arguments for `sd-cli`, such as `--max-vram 6` on a small card.
    pub extra_args: Vec<String>,
    /// How long the program may run before it is stopped.
    pub timeout: std::time::Duration,
}

impl SdCpp {
    pub fn new(program: impl Into<PathBuf>, model: impl Into<PathBuf>, scratch: impl Into<PathBuf>, work_width: u32) -> SdCpp {
        SdCpp { program: program.into(), model: model.into(), scratch: scratch.into(), work_width, steps: 20, cfg_scale: 7.0, extra_args: Vec::new(), timeout: std::time::Duration::from_secs(600) }
    }
}

/// A string as the script language writes one.
#[cfg_attr(not(feature = "sdcpp"), allow(dead_code))]
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"").replace(['\n', '\r'], " "))
}

/// The inpainting script for one run. White in the mask is repainted.
#[cfg_attr(not(feature = "sdcpp"), allow(dead_code))]
pub(crate) fn script(model: &str, input: &str, mask: &str, output: &str, prompt: &str, seed: u64, steps: u32, cfg_scale: f64, side: u32) -> String {
    format!(
        r#"# version: 0.0.0
# Written by Cardsmith for one fill. Adapted from stable-diffusion.cpp's
# docs/dgl/sd/sd1-5-inpainting.dgl (MIT license).

var {{
    weights = {weights}

    input_path = {input}
    mask_path = {mask}
    output_path = {output}

    prompt = {prompt}
    negative_prompt = {negative}

    seed = {seed}

    strength = 1.0
    steps = {steps}
    cfg_scale = {cfg_scale}
    scheduler = "karras"
}}

fn latent_mask(mask, like) {{
    shape = tensor::shape(like)
    w = shape[0]
    h = shape[1]
    batch = shape[3]
    resized = tensor::interpolate(image::to_f32(mask), size = [w, h], mode = "nearest")
    return tensor::broadcast_to(resized, [w, h, 1, batch])
}}

fn slice_sigmas_by_strength(sigmas, strength) {{
    steps = tensor::numel(sigmas) - 1
    retained = math::floor(steps * strength)
    return sigmas[steps - retained:]
}}

fn main() {{
    unet = load_unet(weights)
    text = load_text_encoder(weights)
    vae = load_vae(weights)
    source = tensor::interpolate(image::load(input_path), size = [{side}, {side}], axes = [1, 2])
    gray = tensor::grayscale(image::load(mask_path), axis = 0)
    binary_mask = tensor::threshold(gray, threshold = 128, low = 0, high = 255)
    region = tensor::interpolate(binary_mask, size = [{side}, {side}], axes = [1, 2], mode = "nearest_exact")
    pixels = image::to_f32(source)
    masked = tensor::lerp(pixels, 0.5, image::to_f32(region))
    encode_rng = make_rng("cuda", seed = seed)
    base = vae_to_unet_latents(vae, vae_encode(vae, pixels, rng = encode_rng))
    cond_image = vae_to_unet_latents(vae, vae_encode(vae, masked, rng = encode_rng))
    region_latent = latent_mask(region, like = base)
    condition = tensor::concat([region_latent, cond_image], axis = 2)
    denoise_mask = tensor::max_pool2d(region_latent, kernel = [3, 3], padding_before = [1, 1])
    rng = make_rng("cuda", seed = seed)
    pos = predictor(unet, encode_prompt(text, prompt), concat = condition, prediction = "eps")
    neg = predictor(unet, encode_prompt(text, negative_prompt), concat = condition, prediction = "eps")
    guided = cfg(pos, neg, scale = cfg_scale)
    sigmas = slice_sigmas_by_strength(make_sigmas(guided, latent = base, scheduler = scheduler, steps = steps), strength = strength)
    start = add_noise(guided, base, tensor::randn_like(base, rng = rng), sigma = sigmas[0])
    latent = sample(start, sigmas, predict = guided, base = base, denoise_mask = denoise_mask, rng = rng)
    image = vae_decode(vae, unet_to_vae_latents(vae, latent))
    image::save(output_path, image)
    return {{ image = image, latent = latent, mask = denoise_mask }}
}}
"#,
        weights = quoted(model),
        input = quoted(input),
        mask = quoted(mask),
        output = quoted(output),
        prompt = quoted(prompt),
        negative = quoted(NEGATIVE_PROMPT),
    )
}

#[cfg(feature = "sdcpp")]
fn write_png(path: &std::path::Path, side: u32, rgb: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), side, side);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut w| w.write_image_data(rgb)).map_err(|e| format!("{}: {e}", path.display()))
}

/// Read a PNG as RGB, three bytes a pixel.
#[cfg(feature = "sdcpp")]
fn read_png(path: &std::path::Path) -> Result<(u32, u32, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("{}: {e}", path.display()))?;
    let mut data = vec![0; reader.output_buffer_size().ok_or_else(|| format!("{}: too large", path.display()))?];
    let info = reader.next_frame(&mut data).map_err(|e| format!("{}: {e}", path.display()))?;
    let channels = info.color_type.samples();
    let rgb = data[..info.buffer_size()].chunks_exact(channels).flat_map(|p| if channels >= 3 { [p[0], p[1], p[2]] } else { [p[0], p[0], p[0]] }).collect();
    Ok((info.width, info.height, rgb))
}

impl FillProvider for SdCpp {
    fn id(&self) -> &'static str {
        "sdcpp"
    }

    fn available(&self) -> Availability {
        let no = |reason: String| Availability { ok: false, reason: Some(reason), vram_mb: Some(4000) };
        if !cfg!(feature = "sdcpp") {
            return no("this build was made without stable-diffusion.cpp support".into());
        }
        if !self.program.is_file() {
            return no(format!("the sd-cli program of stable-diffusion.cpp was not found ({} is missing)", self.program.display()));
        }
        if !self.model.is_file() {
            return no(format!("its model has not been downloaded ({} is missing, {:.1} GB)", self.model.display(), MODEL_BYTES as f64 / 1e9));
        }
        Availability { ok: true, reason: None, vram_mb: Some(4000) }
    }

    #[cfg(not(feature = "sdcpp"))]
    fn fill(&mut self, _: &FillRequest) -> Result<Option<Filled>, String> {
        Err("this build was made without stable-diffusion.cpp support".into())
    }

    #[cfg(feature = "sdcpp")]
    fn fill(&mut self, request: &FillRequest) -> Result<Option<Filled>, String> {
        use crate::patchmatch::{inpaint, Guide};

        let mut work = crate::working(&request.art, request.canvas, request.keep, self.work_width);
        if work.nothing_to_do() {
            return Ok(None);
        }
        // The card area, shrunk to fit the model's square and centered in it; missing parts are mid-gray in the picture and white in the mask.
        let (w, h) = (work.width, work.height);
        let shrink = SIDE as f64 / w.max(h) as f64;
        let (gw, gh) = (((w as f64 * shrink).round() as u32).clamp(1, SIDE), ((h as f64 * shrink).round() as u32).clamp(1, SIDE));
        let (ox, oy) = ((SIDE - gw) / 2, (SIDE - gh) / 2);
        let mut picture = vec![128u8; (SIDE * SIDE * 3) as usize];
        let mut mask = vec![255u8; (SIDE * SIDE * 3) as usize];
        for y in 0..gh {
            for x in 0..gw {
                let (sx, sy) = ((x * w / gw).min(w - 1), (y * h / gh).min(h - 1));
                // Known only if the whole block of working pixels under it is.
                let (x1, y1) = (((x + 1) * w / gw).clamp(sx + 1, w), ((y + 1) * h / gh).clamp(sy + 1, h));
                let known = (sy..y1).all(|yy| (sx..x1).all(|xx| !work.hole[(yy * w + xx) as usize]));
                if known {
                    let (i, o) = ((sy * w + sx) as usize * 3, ((oy + y) * SIDE + ox + x) as usize * 3);
                    picture[o..o + 3].copy_from_slice(&work.rgb[i..i + 3]);
                    mask[o..o + 3].copy_from_slice(&[0, 0, 0]);
                }
            }
        }

        std::fs::create_dir_all(&self.scratch).map_err(|e| format!("{}: {e}", self.scratch.display()))?;
        let scratch = self.scratch.canonicalize().map_err(|e| format!("{}: {e}", self.scratch.display()))?;
        let file = |name: &str| scratch.join(name);
        let _ = std::fs::remove_file(file("output.png"));
        write_png(&file("input.png"), SIDE, &picture)?;
        write_png(&file("mask.png"), SIDE, &mask)?;
        let text = |path: PathBuf| path.to_string_lossy().into_owned();
        let model = self.model.canonicalize().unwrap_or_else(|_| self.model.clone());
        let prompt = request.prompt.filter(|p| !p.trim().is_empty()).unwrap_or(DEFAULT_PROMPT);
        std::fs::write(file("fill.dgl"), script(&text(model), &text(file("input.png")), &text(file("mask.png")), &text(file("output.png")), prompt, request.seed, self.steps, self.cfg_scale, SIDE))
            .map_err(|e| format!("{}: {e}", file("fill.dgl").display()))?;

        // Run it, with its log going to a file, and stop it if it takes too long.
        let log = std::fs::File::create(file("log.txt")).map_err(|e| format!("{}: {e}", file("log.txt").display()))?;
        let mut child = std::process::Command::new(&self.program)
            .args(["-M", "dgl", "--script"])
            .arg(file("fill.dgl"))
            .args(&self.extra_args)
            .current_dir(&scratch)
            .stdin(std::process::Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .spawn()
            .map_err(|e| format!("{} could not be started: {e}", self.program.display()))?;
        let started = std::time::Instant::now();
        let status = loop {
            match child.try_wait().map_err(|e| e.to_string())? {
                Some(status) => break status,
                None if started.elapsed() > self.timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("sd-cli was stopped after {} s without finishing", self.timeout.as_secs()));
                }
                None => std::thread::sleep(std::time::Duration::from_millis(100)),
            }
        };
        // The end of the log is the best account of what went wrong.
        let tail = || {
            let log = std::fs::read_to_string(file("log.txt")).unwrap_or_default();
            let lines: Vec<&str> = log.lines().filter(|l| !l.trim().is_empty()).collect();
            lines[lines.len().saturating_sub(3)..].join(" | ")
        };
        if !status.success() {
            return Err(format!("sd-cli ended with {status}: {}", tail()));
        }
        let (rw, rh, result) = read_png(&file("output.png")).map_err(|e| format!("sd-cli made no picture ({e}): {}", tail()))?;
        if rw == 0 || rh == 0 {
            return Err("sd-cli made an empty picture".into());
        }
        // Take the card area back out of the square, whatever size it came back at.
        let (cw, ch) = ((gw * rw / SIDE).max(1), (gh * rh / SIDE).max(1));
        let (cx, cy) = (ox * rw / SIDE, oy * rh / SIDE);
        let mut guide = vec![0u8; (cw * ch * 3) as usize];
        for y in 0..ch {
            let from = (((cy + y).min(rh - 1) * rw + cx.min(rw - 1)) * 3) as usize;
            let len = (cw.min(rw - cx.min(rw - 1)) * 3) as usize;
            guide[(y * cw * 3) as usize..(y * cw * 3) as usize + len].copy_from_slice(&result[from..from + len]);
        }
        inpaint(work.width, work.height, &mut work.rgb, &work.hole, request.seed, Some(&Guide { width: cw, height: ch, rgb: &guide }));
        Ok(Some(work.finish()))
    }
}
