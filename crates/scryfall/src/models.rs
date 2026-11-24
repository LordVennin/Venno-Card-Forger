//! Downloads for the model-based fill tier: the model file, and the ONNX
//! Runtime library that runs it. Neither ships with the app; both are fetched
//! into the cache folder when that tier is first wanted.

use crate::{Client, Error, FetchError, Transport};
use std::io::Read;
use std::path::{Path, PathBuf};

/// PyPI's listing of the official `onnxruntime` package. Its wheels are zip
/// files with the library inside, built by the ONNX Runtime project for each
/// system, which makes PyPI the one place every build can be had from.
const RUNTIME_LISTING: &str = "https://pypi.org/pypi/onnxruntime/json";

fn failed(what: &str, e: FetchError) -> Error {
    match e {
        FetchError::Unreachable(why) => Error::Offline(why),
        FetchError::NotFound(_) => Error::Other(format!("{what} is not at its usual address")),
        FetchError::Other(why) => Error::Other(why),
    }
}

impl<T: Transport> Client<T> {
    /// Download a large file into the cache unless it is already there at the
    /// expected size. Returns where it is.
    pub fn fetch_file(&mut self, url: &str, folder: &str, name: &str, bytes: Option<u64>) -> Result<PathBuf, Error> {
        let dir = self.cache.join(folder);
        let file = dir.join(name);
        let size = |path: &Path| std::fs::metadata(path).map(|m| m.len()).ok();
        if size(&file).is_some_and(|have| bytes.is_none_or(|want| want == have)) {
            return Ok(file);
        }
        std::fs::create_dir_all(&dir).map_err(|e| Error::Other(format!("{}: {e}", dir.display())))?;
        self.transport.download(url, &file).map_err(|e| failed(name, e))?;
        match (size(&file), bytes) {
            (Some(have), Some(want)) if have != want => {
                let _ = std::fs::remove_file(&file);
                Err(Error::Other(format!("{name} came out {have} bytes long where {want} were expected; it was not kept")))
            }
            _ => Ok(file),
        }
    }

    /// Make sure the ONNX Runtime library is in the cache, and return its
    /// path. `file_name` is what the library is called on this system
    /// ("libonnxruntime.so"); `platform` and `machine` pick the build ("manylinux", "x86_64").
    pub fn onnx_runtime(&mut self, file_name: &str, platform: &str, machine: &str) -> Result<PathBuf, Error> {
        let dir = self.cache.join("onnxruntime");
        let library = dir.join(file_name);
        if library.is_file() {
            return Ok(library);
        }
        let listing = self.transport.get(RUNTIME_LISTING).map_err(|e| failed("the ONNX Runtime listing", e))?;
        let listing: serde_json::Value = serde_json::from_slice(&listing).map_err(|e| Error::Other(format!("the ONNX Runtime listing is not JSON: {e}")))?;
        // Any wheel of the newest release for this system will do: the library inside does not depend on the Python version.
        let wheel = listing["urls"].as_array().and_then(|files| files.iter().find(|f| f["filename"].as_str().is_some_and(|n| n.ends_with(".whl") && n.contains(platform) && n.contains(machine))));
        let url = wheel.and_then(|w| w["url"].as_str()).ok_or_else(|| Error::Other(format!("There is no ONNX Runtime build for {platform} {machine} on PyPI.")))?;
        std::fs::create_dir_all(&dir).map_err(|e| Error::Other(format!("{}: {e}", dir.display())))?;
        let archive = dir.join("onnxruntime.whl");
        self.transport.download(url, &archive).map_err(|e| failed("ONNX Runtime", e))?;

        // The library is the one file wanted from the wheel: "onnxruntime/capi/libonnxruntime.so.1.2.3" and the like.
        let stem = file_name.split('.').next().unwrap_or(file_name);
        let bad = |e: String| Error::Other(format!("The ONNX Runtime download could not be unpacked: {e}"));
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&archive).map_err(|e| bad(e.to_string()))?).map_err(|e| bad(e.to_string()))?;
        let inside = (0..zip.len()).filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string())).find(|n| n.rsplit('/').next().is_some_and(|base| base.starts_with(stem) && !base.contains("providers")));
        let inside = inside.ok_or_else(|| bad(format!("it has no {file_name} in it")))?;
        let mut bytes = Vec::new();
        zip.by_name(&inside).map_err(|e| bad(e.to_string()))?.read_to_end(&mut bytes).map_err(|e| bad(e.to_string()))?;
        std::fs::write(&library, bytes).map_err(|e| Error::Other(format!("{}: {e}", library.display())))?;
        let _ = std::fs::remove_file(&archive);
        Ok(library)
    }
}
