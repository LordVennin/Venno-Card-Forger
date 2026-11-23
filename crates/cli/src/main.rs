//! cardsmith render <doc.json | file.cardsmith> -o <out.png>
//! cardsmith samples <dir>
//! cardsmith hashes        print a hash of each sample render
//! cardsmith bench         time editing on the card-sized sample
//! cardsmith compare <actual.png> <reference.png> [--ignore x,y,w,h]... [--heatmap <out.png>]
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

const USAGE: &str = "usage: cardsmith render <doc.json | file.cardsmith> -o <out.png>\n       cardsmith samples <dir>\n       cardsmith hashes\n       cardsmith bench\n       cardsmith compare <actual.png> <reference.png> [--ignore x,y,w,h]... [--heatmap <out.png>]";

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
        Some("hashes") => {
            // One line per sample: "<name> <width>x<height> <hash>". Used to check that
            // the WebAssembly build renders the same bytes as this native one.
            // The card-sized sample is included: it is the one that matters in practice.
            for sample in samples().into_iter().chain([cardsmith_core::samples::card()]) {
                let r = render(&sample.doc, &sample.assets);
                println!("{} {}x{} {:016x}", sample.name, r.width, r.height, cardsmith_core::render::hash(&r.pixels));
            }
            Ok(())
        }
        Some("bench") => bench(),
        Some("compare") => {
            // cardsmith compare <actual.png> <reference.png> [--ignore x,y,w,h]... [--heatmap <out.png>]
            // Prints one line of JSON with the metrics.
            use cardsmith_core::raster::Region;
            use cardsmith_core::render::decode_image;
            let mut files = Vec::new();
            let mut ignore = Vec::new();
            let mut heatmap = None;
            let mut rest = args[1..].iter();
            while let Some(arg) = rest.next() {
                match arg.as_str() {
                    "--ignore" => {
                        let value = rest.next().ok_or(USAGE)?;
                        let n: Vec<i32> = value.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                        let [x, y, w, h] = n[..] else { return Err(format!("--ignore takes x,y,w,h, not \"{value}\"")) };
                        ignore.push(Region::new(x, y, w, h));
                    }
                    "--heatmap" => heatmap = rest.next(),
                    _ => files.push(arg),
                }
            }
            let [actual, reference] = files[..] else { return Err(USAGE.into()) };
            let load = |file: &String| -> Result<_, String> {
                let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
                decode_image(&bytes).ok_or_else(|| format!("{file}: not a readable image"))
            };
            let (a, r) = (load(actual)?, load(reference)?);
            if (a.w, a.h) != (r.w, r.h) {
                return Err(format!("the images differ in size: {}×{} and {}×{}", a.w, a.h, r.w, r.h));
            }
            let (c, heat) = cardsmith_core::compare::compare(a.w as u32, a.h as u32, &a.data, &r.data, &ignore);
            if let Some(out) = heatmap {
                write(Path::new(out), &encode_png(a.w as u32, a.h as u32, &heat).map_err(|e| e.to_string())?)?;
            }
            println!(
                "{}",
                serde_json::json!({
                    // JSON has no infinity; identical images report null.
                    "psnr": if c.psnr.is_finite() { Some(c.psnr) } else { None },
                    "maxDiff": c.max_diff,
                    "differing": c.differing,
                    "compared": c.compared,
                    "worstBlock": { "x": c.worst_block.x, "y": c.worst_block.y, "w": c.worst_block.w, "h": c.worst_block.h, "meanError": c.worst_block_error },
                })
            );
            Ok(())
        }
        Some("render") => {
            let mut input = None;
            let mut out = None;
            let mut raw = None;
            let mut rest = args[1..].iter();
            while let Some(arg) = rest.next() {
                match arg.as_str() {
                    "-o" | "--out" => out = rest.next(),
                    // Also write the pixels as bare RGBA bytes, for tools that compare them.
                    "--raw" => raw = rest.next(),
                    _ => input = Some(arg),
                }
            }
            let (Some(input), Some(out)) = (input, out) else { return Err(USAGE.into()) };
            let (doc, assets) = load(Path::new(input))?;
            let started = Instant::now();
            let r = render(&doc, &assets);
            let elapsed = started.elapsed().as_millis();
            write(Path::new(out), &encode_png(r.width, r.height, &r.pixels).map_err(|e| e.to_string())?)?;
            if let Some(raw) = raw {
                write(Path::new(raw), &r.pixels)?;
            }
            for warning in &r.warnings {
                eprintln!("warning: {warning}");
            }
            eprintln!("{}×{} in {elapsed} ms -> {out}", r.width, r.height);
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

/// Time the operations that matter for editing, on the card-sized sample.
fn bench() -> Result<(), String> {
    use cardsmith_core::paint::Brush;
    use cardsmith_core::session::{Session, View};
    let timed = |label: &str, f: &mut dyn FnMut()| {
        let started = Instant::now();
        f();
        println!("{label:<52} {:>8.1} ms", started.elapsed().as_secs_f64() * 1000.0);
    };
    let sample = cardsmith_core::samples::card();
    let layers = { let mut n = 0; cardsmith_core::tree::walk(&sample.doc.root, &mut |_| n += 1); n };
    println!("card sample: {}×{}, {layers} layers", sample.doc.width, sample.doc.height);
    let mut session = None;
    timed("open and render everything", &mut || session = Some(Session::new(sample.doc.clone(), sample.assets.clone())));
    let mut s = session.unwrap();
    timed("fit-to-window view (980×1333)", &mut || s.set_view(View { scale: 0.3, x: 0.0, y: 0.0, w: 980, h: 1333 }));
    let mut layer = String::new();
    timed("add a paint layer (full redraw)", &mut || layer = s.add_paint_layer("Paint").unwrap());

    for (size, hardness) in [(32.0, 1.0), (128.0, 0.8), (256.0, 0.5), (600.0, 0.0)] {
        let brush = Brush { size, hardness, color: [200, 30, 30], ..Brush::default() };
        let begin = Instant::now();
        s.stroke_begin(&layer, brush).map_err(|e| e.to_string())?;
        let begin = begin.elapsed().as_secs_f64() * 1000.0;
        // A diagonal stroke across the art and the frame, in mouse-sized steps.
        let mut times = Vec::new();
        for i in 0..60 {
            let started = Instant::now();
            s.stroke_to(500.0 + i as f32 * 36.0, 700.0 + i as f32 * 40.0, 1.0);
            times.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        s.stroke_end("Brush");
        times.sort_by(f64::total_cmp);
        println!(
            "brush {size:>4.0} px: begin {begin:>5.1} ms, per move: median {:>5.1}, 95th {:>5.1}, worst {:>5.1} ms",
            times[times.len() / 2], times[times.len() * 95 / 100], times[times.len() - 1]
        );
    }
    timed("undo last stroke", &mut || { s.undo(); });
    timed("redo", &mut || { s.redo(); });
    timed("toggle a layer (full redraw)", &mut || {
        let props = serde_json::json!({ "visible": false }).as_object().unwrap().clone();
        s.execute(cardsmith_core::commands::Edit::SetLayerProps { id: "art-shade".into(), props }, "Hide").unwrap();
    });
    Ok(())
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
