//! The app's native side.
//!
//! The document, its history and its pixels live in the WebAssembly core in
//! the webview. This side does what a webview cannot: read folders the user
//! chose (templates, fonts, art), ask Scryfall and hexproof.io through the
//! polite, cached client, and make fills, which take seconds of processor or
//! graphics-card time. Commands and files cross the bridge; frames never do.

use cardsmith_cards::file_name::CardInfo;
use cardsmith_fill::tiers::Tiers;
use cardsmith_scryfall::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::ipc::Response;
use tauri::{Manager, State};

/// Where the user keeps things. Saved between runs.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    /// A folder of imported template packages (each a folder with doc.json and manifest.json).
    templates_dir: String,
    fonts_dir: String,
}

struct Shared {
    settings_file: PathBuf,
    settings: Mutex<Settings>,
    /// One client for the whole app, so requests are spaced out whoever asks.
    client: Mutex<Client>,
    cache: PathBuf,
    /// Finished fills waiting to be fetched, by ticket.
    fills: Mutex<(u64, HashMap<u64, Vec<u8>>)>,
}

type Answer<T> = Result<T, String>;

/// The cache the command line uses too, so models installed for one are found by the other.
fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))).unwrap_or_else(|| PathBuf::from(".cache")).join("cardsmith/scryfall")
}

/// A file inside a folder the user chose, by a relative path that cannot climb out of it.
fn inside(folder: &str, relative: &str) -> Answer<PathBuf> {
    if folder.is_empty() {
        return Err("No folder has been chosen yet.".into());
    }
    let relative = Path::new(relative);
    if relative.is_absolute() || relative.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return Err(format!("{} is not a file inside the folder.", relative.display()));
    }
    Ok(Path::new(folder).join(relative))
}

fn read(path: &Path) -> Answer<Vec<u8>> {
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

// ---- settings --------------------------------------------------------------------

#[tauri::command]
fn settings(shared: State<Shared>) -> Settings {
    shared.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(shared: State<Shared>, settings: Settings) -> Answer<()> {
    if let Some(dir) = shared.settings_file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&shared.settings_file, text).map_err(|e| format!("{}: {e}", shared.settings_file.display()))?;
    *shared.settings.lock().unwrap() = settings;
    Ok(())
}

// ---- templates and fonts -----------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TemplateEntry {
    /// The package's folder name, which is how it is asked for.
    folder: String,
    name: String,
    /// False when the importer found none of the layers the card engine sets, so no card can be built on it.
    buildable: bool,
}

/// The template packages in the templates folder, by name.
#[tauri::command]
fn templates(shared: State<Shared>) -> Answer<Vec<TemplateEntry>> {
    let dir = shared.settings.lock().unwrap().templates_dir.clone();
    if dir.is_empty() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| format!("{dir}: {e}"))?.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(manifest) = std::fs::read_to_string(path.join("manifest.json")) else { continue };
        if !path.join("doc.json").is_file() {
            continue;
        }
        let manifest: Value = serde_json::from_str(&manifest).unwrap_or(Value::Null);
        let folder = entry.file_name().to_string_lossy().into_owned();
        let name = manifest["name"].as_str().unwrap_or(&folder).to_string();
        let buildable = manifest["bindings"].as_object().is_some_and(|b| !b.is_empty());
        found.push(TemplateEntry { folder, name, buildable });
    }
    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(found)
}

/// A template package's document and manifest, as text.
#[tauri::command(async)]
fn template(shared: State<Shared>, folder: String) -> Answer<Value> {
    let dir = shared.settings.lock().unwrap().templates_dir.clone();
    let text = |name: &str| inside(&dir, &format!("{folder}/{name}")).and_then(|p| std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display())));
    Ok(json!({ "doc": text("doc.json")?, "manifest": text("manifest.json")? }))
}

/// One of a template package's files (an image), as bytes.
#[tauri::command(async)]
fn template_asset(shared: State<Shared>, folder: String, path: String) -> Answer<Response> {
    let dir = shared.settings.lock().unwrap().templates_dir.clone();
    Ok(Response::new(read(&inside(&dir, &format!("{folder}/{path}"))?)?))
}

fn is_font(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc"))
}

/// The font files in the fonts folder.
#[tauri::command]
fn fonts(shared: State<Shared>) -> Answer<Vec<String>> {
    let dir = shared.settings.lock().unwrap().fonts_dir.clone();
    if dir.is_empty() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = std::fs::read_dir(&dir).map_err(|e| format!("{dir}: {e}"))?.filter_map(Result::ok).filter(|e| is_font(&e.path())).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    Ok(names)
}

#[tauri::command(async)]
fn font(shared: State<Shared>, name: String) -> Answer<Response> {
    let dir = shared.settings.lock().unwrap().fonts_dir.clone();
    Ok(Response::new(read(&inside(&dir, &name)?)?))
}

/// A picture the user picked for the art, as bytes.
#[tauri::command(async)]
fn art_file(path: String) -> Answer<Response> {
    let path = PathBuf::from(path);
    let is_picture = path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp"));
    if !is_picture {
        return Err("Art has to be a PNG, JPEG or WebP picture.".into());
    }
    Ok(Response::new(read(&path)?))
}

// ---- cards -------------------------------------------------------------------------

#[tauri::command(async)]
fn autocomplete(shared: State<Shared>, typed: String) -> Answer<Vec<String>> {
    shared.client.lock().unwrap().autocomplete(&typed).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn printings(shared: State<Shared>, name: String) -> Answer<Vec<cardsmith_scryfall::Printing>> {
    shared.client.lock().unwrap().printings(&name).map_err(|e| e.to_string())
}

/// What goes with a card's set and rarity: the symbol, the set's size for the
/// collector line, and a basic land's watermark.
fn set_extras(client: &mut Client, set: &str, rarity: &str, basic_land_type: Option<&str>) -> Value {
    let letter = match rarity {
        "uncommon" => "U",
        "rare" => "R",
        "mythic" => "M",
        "special" | "bonus" => "S",
        "timeshifted" => "T",
        _ => "C",
    };
    json!({
        "setSize": client.set_info(set).size,
        "symbolSvg": client.symbol_svg(set, letter),
        "watermarkSvg": basic_land_type.and_then(|name| client.watermark_svg(name)),
    })
}

fn basic_land_type(card: &Value) -> Option<&'static str> {
    let type_line = card["type_line"].as_str().unwrap_or_default();
    if !type_line.contains("Basic") || !type_line.contains("Land") {
        return None;
    }
    Some(["Plains", "Island", "Swamp", "Mountain", "Forest"].into_iter().find(|land| type_line.contains(land)).map_or("wastes", |land| match land {
        "Plains" => "plains",
        "Island" => "island",
        "Swamp" => "swamp",
        "Mountain" => "mountain",
        _ => "forest",
    }))
}

/// Look a card up: Scryfall's JSON with its layout settled, the name of the
/// face wanted, and what goes with its set.
#[tauri::command(async)]
fn card(shared: State<Shared>, name: String, set: Option<String>, number: Option<String>) -> Answer<Value> {
    let mut client = shared.client.lock().unwrap();
    let info = CardInfo { name, set: set.unwrap_or_default(), number: number.unwrap_or_default(), ..CardInfo::default() };
    let (scryfall, face) = client.card_settled(&info).map_err(|e| e.to_string())?;
    let extras = set_extras(&mut client, scryfall["set"].as_str().unwrap_or_default(), scryfall["rarity"].as_str().unwrap_or_default(), basic_land_type(&scryfall));
    Ok(json!({ "scryfall": scryfall, "name": face, "extras": extras }))
}

/// The symbol and set size for a set and rarity typed into the card form.
#[tauri::command(async)]
fn set_symbol(shared: State<Shared>, set: String, rarity: String, type_line: String) -> Value {
    let mut client = shared.client.lock().unwrap();
    set_extras(&mut client, &set, &rarity, basic_land_type(&json!({ "type_line": type_line })))
}

// ---- fills -------------------------------------------------------------------------

#[derive(Serialize)]
struct TierState {
    id: &'static str,
    ok: bool,
    /// Why it cannot run here, in words for the user.
    reason: Option<String>,
}

/// The fill tiers, best first, and whether each can run on this machine.
#[tauri::command(async)]
fn fill_tiers(shared: State<Shared>) -> Vec<TierState> {
    Tiers::new(&shared.cache).available().into_iter().map(|(id, state)| TierState { id, ok: state.ok, reason: state.reason }).collect()
}

/// What the workbench reports about a card whose art needs filling around.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FillInfo {
    art_position: (f64, f64),
    art_scale: f64,
    canvas: (u32, u32),
    keep: [f64; 4],
}

/// Fill around a card's art. Returns which tier made it, what was passed
/// over and why, and a ticket to fetch the picture with (`fill_image`), or no
/// ticket when there was nothing to fill.
#[tauri::command(async)]
fn fill(shared: State<Shared>, art_path: String, info: FillInfo, tier: String, seed: u64, prompt: Option<String>) -> Answer<Value> {
    let started = std::time::Instant::now();
    let image = cardsmith_core::render::decode_image(&read(Path::new(&art_path))?).ok_or("The art could not be read as a picture.")?;
    let art = cardsmith_fill::Placed { width: image.w as u32, height: image.h as u32, rgba: &image.data, x: info.art_position.0, y: info.art_position.1, scale: info.art_scale };
    // A quarter of the card's size is plenty for made-up background, and quick.
    let work_width = info.canvas.0 / 4;
    let request = cardsmith_fill::FillRequest { art, canvas: info.canvas, keep: info.keep, seed, prompt: prompt.as_deref().filter(|p| !p.trim().is_empty()) };
    let outcome = Tiers::new(&shared.cache).fill(&tier, &request, work_width)?;
    let ticket = match outcome.filled {
        Some(filled) => {
            let png = cardsmith_core::render::encode_png(filled.width, filled.height, &filled.rgba).map_err(|e| e.to_string())?;
            let mut fills = shared.fills.lock().unwrap();
            fills.0 += 1;
            let ticket = fills.0;
            fills.1.insert(ticket, png);
            Some(ticket)
        }
        None => None,
    };
    Ok(json!({ "ticket": ticket, "provider": outcome.provider, "notices": outcome.notices, "ms": started.elapsed().as_millis() as u64, "workWidth": work_width }))
}

/// The picture of a finished fill (PNG). A ticket is good once.
#[tauri::command]
fn fill_image(shared: State<Shared>, ticket: u64) -> Answer<Response> {
    shared.fills.lock().unwrap().1.remove(&ticket).map(Response::new).ok_or_else(|| "That fill has already been fetched.".to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let settings_file = app.path().app_config_dir()?.join("settings.json");
            let mut settings: Settings = std::fs::read_to_string(&settings_file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
            // For trying the app out against a set of fixtures without touching saved settings.
            if let Some(dir) = std::env::var_os("CARDSMITH_TEMPLATES") {
                settings.templates_dir = dir.to_string_lossy().into_owned();
            }
            if let Some(dir) = std::env::var_os("CARDSMITH_FONTS") {
                settings.fonts_dir = dir.to_string_lossy().into_owned();
            }
            let cache = std::env::var_os("CARDSMITH_CACHE").map(PathBuf::from).unwrap_or_else(cache_dir);
            app.manage(Shared { settings_file, settings: Mutex::new(settings), client: Mutex::new(Client::new(&cache)), cache, fills: Mutex::new((0, HashMap::new())) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![settings, save_settings, templates, template, template_asset, fonts, font, art_file, autocomplete, printings, card, set_symbol, fill_tiers, fill, fill_image])
        .run(tauri::generate_context!())
        .expect("error while running Cardsmith");
}
