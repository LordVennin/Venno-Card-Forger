//! cardsmith render <doc.json | file.cardsmith> -o <out.png>
//! cardsmith samples <dir>
//!
//! For a bare doc.json, assets are read from the paths its asset table names,
//! relative to the JSON file.

use cardsmith_core::doc::Doc;
use cardsmith_core::render::{encode_png, render};
use cardsmith_core::samples::samples;
use cardsmith_core::serialize::{is_zip, load_cardsmith, parse_doc, save_cardsmith, AssetBytes};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "usage: cardsmith render <doc.json | file.cardsmith> -o <out.png>\n       cardsmith samples <dir>";

fn load(file: &Path) -> Result<(Doc, AssetBytes), String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    if is_zip(&bytes) {
        return load_cardsmith(&bytes).map_err(|e| e.to_string());
    }
    let doc = parse_doc(&String::from_utf8_lossy(&bytes)).map_err(|e| e.to_string())?;
    let dir = file.parent().unwrap_or(Path::new("."));
    let mut assets = AssetBytes::new();
    for (id, asset) in &doc.assets {
        // A missing file is left out on purpose: the renderer reports it as a warning.
        if let Ok(data) = std::fs::read(dir.join(&asset.path)) {
            assets.insert(id.clone(), data);
        }
    }
    Ok((doc, assets))
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("samples") => {
            let dir = PathBuf::from(args.get(1).ok_or(USAGE)?);
            for sample in samples() {
                let r = render(&sample.doc, &sample.assets);
                write(&dir.join(format!("{}.cardsmith", sample.name)), &save_cardsmith(&sample.doc, &sample.assets).map_err(|e| e.to_string())?)?;
                write(&dir.join(format!("{}.png", sample.name)), &encode_png(r.width, r.height, &r.pixels).map_err(|e| e.to_string())?)?;
                eprintln!("{}: {}×{} -> {}", sample.name, r.width, r.height, dir.join(sample.name).display());
            }
            Ok(())
        }
        Some("render") => {
            let mut input = None;
            let mut out = None;
            let mut rest = args[1..].iter();
            while let Some(arg) = rest.next() {
                match arg.as_str() {
                    "-o" | "--out" => out = rest.next(),
                    _ => input = Some(arg),
                }
            }
            let (Some(input), Some(out)) = (input, out) else { return Err(USAGE.into()) };
            let (doc, assets) = load(Path::new(input))?;
            let started = Instant::now();
            let r = render(&doc, &assets);
            let elapsed = started.elapsed().as_millis();
            write(Path::new(out), &encode_png(r.width, r.height, &r.pixels).map_err(|e| e.to_string())?)?;
            for warning in &r.warnings {
                eprintln!("warning: {warning}");
            }
            eprintln!("{}×{} in {elapsed} ms -> {out}", r.width, r.height);
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{}{message}", if message.starts_with("usage") { "" } else { "error: " });
            ExitCode::from(if message.starts_with("usage") { 2 } else { 1 })
        }
    }
}
