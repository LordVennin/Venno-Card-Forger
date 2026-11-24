//! cardsmith render <doc.json | file.cardsmith> -o <out.png> [--fonts <dir>]
//! cardsmith samples <dir>
//! cardsmith hashes        print a hash of each sample render
//! cardsmith bench         time editing on the card-sized sample
//! cardsmith compare <actual.png> <reference.png> [--ignore x,y,w,h]... [--heatmap <out.png>]
//! cardsmith text <suite.json> -o <dir> --fonts <dir>    render a suite of rules text boxes
//! cardsmith card <name> [--set <code>] [--number <n>] [--cache <dir>] [--raw]    look a card up on Scryfall (cached)
//! cardsmith bulk <oracle | default> [--cache <dir>]      download Scryfall's bulk data for offline lookups
//! cardsmith build <template-dir> <name | card.json> -o <out.png | out.cardsmith> [--fonts <dir>] [--art <image>] [--cache <dir>] [--no-symbol] [--fill patchmatch|none] [--seed <n>]
//!                                                         build a card from an imported template
//! cardsmith symbols [--cache <dir>]                      download set symbols and set data (hexproof.io) for offline use
//! cardsmith sheet -o <out.png> [--across N] [--shrink N] <image.png>...    many images on one sheet
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

const USAGE: &str = "usage: cardsmith render <doc.json | file.cardsmith> -o <out.png> [--fonts <dir>]\n       cardsmith samples <dir>\n       cardsmith hashes\n       cardsmith bench\n       cardsmith compare <actual.png> <reference.png> [--ignore x,y,w,h]... [--heatmap <out.png>]\n       cardsmith text <suite.json> -o <dir> --fonts <dir>\n       cardsmith card <name> [--set <code>] [--number <n>] [--cache <dir>] [--raw]\n       cardsmith bulk <oracle | default> [--cache <dir>]\n       cardsmith build <template-dir> <name | card.json> -o <out.png | out.cardsmith> [--fonts <dir>] [--art <image>] [--cache <dir>] [--no-symbol] [--fill patchmatch|none] [--seed <n>]\n       cardsmith symbols [--cache <dir>]";

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

/// Load every font file in a folder into a renderer. Files that are not fonts are skipped.
fn load_fonts(renderer: &mut cardsmith_core::render::Renderer, dir: &str) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{dir}: {e}"))?;
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        let is_font = path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc"));
        if is_font {
            if let Ok(bytes) = std::fs::read(&path) {
                // A file that will not parse is reported by the renderer as a missing font, by name.
                let _ = renderer.fonts_mut().add(bytes);
            }
        }
    }
    Ok(())
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
            let mut fonts = None;
            let mut text_bounds = None;
            let mut rest = args[1..].iter();
            while let Some(arg) = rest.next() {
                match arg.as_str() {
                    "-o" | "--out" => out = rest.next(),
                    // A folder of font files for text layers.
                    "--fonts" => fonts = rest.next(),
                    // Also write each text layer's ink bounds as JSON, for checking layout.
                    "--text-bounds" => text_bounds = rest.next(),
                    // Also write the pixels as bare RGBA bytes, for tools that compare them.
                    "--raw" => raw = rest.next(),
                    _ => input = Some(arg),
                }
            }
            let (Some(input), Some(out)) = (input, out) else { return Err(USAGE.into()) };
            let (doc, assets) = load(Path::new(input))?;
            let mut renderer = cardsmith_core::render::Renderer::new();
            if let Some(dir) = fonts {
                load_fonts(&mut renderer, dir)?;
            }
            let started = Instant::now();
            let r = renderer.render(&doc, &assets);
            let elapsed = started.elapsed().as_millis();
            if let Some(file) = text_bounds {
                let bounds: serde_json::Map<String, serde_json::Value> = renderer.text_ink_bounds(&doc).into_iter().map(|(id, ink)| (id, serde_json::json!(ink))).collect();
                write(Path::new(file), serde_json::to_string(&bounds).unwrap_or_default().as_bytes())?;
            }
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
        Some("symbols") => {
            // cardsmith symbols [--cache <dir>]: download the set symbols and set data for offline use.
            let cache = match (args.get(1).map(String::as_str), args.get(2)) {
                (Some("--cache"), Some(dir)) => PathBuf::from(dir),
                (None, _) => scryfall_cache(),
                _ => return Err(USAGE.into()),
            };
            let mut client = cardsmith_scryfall::Client::new(&cache);
            let dir = client.symbols().map_err(|e| e.to_string())?;
            let sets = std::fs::read_dir(dir.join("set")).map(|d| d.count()).unwrap_or(0);
            let known = client.set_info("lea").size.is_some();
            eprintln!("Symbols for {sets} sets in {}; set data {}.", dir.display(), if known { "downloaded" } else { "could not be downloaded" });
            Ok(())
        }
        Some("card") | Some("bulk") => {
            let mut what = None;
            let (mut set, mut number, mut raw) = (None, None, false);
            // Like other apps' caches, under the user's cache folder unless told otherwise.
            let mut cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))).unwrap_or_else(|| PathBuf::from(".cache")).join("cardsmith/scryfall");
            let mut rest = args[1..].iter();
            while let Some(arg) = rest.next() {
                match arg.as_str() {
                    "--set" => set = rest.next(),
                    "--number" => number = rest.next(),
                    "--cache" => cache = PathBuf::from(rest.next().ok_or(USAGE)?),
                    // Scryfall's own data instead of the card as the engine will use it.
                    "--raw" => raw = true,
                    _ => what = Some(arg),
                }
            }
            let what = what.ok_or(USAGE)?;
            let mut client = cardsmith_scryfall::Client::new(&cache);
            if args[0] == "bulk" {
                let kind = match what.as_str() {
                    "oracle" => cardsmith_scryfall::BulkKind::OracleCards,
                    "default" => cardsmith_scryfall::BulkKind::DefaultCards,
                    _ => return Err(USAGE.into()),
                };
                let cards = client.bulk(kind).map_err(|e| e.to_string())?;
                eprintln!("{} cards in {}", cards.len(), cache.join("bulk").display());
                return Ok(());
            }
            // The name may carry Proxyshop's art file tags: "Name (Artist) [SET] {num} $creator".
            let mut info = cardsmith_cards::file_name::parse_card_info(what);
            if let Some(set) = set {
                info.set = set.clone();
            }
            if let Some(number) = number {
                info.number = number.clone();
            }
            let out = if raw {
                client.card(&info.name, Some(&info.set), Some(&info.number)).map_err(|e| e.to_string())?
            } else {
                serde_json::to_value(client.card_data(&info).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?
            };
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
            Ok(())
        }
        Some("sheet") => {
            // cardsmith sheet -o <out.png> [--across N] [--shrink N] <image.png>...
            let (mut out, mut across, mut shrink) = (None, 4, 4);
            let mut files = Vec::new();
            let mut rest = args[1..].iter();
            while let Some(arg) = rest.next() {
                match arg.as_str() {
                    "-o" | "--out" => out = rest.next(),
                    "--across" => across = rest.next().and_then(|n| n.parse().ok()).filter(|n| *n > 0).ok_or(USAGE)?,
                    "--shrink" => shrink = rest.next().and_then(|n| n.parse().ok()).filter(|n| *n > 0).ok_or(USAGE)?,
                    _ => files.push(arg),
                }
            }
            let out = out.ok_or(USAGE)?;
            let mut images = Vec::new();
            for file in files {
                let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
                let image = cardsmith_core::render::decode_image(&bytes).ok_or(format!("{file}: not a readable image"))?;
                images.push(cardsmith_core::render::RenderResult { width: image.w as u32, height: image.h as u32, pixels: image.data, warnings: Vec::new() });
            }
            write(Path::new(out), &contact_sheet(&images, across, shrink)?)
        }
        Some("build") => {
            let mut plain = Vec::new();
            let (mut out, mut fonts, mut art, mut cache) = (None, None, None, None);
            let mut symbol = true;
            let (mut fill, mut seed) = (true, 1u64);
            let mut rest = args[1..].iter();
            while let Some(arg) = rest.next() {
                match arg.as_str() {
                    "-o" | "--out" => out = rest.next(),
                    "--fonts" => fonts = rest.next(),
                    "--art" => art = rest.next(),
                    "--cache" => cache = rest.next(),
                    // Leave the set symbol off, and make no request for it.
                    "--no-symbol" => symbol = false,
                    // How the space around the art is filled: "patchmatch" (the default) or "none".
                    "--fill" => fill = match rest.next().map(String::as_str) {
                        Some("patchmatch") => true,
                        Some("none") => false,
                        _ => return Err(USAGE.into()),
                    },
                    // A different seed gives a different fill.
                    "--seed" => seed = rest.next().and_then(|n| n.parse().ok()).ok_or(USAGE)?,
                    _ => plain.push(arg),
                }
            }
            let (Some(dir), Some(what), Some(out)) = (plain.first(), plain.get(1), out) else { return Err(USAGE.into()) };
            build_card(Path::new(dir), what, Path::new(out), fonts.map(String::as_str), art.map(String::as_str), cache.map(PathBuf::from), symbol, fill.then_some(seed))
        }
        Some("text") => {
            let mut input = None;
            let mut out = None;
            let mut fonts = None;
            let mut rest = args[1..].iter();
            while let Some(arg) = rest.next() {
                match arg.as_str() {
                    "-o" | "--out" => out = rest.next(),
                    "--fonts" => fonts = rest.next(),
                    _ => input = Some(arg),
                }
            }
            let (Some(input), Some(out), Some(fonts)) = (input, out, fonts) else { return Err(USAGE.into()) };
            text_suite(Path::new(input), Path::new(out), fonts)
        }
        _ => Err(USAGE.into()),
    }
}

/// Many renders on one image, `across` to a row, each shrunk by a whole factor, over grey so transparent corners show.
fn contact_sheet(images: &[cardsmith_core::render::RenderResult], across: u32, shrink: u32) -> Result<Vec<u8>, String> {
    let (width, height) = (images.iter().map(|i| i.width).max().unwrap_or(1), images.iter().map(|i| i.height).max().unwrap_or(1));
    let (cw, ch) = (width / shrink, height / shrink);
    let rows = (images.len() as u32).div_ceil(across);
    let (sw, sh) = (cw * across + (across + 1) * 8, ch * rows + (rows + 1) * 8);
    const GREY: u32 = 96;
    let mut sheet = vec![GREY as u8; (sw * sh * 4) as usize];
    sheet.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    for (n, image) in images.iter().enumerate() {
        let (ox, oy) = (8 + (n as u32 % across) * (cw + 8), 8 + (n as u32 / across) * (ch + 8));
        for y in 0..image.height / shrink {
            for x in 0..image.width / shrink {
                // Average the block, over the grey.
                let mut sum = [0u32; 3];
                for sy in 0..shrink {
                    for sx in 0..shrink {
                        let i = (((y * shrink + sy) * image.width + x * shrink + sx) * 4) as usize;
                        let a = image.pixels[i + 3] as u32;
                        for c in 0..3 {
                            sum[c] += (image.pixels[i + c] as u32 * a + GREY * (255 - a)) / 255;
                        }
                    }
                }
                let o = (((oy + y) * sw + ox + x) * 4) as usize;
                for c in 0..3 {
                    sheet[o + c] = (sum[c] / (shrink * shrink)) as u8;
                }
            }
        }
    }
    encode_png(sw, sh, &sheet).map_err(|e| e.to_string())
}

fn scryfall_cache() -> PathBuf {
    // Like other apps' caches, under the user's cache folder.
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))).unwrap_or_else(|| PathBuf::from(".cache")).join("cardsmith/scryfall")
}

/// Build a card from a template package and write it as an image or a document.
///
/// `what` is a card name (with Proxyshop's art file tags if wanted), looked up
/// on Scryfall through the cache, or a JSON file holding card data as
/// `cardsmith card` prints it, for custom cards and for tests.
fn build_card(dir: &Path, what: &str, out: &Path, fonts: Option<&str>, art: Option<&str>, cache: Option<PathBuf>, symbol: bool, fill: Option<u64>) -> Result<(), String> {
    use cardsmith_engine::{Art, Options, Template};
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).map_err(|e| format!("{}: {e}", dir.join(name).display()));
    let template = Template::from_json(&read("doc.json")?, &read("manifest.json")?)?;
    let (_, mut assets) = load(&dir.join("doc.json"))?;

    let card: cardsmith_cards::layout::CardData = if what.ends_with(".json") {
        serde_json::from_str(&std::fs::read_to_string(what).map_err(|e| format!("{what}: {e}"))?).map_err(|e| format!("{what}: {e}"))?
    } else {
        let info = cardsmith_cards::file_name::parse_card_info(what);
        cardsmith_scryfall::Client::new(cache.clone().unwrap_or_else(scryfall_cache)).card_data(&info).map_err(|e| e.to_string())?
    };

    let mut options = Options::default();
    let mut art_pixels = None;
    if symbol {
        // The set's symbol in this card's rarity, and the set's size for the collector line, from hexproof.io through the cache.
        let mut client = cardsmith_scryfall::Client::new(cache.unwrap_or_else(scryfall_cache));
        options.set_size = client.set_info(&card.set).size;
        match client.symbol_svg(&card.set, &card.rarity_letter()).map(|svg| cardsmith_engine::svg::parse_svg(&svg)) {
            Some(Ok(art)) => options.symbol = Some(art),
            Some(Err(e)) => eprintln!("warning: The set symbol for {} could not be read: {e}", card.set),
            None => eprintln!("warning: No set symbol for {} could be found; the card is built without one.", card.set),
        }
        // A basic land's textbox shows its land type's symbol.
        if card.is_basic_land {
            let name = match card.frame.pinlines.as_str() {
                "W" => "plains",
                "U" => "island",
                "B" => "swamp",
                "R" => "mountain",
                "G" => "forest",
                _ => "wastes",
            };
            options.basic_watermark = client.watermark_svg(name).and_then(|svg| cardsmith_engine::svg::parse_svg(&svg).ok());
        }
    }
    if let Some(file) = art {
        let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
        let image = cardsmith_core::render::decode_image(&bytes).ok_or(format!("{file}: not a readable image"))?;
        options.art = Some(Art { asset: "card-art".into(), path: "assets/card-art.png".into(), width: image.w as u32, height: image.h as u32 });
        assets.insert("card-art".into(), bytes);
        art_pixels = Some(image);
    }

    let mut renderer = cardsmith_core::render::Renderer::new();
    if let Some(dir) = fonts {
        load_fonts(&mut renderer, dir)?;
    }
    let built = cardsmith_engine::build_card(&template, &card, renderer.fonts(), &options)?;
    for warning in &built.warnings {
        eprintln!("warning: {warning}");
    }
    let mut doc = template.doc.clone();
    cardsmith_core::commands::apply(&mut doc, &built.edit).map_err(|e| e.to_string())?;

    // Fill the space the art leaves, on a layer of its own beneath it.
    if let (Some(plan), Some(seed), Some(image)) = (&built.fill, fill, &art_pixels) {
        let started = Instant::now();
        let placed = cardsmith_fill::Placed { width: image.w as u32, height: image.h as u32, rgba: &image.data, x: plan.art_position.0, y: plan.art_position.1, scale: plan.art_scale };
        // A quarter of the card's size is plenty for made-up background, and quick.
        let work_width = plan.canvas.0 / 4;
        let request = cardsmith_fill::FillRequest { art: placed, canvas: plan.canvas, keep: plan.keep, seed, prompt: None };
        // Only the first tier exists so far; the others will go in front of it here.
        let outcome = cardsmith_fill::fill_with_fallback(&mut [&mut cardsmith_fill::PatchMatch { work_width }], &request)?;
        for notice in &outcome.notices {
            eprintln!("warning: {notice}");
        }
        if let Some(filled) = outcome.filled {
            let png = encode_png(filled.width, filled.height, &filled.rgba).map_err(|e| e.to_string())?;
            let edit = plan.edit("card-fill", "assets/card-fill.png", filled.width, filled.height, filled.scale, outcome.provider, serde_json::json!({ "seed": seed, "workWidth": work_width }))?;
            assets.insert("card-fill".into(), png);
            cardsmith_core::commands::apply(&mut doc, &edit).map_err(|e| e.to_string())?;
            eprintln!("filled around the art in {} ms ({}×{})", started.elapsed().as_millis(), filled.width, filled.height);
        }
    }

    if out.extension().is_some_and(|e| e == "cardsmith") {
        write(out, &save_cardsmith(&doc, &assets).map_err(|e| e.to_string())?)?;
    } else {
        let r = renderer.render(&doc, &assets);
        for warning in &r.warnings {
            eprintln!("warning: {warning}");
        }
        write(out, &encode_png(r.width, r.height, &r.pixels).map_err(|e| e.to_string())?)?;
    }
    eprintln!("{} ({}) -> {}", card.name, template.id, out.display());
    Ok(())
}

/// Render every card of a rules-text suite into its own image, plus one sheet
/// of them all, and write what was measured to results.json.
///
/// The suite file gives the text box (`box.w`, `box.h`), the font and size,
/// the document resolution, and a list of cards with `name`, `rules` and
/// optionally `flavor`. Each card is formatted the way the card engine will
/// (symbols, italics, bullets, flavor spacing), shrunk to fit the box in
/// Proxyshop's steps, and centered vertically.
fn text_suite(input: &Path, out: &Path, fonts: &str) -> Result<(), String> {
    use cardsmith_cards::rules_text::{format_rules_text, RulesFormat};
    use cardsmith_core::doc::{build::shape, Layer};
    use cardsmith_core::text::layout_text;
    use serde_json::{json, Value};

    let suite: Value = serde_json::from_str(&std::fs::read_to_string(input).map_err(|e| format!("{}: {e}", input.display()))?).map_err(|e| format!("{}: {e}", input.display()))?;
    let number = |v: &Value, what: &str| v.as_f64().ok_or(format!("{}: {what} must be a number", input.display()));
    let (w, h) = (number(&suite["box"]["w"], "box.w")?, number(&suite["box"]["h"], "box.h")?);
    let size = number(&suite["size"], "size")?;
    let dpi = number(&suite["dpi"], "dpi")?;
    let font = suite["font"].as_str().ok_or("font must be a string")?;
    let cards = suite["cards"].as_array().ok_or("cards must be a list")?;
    let mut renderer = cardsmith_core::render::Renderer::new();
    load_fonts(&mut renderer, fonts)?;

    const PAD: f64 = 48.0;
    let (width, height) = ((w + PAD * 2.0) as u32, (h + PAD * 2.0) as u32);
    let mut results = Vec::new();
    let mut images = Vec::new();
    for card in cards {
        let name = card["name"].as_str().ok_or("every card needs a name")?;
        let slug: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
        let (rules, flavor) = (card["rules"].as_str().unwrap_or(""), card["flavor"].as_str().unwrap_or(""));
        let format = RulesFormat { divider: !rules.is_empty() && !flavor.is_empty(), ..RulesFormat::for_dpi(dpi) };
        let formatted = format_rules_text(rules, flavor, &format);
        let layer: Layer = serde_json::from_value(json!({
            "kind": "text", "id": "rules", "name": "Rules Text", "visible": true, "opacity": 1, "blend": "normal", "clip": false,
            "box": { "x": PAD, "y": PAD, "w": w, "h": h },
            "content": formatted.content,
            "style": { "font": font, "size": size, "color": [0, 0, 0, 1], "leading": 1, "align": "left" },
            "fit": "shrinkHeight", "fitStep": 0.4 * dpi / 72.0, "verticalAlign": "center"
        }))
        .map_err(|e| e.to_string())?;
        let Layer::Text(text) = &layer else { unreachable!() };
        let layout = layout_text(text, renderer.fonts());

        let mut doc = Doc::empty(width, height, dpi);
        doc.root.children.push(shape("paper", &format!("M0 0H{width}V{height}H0Z"), [1.0, 1.0, 1.0, 1.0]));
        // Stands in for the flavor divider: a line midway between the rules and the flavor text.
        let mut divider = None;
        if let Some(at) = formatted.flavor_paragraph {
            let above = layout.paragraph_ink[..at].iter().rev().flatten().next();
            let below = layout.paragraph_ink[at..].iter().flatten().next();
            if let (Some(above), Some(below)) = (above, below) {
                let y = ((above[3] + below[1]) / 2.0).round();
                doc.root.children.push(shape("divider", &format!("M{PAD} {}H{}V{}H{PAD}Z", y - 2.0, PAD + w, y + 2.0), [0.0, 0.0, 0.0, 0.35]));
                divider = Some(y);
            }
        }
        doc.root.children.push(layer.clone());
        let r = renderer.render(&doc, &AssetBytes::new());
        write(&out.join(format!("{slug}.png")), &encode_png(r.width, r.height, &r.pixels).map_err(|e| e.to_string())?)?;

        // Fits when it is no taller than the box. Sideways, a glyph may overhang a little, as an italic "f" does.
        let overhang = size * 0.2;
        let fits = layout.ink.is_none_or(|i| i[0] >= PAD - overhang && i[2] <= PAD + w + overhang && i[1] >= PAD - 1.0 && i[3] <= PAD + h + 1.0);
        results.push(json!({
            "name": name, "file": format!("{slug}.png"), "scale": layout.scale, "size": size * layout.scale, "lines": layout.lines,
            "ink": layout.ink, "fits": fits, "divider": divider, "unknownSymbols": formatted.unknown_symbols, "missingFonts": layout.missing,
        }));
        eprintln!("{name}: {} lines at {:.1} px{}", layout.lines, size * layout.scale, if fits { "" } else { " — DOES NOT FIT" });
        images.push(r);
    }

    // One sheet of every card at a quarter size, four across.
    if !images.is_empty() {
        write(&out.join("sheet.png"), &contact_sheet(&images, 4, 4)?)?;
    }
    write(&out.join("results.json"), serde_json::to_string_pretty(&results).unwrap_or_default().as_bytes())
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
