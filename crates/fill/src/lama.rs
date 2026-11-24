//! Tier 2: LaMa, a small neural network that fills a hole with what belongs
//! there, where PatchMatch can only copy what is elsewhere in the picture.
//!
//! LaMa ("Resolution-robust Large Mask Inpainting with Fourier Convolutions",
//! Suvorov et al., 2021; Apache-2.0) is run through ONNX Runtime from a model
//! file, on the CPU: no graphics card is needed. The model works on a
//! 512 x 512 picture, so its answer is right in its shapes but soft. It is
//! therefore used as a guide: PatchMatch then fills at the usual working size,
//! starting from LaMa's picture and adding texture from the art.
//!
//! Neither the model (208 MB) nor ONNX Runtime ships with the app. Both are
//! files fetched when this tier is chosen; until they are there the tier says
//! it is not available and the next one is used. Without the `onnx` feature
//! this tier is never available.

use crate::{Availability, FillProvider, FillRequest, Filled};
use std::path::PathBuf;

/// Where the published model comes from, how large it is and what it must hash to.
pub const MODEL_URL: &str = "https://huggingface.co/Carve/LaMa-ONNX/resolve/main/lama_fp32.onnx";
pub const MODEL_BYTES: u64 = 208_044_816;
pub const MODEL_SHA256: &str = "1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6";
/// The file name the model is kept under.
pub const MODEL_FILE: &str = "lama_fp32.onnx";

/// The side of the square picture the model takes.
#[cfg(feature = "onnx")]
const SIDE: usize = 512;

pub struct Lama {
    /// The ONNX Runtime library file (libonnxruntime.so, onnxruntime.dll, libonnxruntime.dylib).
    pub runtime: PathBuf,
    /// The model file.
    pub model: PathBuf,
    /// Width of the picture PatchMatch finishes at.
    pub work_width: u32,
    #[cfg(feature = "onnx")]
    loaded: Option<(ort::environment::Environment, ort::session::Session)>,
}

impl Lama {
    pub fn new(runtime: impl Into<PathBuf>, model: impl Into<PathBuf>, work_width: u32) -> Lama {
        Lama {
            runtime: runtime.into(),
            model: model.into(),
            work_width,
            #[cfg(feature = "onnx")]
            loaded: None,
        }
    }
}

impl FillProvider for Lama {
    fn id(&self) -> &'static str {
        "lama"
    }

    fn available(&self) -> Availability {
        let no = |reason: String| Availability { ok: false, reason: Some(reason), vram_mb: None };
        if !cfg!(feature = "onnx") {
            return no("this build was made without ONNX Runtime support".into());
        }
        if !self.runtime.is_file() {
            return no(format!("ONNX Runtime has not been downloaded ({} is missing)", self.runtime.display()));
        }
        if !self.model.is_file() {
            return no(format!("its model has not been downloaded ({} is missing, {} MB)", self.model.display(), MODEL_BYTES / 1_000_000));
        }
        // It runs on the CPU, so it needs no graphics memory.
        Availability { ok: true, reason: None, vram_mb: Some(0) }
    }

    #[cfg(not(feature = "onnx"))]
    fn fill(&mut self, _: &FillRequest) -> Result<Option<Filled>, String> {
        Err("this build was made without ONNX Runtime support".into())
    }

    #[cfg(feature = "onnx")]
    fn fill(&mut self, request: &FillRequest) -> Result<Option<Filled>, String> {
        use crate::patchmatch::{inpaint, Guide};
        use ort::value::Tensor;

        let mut work = crate::working(&request.art, request.canvas, request.keep, self.work_width);
        if work.nothing_to_do() {
            return Ok(None);
        }
        if self.loaded.is_none() {
            let environment = ort::init_from(&self.runtime).map_err(|e| format!("ONNX Runtime could not be loaded: {e}"))?.build().map_err(|e| format!("ONNX Runtime could not be started: {e}"))?;
            let session = ort::session::Session::builder(&environment).and_then(|mut b| b.commit_from_file(&self.model)).map_err(|e| format!("the LaMa model could not be loaded: {e}"))?;
            self.loaded = Some((environment, session));
        }
        let (_, session) = self.loaded.as_mut().expect("just loaded");

        // The card area, shrunk to fit the model's square and centered in it. What is left over at the sides counts as missing.
        let (w, h) = (work.width as usize, work.height as usize);
        let shrink = SIDE as f64 / w.max(h) as f64;
        let (gw, gh) = (((w as f64 * shrink).round() as usize).clamp(1, SIDE), ((h as f64 * shrink).round() as usize).clamp(1, SIDE));
        let (ox, oy) = ((SIDE - gw) / 2, (SIDE - gh) / 2);
        let mut image = vec![0f32; 3 * SIDE * SIDE];
        let mut mask = vec![1f32; SIDE * SIDE];
        for y in 0..gh {
            for x in 0..gw {
                // Average the working pixels under this one; it is known only if all of them are.
                let (x0, x1) = (x * w / gw, ((x + 1) * w / gw).max(x * w / gw + 1).min(w));
                let (y0, y1) = (y * h / gh, ((y + 1) * h / gh).max(y * h / gh + 1).min(h));
                let (mut sum, mut n, mut missing) = ([0f32; 3], 0f32, false);
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let i = sy * w + sx;
                        if work.hole[i] {
                            missing = true;
                        } else {
                            for c in 0..3 {
                                sum[c] += work.rgb[i * 3 + c] as f32;
                            }
                            n += 1.0;
                        }
                    }
                }
                let o = (oy + y) * SIDE + ox + x;
                if !missing && n > 0.0 {
                    mask[o] = 0.0;
                    for c in 0..3 {
                        image[c * SIDE * SIDE + o] = sum[c] / n / 255.0;
                    }
                }
            }
        }

        let shape = |channels: usize| vec![1i64, channels as i64, SIDE as i64, SIDE as i64];
        let inputs = ort::inputs![
            "image" => Tensor::from_array((shape(3), image)).map_err(|e| e.to_string())?,
            "mask" => Tensor::from_array((shape(1), mask)).map_err(|e| e.to_string())?,
        ];
        let outputs = session.run(inputs).map_err(|e| format!("LaMa failed: {e}"))?;
        let (_, values) = outputs[0].try_extract_tensor::<f32>().map_err(|e| format!("LaMa's answer could not be read: {e}"))?;
        if values.len() != 3 * SIDE * SIDE {
            return Err(format!("LaMa answered with {} numbers where {} were expected", values.len(), 3 * SIDE * SIDE));
        }
        // Some exports answer in 0..1 and some in 0..255.
        let top = values.iter().copied().fold(0f32, f32::max);
        let gain = if top <= 2.0 { 255.0 } else { 1.0 };
        let mut guide = vec![0u8; gw * gh * 3];
        for y in 0..gh {
            for x in 0..gw {
                for c in 0..3 {
                    guide[(y * gw + x) * 3 + c] = (values[c * SIDE * SIDE + (oy + y) * SIDE + ox + x] * gain).clamp(0.0, 255.0) as u8;
                }
            }
        }
        inpaint(work.width, work.height, &mut work.rgb, &work.hole, request.seed, Some(&Guide { width: gw as u32, height: gh as u32, rgb: &guide }));
        Ok(Some(work.finish()))
    }
}
