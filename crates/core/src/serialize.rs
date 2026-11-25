//! Document JSON and the `.cardsmith` save format: a zip holding `doc.json`
//! and every asset at the path its `AssetRef` names (normally under `assets/`).

use crate::doc::{AssetId, Doc, Layer, LayerBase};
use crate::tree::walk;
use crate::{err, Result};
use std::collections::{BTreeMap, HashSet};
use std::io::{Cursor, Read, Write};

/// Asset bytes keyed by asset id.
pub type AssetBytes = BTreeMap<AssetId, Vec<u8>>;

fn invalid(message: String) -> crate::Error {
    err!("Invalid document: {message}")
}

fn check_base<'a>(base: &'a LayerBase, doc: &Doc, seen: &mut HashSet<&'a str>) -> Result<()> {
    if base.id.is_empty() {
        return Err(invalid("a layer has no id".into()));
    }
    if !seen.insert(&base.id) {
        return Err(invalid(format!("duplicate layer id \"{}\"", base.id)));
    }
    let unit = |v: f64| (0.0..=1.0).contains(&v);
    if !unit(base.opacity) || !base.fill_opacity.is_none_or(unit) {
        return Err(invalid(format!("layer \"{}\" opacity must be 0..1", base.id)));
    }
    if let Some(mask) = &base.mask {
        needs_asset(doc, &base.id, "mask", &mask.asset)?;
    }
    Ok(())
}

fn needs_asset(doc: &Doc, layer: &str, what: &str, asset: &str) -> Result<()> {
    if doc.assets.contains_key(asset) {
        Ok(())
    } else {
        Err(invalid(format!("layer \"{layer}\" {what} refers to missing asset \"{asset}\"")))
    }
}

/// Check the structure the renderer and commands rely on.
pub fn validate(doc: &Doc) -> Result<()> {
    if doc.version != 1 {
        return Err(invalid(format!("unsupported version {} (this build reads version 1)", doc.version)));
    }
    if doc.width == 0 || doc.height == 0 {
        return Err(invalid("\"width\" and \"height\" must be positive".into()));
    }
    if !(doc.dpi > 0.0) {
        return Err(invalid("\"dpi\" must be a positive number".into()));
    }
    for (id, asset) in &doc.assets {
        let unsafe_path = asset.path.is_empty()
            || asset.path.starts_with('/')
            || asset.path.split(['/', '\\']).any(|part| part == "..")
            || asset.path == "doc.json";
        if unsafe_path {
            return Err(invalid(format!("asset \"{id}\" has an unsafe path")));
        }
    }
    let mut seen = HashSet::new();
    check_base(&doc.root.base, doc, &mut seen)?;
    let mut result = Ok(());
    walk(&doc.root, &mut |layer| {
        if result.is_err() {
            return;
        }
        result = check_base(layer.base(), doc, &mut seen).and_then(|()| match layer {
            Layer::Pixel(l) => needs_asset(doc, &l.base.id, "image", &l.asset),
            Layer::Fill(l) => needs_asset(doc, &l.base.id, "image", &l.asset),
            Layer::Art(l) => l.asset.as_ref().map_or(Ok(()), |a| needs_asset(doc, &l.base.id, "image", a)),
            _ => Ok(()),
        });
    });
    result
}

pub fn parse_doc(json: &str) -> Result<Doc> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| invalid(format!("not JSON ({e})")))?;
    // Checked first, so a newer format gets a clear message instead of a field error.
    match value.get("version") {
        Some(v) if v.as_u64() == Some(1) => {}
        Some(v) => return Err(invalid(format!("unsupported version {v} (this build reads version 1)"))),
        None => return Err(invalid("no \"version\"".into())),
    }
    let doc: Doc = serde_json::from_value(value).map_err(|e| invalid(e.to_string()))?;
    validate(&doc)?;
    Ok(doc)
}

pub fn stringify_doc(doc: &Doc) -> Result<String> {
    validate(doc)?;
    let mut json = serde_json::to_string_pretty(doc).map_err(|e| err!("{e}"))?;
    json.push('\n');
    Ok(json)
}

/// Pack a document and its asset bytes into `.cardsmith` zip bytes.
pub fn save_cardsmith(doc: &Doc, assets: &AssetBytes) -> Result<Vec<u8>> {
    save(doc, assets, true)
}

/// As [`save_cardsmith`], but an asset with no data is left out of the file
/// instead of refusing. A card built on a template holds only the images its
/// showing layers needed; the rest stay in the template's folder, and a file
/// saved this way is a fraction of the size. Open it with [`load_cardsmith_available`].
pub fn save_cardsmith_available(doc: &Doc, assets: &AssetBytes) -> Result<Vec<u8>> {
    save(doc, assets, false)
}

fn save(doc: &Doc, assets: &AssetBytes, whole: bool) -> Result<Vec<u8>> {
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;
    let io = |e: std::io::Error| err!("Could not write the save file: {e}");
    let zipped = |e: zip::result::ZipError| err!("Could not write the save file: {e}");

    let json = stringify_doc(doc)?;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("doc.json", SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)).map_err(zipped)?;
    zip.write_all(json.as_bytes()).map_err(io)?;
    // Images are already compressed; storing them is faster and no bigger.
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (id, asset) in &doc.assets {
        let Some(bytes) = assets.get(id) else {
            if whole {
                return Err(err!("Cannot save: no data for asset \"{id}\" ({})", asset.path));
            }
            continue;
        };
        zip.start_file(asset.path.as_str(), stored).map_err(zipped)?;
        zip.write_all(bytes).map_err(io)?;
    }
    Ok(zip.finish().map_err(zipped)?.into_inner())
}

/// Unpack `.cardsmith` zip bytes. Fails if an asset the document names is absent.
pub fn load_cardsmith(bytes: &[u8]) -> Result<(Doc, AssetBytes)> {
    load(bytes, true)
}

/// As [`load_cardsmith`], but an asset the file does not hold is simply not
/// loaded: its layers draw nothing until its data is supplied.
pub fn load_cardsmith_available(bytes: &[u8]) -> Result<(Doc, AssetBytes)> {
    load(bytes, false)
}

fn load(bytes: &[u8], whole: bool) -> Result<(Doc, AssetBytes)> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| err!("Not a .cardsmith file (it is not a zip archive)"))?;
    let mut read = |name: &str| -> Option<Vec<u8>> {
        let mut file = zip.by_name(name).ok()?;
        let mut out = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut out).ok()?;
        Some(out)
    };
    let json = read("doc.json").ok_or_else(|| err!("Not a .cardsmith file (no doc.json inside)"))?;
    let doc = parse_doc(std::str::from_utf8(&json).map_err(|_| invalid("doc.json is not UTF-8".into()))?)?;
    let mut assets = AssetBytes::new();
    for (id, asset) in &doc.assets {
        match read(&asset.path) {
            Some(data) => {
                assets.insert(id.clone(), data);
            }
            None if whole => return Err(err!("Save file is missing asset \"{id}\" ({})", asset.path)),
            None => {}
        }
    }
    Ok((doc, assets))
}

/// True if the bytes look like a zip archive (and so a `.cardsmith` file) instead of bare JSON.
pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK")
}
