//! The fill tiers as they are installed on this machine.
//!
//! The command line and the app both fill the same way: from a folder that
//! holds the downloaded models and programs, try the tier that was asked for,
//! then each simpler one, and say which made the fill. That is here so the
//! two cannot drift apart.

use crate::lama::Lama;
use crate::sdcpp::SdCpp;
use crate::{fill_with_fallback, Availability, FillProvider, FillRequest, Outcome, PatchMatch};
use std::path::PathBuf;

/// What the ONNX Runtime library and the `sd-cli` program are called on this system.
pub const RUNTIME_FILE: &str = if cfg!(target_os = "windows") { "onnxruntime.dll" } else if cfg!(target_os = "macos") { "libonnxruntime.dylib" } else { "libonnxruntime.so" };
pub const SD_CLI_FILE: &str = if cfg!(target_os = "windows") { "sd-cli.exe" } else { "sd-cli" };

/// Where the tiers' files are. Everything defaults to a place under `cache`.
#[derive(Clone, Debug, Default)]
pub struct Tiers {
    /// The folder `cardsmith models` and `tools/install-tier3.sh` download into.
    pub cache: PathBuf,
    pub sd_program: Option<PathBuf>,
    pub sd_model: Option<PathBuf>,
    /// Extra arguments for `sd-cli`, for example `--max-vram 6`.
    pub sd_args: Vec<String>,
}

impl Tiers {
    pub fn new(cache: impl Into<PathBuf>) -> Tiers {
        Tiers { cache: cache.into(), ..Tiers::default() }
    }

    fn lama(&self, work_width: u32) -> Lama {
        let runtime = std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from).unwrap_or_else(|| self.cache.join("onnxruntime").join(RUNTIME_FILE));
        Lama::new(runtime, self.cache.join("models").join(crate::lama::MODEL_FILE), work_width)
    }

    fn diffusion(&self, work_width: u32) -> SdCpp {
        let program = self.sd_program.clone().or_else(|| std::env::var_os("CARDSMITH_SD_CLI").map(PathBuf::from)).unwrap_or_else(|| self.cache.join("sdcpp").join(SD_CLI_FILE));
        let model = self.sd_model.clone().unwrap_or_else(|| self.cache.join("models").join(crate::sdcpp::MODEL_FILE));
        let mut diffusion = SdCpp::new(program, model, self.cache.join("sdcpp").join("last-run"), work_width);
        diffusion.extra_args = self.sd_args.clone();
        diffusion
    }

    /// Each tier, best first, and whether it can run here: ("sd" | "lama" | "patchmatch", availability).
    pub fn available(&self) -> Vec<(&'static str, Availability)> {
        vec![("sd", self.diffusion(512).available()), ("lama", self.lama(512).available()), ("patchmatch", PatchMatch { work_width: 512 }.available())]
    }

    /// Fill with `tier` ("sd", "lama", "auto" or "patchmatch"), falling back
    /// to each simpler tier. "auto" is the model if it is installed, else
    /// PatchMatch; Stable Diffusion is only used when asked for by name.
    pub fn fill(&self, tier: &str, request: &FillRequest, work_width: u32) -> Result<Outcome, String> {
        let mut diffusion = self.diffusion(work_width);
        let mut lama = self.lama(work_width);
        let mut patchmatch = PatchMatch { work_width };
        let mut providers: Vec<&mut dyn FillProvider> = Vec::new();
        match tier {
            "sd" => providers.push(&mut diffusion),
            "lama" | "auto" | "patchmatch" => {}
            other => return Err(format!("There is no fill tier called \"{other}\" (sd, lama, auto and patchmatch are the choices).")),
        }
        if tier != "patchmatch" {
            providers.push(&mut lama);
        }
        providers.push(&mut patchmatch);
        fill_with_fallback(&mut providers, request)
    }
}
