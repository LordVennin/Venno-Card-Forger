// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported from Proxyshop src/templates/_core.py (BaseTemplate, StarterTemplate,
// NormalTemplate: which layers are switched on, how text layers are filled
// and fitted, collector info, art reference choice), src/templates/_cosmetic.py
// (NyxMod, CompanionMod), src/text_layers.py (ScaledTextField,
// FormattedTextArea) and src/helpers/position.py (frame_layer,
// position_between_layers, clear_reference_vertical).

//! The card engine: it takes an imported template and a card, and works out
//! the edits that turn the one into the other. Switch on the right frame
//! layers, fill and fit the text, place the art.
//!
//! The result is an [`Edit`], not pixels and not a new kind of document
//! (rules 1 and 3): applying it gives an ordinary document in which every
//! layer can still be edited, and undoing it gives the template back.
//!
//! Two kinds of template are built: ones whose frame is pre-drawn in layers
//! per color ("raster", such as Proxyshop's `normal` and `normal-extended`),
//! and Proxyshop's borderless vector template, whose shapes are colored here
//! (see `vector.rs`). Not done yet: the set symbol and watermarks.

use cardsmith_cards::layout::CardData;
use cardsmith_cards::rules_text::{format_rules_text, strip_reminder_text, RulesFormat};
use cardsmith_core::commands::{apply, Edit};
use cardsmith_core::doc::{Doc, FitMode, Layer, RichText, TextLayer, TextRun, VerticalAlign};
use cardsmith_core::text::{layout_text, Fonts, TextLayout};
use cardsmith_core::tree;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub mod svg;
mod vector;

/// Which of a template's layers play which part. Written by the importer into
/// the template's manifest.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Bindings {
    /// "raster": colors are pre-drawn layers. "vector": shapes are recolored.
    pub kind: String,
    pub layouts: Vec<String>,
    /// Areas the art can be framed in: "normal", "fullArt".
    pub art_refs: BTreeMap<String, String>,
    pub groups: BTreeMap<String, String>,
    pub text: BTreeMap<String, String>,
    pub text_refs: BTreeMap<String, String>,
    /// Sets of layers of which one is shown, by what it is for and then by color.
    pub toggles: BTreeMap<String, BTreeMap<String, String>>,
    pub layers: BTreeMap<String, String>,
    /// The template's placeholder for the art.
    pub art: Option<String>,
    /// "always" for templates whose art never reaches the edges of what shows (extended and borderless frames).
    pub art_fill: Option<String>,
}

/// An imported template: its document and what its layers are for.
#[derive(Clone, Debug)]
pub struct Template {
    pub id: String,
    pub doc: Doc,
    pub bindings: Bindings,
}

impl Template {
    /// From a template package's `doc.json` and `manifest.json`.
    pub fn from_json(doc: &str, manifest: &str) -> Result<Template, String> {
        let doc = cardsmith_core::serialize::parse_doc(doc).map_err(|e| e.to_string())?;
        let manifest: Value = serde_json::from_str(manifest).map_err(|e| format!("manifest.json: {e}"))?;
        let bindings = serde_json::from_value(manifest["bindings"].clone()).map_err(|e| format!("manifest.json bindings: {e}"))?;
        Ok(Template { id: manifest["id"].as_str().unwrap_or("template").to_string(), doc, bindings })
    }
}

/// The picture for a card: an image the caller has, or will, put in the document's assets.
#[derive(Clone, Debug, PartialEq)]
pub struct Art {
    /// The asset id to register it under.
    pub asset: String,
    /// Where the image is, as an asset path.
    pub path: String,
    pub width: u32,
    pub height: u32,
}

/// Choices Proxyshop offers as settings.
#[derive(Clone, Debug)]
pub struct Options {
    /// Show the divider between rules and flavor text.
    pub flavor_divider: bool,
    pub remove_flavor: bool,
    pub remove_reminder: bool,
    /// Frame the art in the full-art area whenever the frame is a full-art one.
    pub vertical_fullart: bool,
    pub art: Option<Art>,
    /// The set symbol, already in the rarity's colors. Drawn at the right of the type line.
    pub symbol: Option<svg::SvgArt>,
    /// The land-type symbol drawn large in a basic land's textbox in place of its rules text.
    pub basic_watermark: Option<svg::SvgArt>,
    /// How many cards the set has, for a collector line like "012/280 R".
    pub set_size: Option<u32>,
    /// PostScript names of the fonts for italic text, bold rules text and symbols.
    pub font_italic: String,
    pub font_bold: String,
    pub font_symbol: String,
}

impl Default for Options {
    fn default() -> Options {
        let fonts = RulesFormat::for_dpi(72.0);
        Options { flavor_divider: true, remove_flavor: false, remove_reminder: false, vertical_fullart: false, art: None, symbol: None, basic_watermark: None, set_size: None, font_italic: fonts.font_italic, font_bold: fonts.font_bold, font_symbol: fonts.font_symbol }
    }
}

/// Empty space around the art that should be filled with made-up picture.
///
/// The engine only says that it should be done and where; making the picture
/// needs the art's pixels and real work, which is the caller's (see the
/// `cardsmith-fill` crate). The caller then asks for the edit that puts the
/// result in the document with [`FillPlan::edit`].
#[derive(Clone, Debug, PartialEq)]
pub struct FillPlan {
    /// The art to carry on from, and where it sits: document position of its top-left corner, and its scale.
    pub art_asset: String,
    pub art_position: (f64, f64),
    pub art_scale: f64,
    /// Document size in pixels.
    pub canvas: (u32, u32),
    /// The part of the art that is trusted (left, top, right, bottom): the art
    /// where it shows, less a margin at its edges, where scans often have a
    /// border. Everything outside this is to be filled.
    pub keep: [f64; 4],
    parent: String,
    index: usize,
}

impl FillPlan {
    /// The edit that adds a finished fill: an image of the whole card area,
    /// one pixel of it covering `scale` document pixels, on its own layer
    /// just beneath the art. `source` names what made it ("patchmatch") and
    /// `params` records how (seed, size), so it can be made again differently.
    pub fn edit(&self, asset: &str, path: &str, width: u32, height: u32, scale: f64, source: &str, params: Value) -> Result<Edit, String> {
        let layer: Layer = serde_json::from_value(json!({
            "kind": "fill", "id": "card-fill", "name": "Art Fill", "visible": true, "opacity": 1, "blend": "normal", "clip": false, "role": "card.fill",
            "source": source, "params": params, "asset": asset, "transform": [scale, 0.0, 0.0, 0.0, scale, 0.0]
        }))
        .map_err(|e| e.to_string())?;
        let image = serde_json::from_value(json!({ "kind": "image", "path": path, "width": width, "height": height })).map_err(|e| e.to_string())?;
        Ok(Edit::Batch { edits: vec![Edit::SetAsset { id: asset.to_string(), asset: Some(image) }, Edit::AddLayer { parent: self.parent.clone(), layer, index: Some(self.index) }] })
    }
}

/// What building a card gives back.
#[derive(Clone, Debug)]
pub struct Built {
    /// One step that turns the template into the card. Apply it with the core's command system.
    pub edit: Edit,
    /// Set when the art leaves space that should be filled.
    pub fill: Option<FillPlan>,
    /// Things that could not be done, in plain words: a color the template has no layer for, a font that is not loaded.
    pub warnings: Vec<String>,
}

/// The bounds of a path whose coordinates are all absolute, as the importer writes them: (left, top, right, bottom).
fn path_bounds(d: &str) -> Option<[f64; 4]> {
    let numbers: Vec<f64> = d.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e')).filter_map(|n| n.parse().ok()).collect();
    let mut bounds: Option<[f64; 4]> = None;
    for pair in numbers.chunks_exact(2) {
        bounds = Some(match bounds {
            None => [pair[0], pair[1], pair[0], pair[1]],
            Some(b) => [b[0].min(pair[0]), b[1].min(pair[1]), b[2].max(pair[0]), b[3].max(pair[1])],
        });
    }
    bounds
}

/// Replace the first occurrence of `find` in a text layer's runs. The run keeps its style.
fn replace_text(content: &mut RichText, find: &str, with: &str) -> bool {
    for run in &mut content.runs {
        if run.text.contains(find) {
            run.text = run.text.replacen(find, with, 1);
            return true;
        }
    }
    false
}

pub(crate) struct Builder<'a> {
    template: &'a Template,
    fonts: &'a Fonts,
    /// The template's document with the edits so far applied, for measuring.
    doc: Doc,
    edits: Vec<Edit>,
    warnings: Vec<String>,
}

impl Builder<'_> {
    fn warn(&mut self, message: String) {
        if !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }

    fn edit(&mut self, edit: Edit) {
        match apply(&mut self.doc, &edit) {
            Ok(_) => self.edits.push(edit),
            Err(e) => self.warn(format!("An edit was refused: {e}")),
        }
    }

    fn set(&mut self, id: &str, props: Value) {
        let Value::Object(props) = props else { return };
        self.edit(Edit::SetLayerProps { id: id.to_string(), props });
    }

    fn show(&mut self, id: &str) {
        self.set(id, json!({ "visible": true }));
    }

    fn hide(&mut self, id: &str) {
        self.set(id, json!({ "visible": false }));
    }

    fn bound(&self, map: &BTreeMap<String, String>, key: &str) -> Option<String> {
        map.get(key).filter(|id| tree::find(&self.doc, id).is_some()).cloned()
    }

    /// The layer of a toggle set for a color, with a warning when the template has none.
    fn toggle(&mut self, set: &str, key: &str) -> Option<String> {
        let found = self.template.bindings.toggles.get(set).and_then(|layers| self.bound(layers, key));
        if found.is_none() && self.template.bindings.toggles.contains_key(set) {
            self.warn(format!("The template has no \"{key}\" layer among its {set} layers."));
        }
        found
    }

    fn text(&self, id: &str) -> Option<TextLayer> {
        match tree::find(&self.doc, id)? {
            Layer::Text(text) => Some(text.clone()),
            _ => None,
        }
    }

    fn layout(&mut self, id: &str) -> Option<TextLayout> {
        let layout = layout_text(&self.text(id)?, self.fonts);
        for font in &layout.missing {
            self.warn(format!("The font \"{font}\" is not loaded, so text set in it cannot be measured or drawn."));
        }
        Some(layout)
    }

    /// The area a reference layer marks: a shape's path, or a text layer's ink.
    fn area(&mut self, id: &str) -> Option<[f64; 4]> {
        match tree::find(&self.doc, id)? {
            Layer::Shape(shape) => path_bounds(&shape.path.d),
            Layer::Text(_) => self.layout(id)?.ink,
            // An image that is only moved, not scaled: where its pixels are.
            Layer::Pixel(pixel) => {
                let asset = self.doc.assets.get(&pixel.asset)?;
                let (x, y) = (pixel.transform[2], pixel.transform[5]);
                Some([x, y, x + asset.width? as f64, y + asset.height? as f64])
            }
            _ => None,
        }
    }

    /// A layer found by the names of the groups it is in, under a starting
    /// layer (the document's top level when `under` is None). This is how
    /// Proxyshop's own scripts find layers.
    fn named(&self, under: Option<&str>, names: &[&str]) -> Option<String> {
        let mut group = match under {
            Some(id) => match tree::find(&self.doc, id)? {
                Layer::Group(g) => g,
                _ => return None,
            },
            None => &self.doc.root,
        };
        let (last, groups) = names.split_last()?;
        for name in groups {
            group = group.children.iter().find_map(|c| match c {
                Layer::Group(g) if g.base.name == *name => Some(g),
                _ => None,
            })?;
        }
        group.children.iter().find(|c| c.base().name == *last).map(|c| c.id().to_string())
    }

    /// Move a layer, and everything in it, by a number of pixels.
    fn translate(&mut self, id: &str, dx: f64, dy: f64) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        let Some(layer) = tree::find(&self.doc, id).cloned() else { return };
        let moved_path = |d: &str| kurbo::BezPath::from_svg(d).ok().map(|mut path| {
            path.apply_affine(kurbo::Affine::translate((dx, dy)));
            svg::path_data(&path)
        });
        let mut props = json!({});
        let base = layer.base();
        if let Some(mask) = &base.mask {
            let mut mask = serde_json::to_value(mask).unwrap_or_default();
            mask["offset"] = json!({ "x": base.mask.as_ref().map_or(0.0, |m| m.offset.x) + dx, "y": base.mask.as_ref().map_or(0.0, |m| m.offset.y) + dy });
            props["mask"] = mask;
        }
        if let Some(vector) = &base.vector_mask {
            if let Some(d) = moved_path(&vector.path.d) {
                let mut vector = serde_json::to_value(vector).unwrap_or_default();
                vector["path"]["d"] = json!(d);
                props["vectorMask"] = vector;
            }
        }
        match &layer {
            Layer::Group(group) => {
                for child in &group.children {
                    self.translate(child.id(), dx, dy);
                }
            }
            Layer::Shape(shape) => {
                if let Some(d) = moved_path(&shape.path.d) {
                    let mut path = serde_json::to_value(&shape.path).unwrap_or_default();
                    path["d"] = json!(d);
                    props["path"] = path;
                }
            }
            Layer::Text(text) => {
                props["box"] = json!({ "x": text.bounds.x + dx, "y": text.bounds.y + dy, "w": text.bounds.w, "h": text.bounds.h });
                if let Some(origin) = text.origin {
                    props["origin"] = json!({ "x": origin.x + dx, "y": origin.y + dy });
                }
            }
            Layer::Pixel(l) => props["transform"] = json!([l.transform[0], l.transform[1], l.transform[2] + dx, l.transform[3], l.transform[4], l.transform[5] + dy]),
            Layer::Art(l) => props["transform"] = json!([l.transform[0], l.transform[1], l.transform[2] + dx, l.transform[3], l.transform[4], l.transform[5] + dy]),
            Layer::Fill(l) => props["transform"] = json!([l.transform[0], l.transform[1], l.transform[2] + dx, l.transform[3], l.transform[4], l.transform[5] + dy]),
        }
        if props.as_object().is_some_and(|p| !p.is_empty()) {
            self.set(id, props);
        }
    }

    fn parent_of(&self, id: &str) -> Option<String> {
        let path = tree::locate(&self.doc, id)?;
        let (_, parent) = path.split_last()?;
        // A layer at the top level has no group around it.
        (!parent.is_empty()).then(|| tree::layer_at(&self.doc, parent).id().to_string())
    }

    /// Switch a layer's mask on, if it has one.
    fn enable_mask(&mut self, id: &str) {
        let Some(layer) = tree::find(&self.doc, id) else { return };
        let Some(mut mask) = serde_json::to_value(layer).ok().map(|v| v["mask"].clone()).filter(Value::is_object) else { return };
        mask["enabled"] = json!(true);
        self.set(id, json!({ "mask": mask }));
    }

    /// Put new runs on a text layer and say what part of the card it is.
    fn write(&mut self, id: &str, content: &RichText, role: &str, more: Value) {
        let mut props = json!({ "content": content, "role": role, "visible": true });
        if let (Value::Object(props), Value::Object(more)) = (&mut props, more) {
            props.extend(more);
        }
        self.set(id, props);
    }
}

fn plain(text: &str) -> RichText {
    RichText { runs: vec![TextRun { text: text.to_string(), ..TextRun::default() }], paragraphs: Vec::new() }
}

/// What the frame step settled for the steps after it.
pub(crate) struct Plan {
    /// The P/T numbers sit on a dark box and are drawn white.
    pub pt_white: bool,
    /// The rules text layer to fill.
    pub rules: Option<String>,
    /// The area rules text is fitted to and centered in.
    pub textbox: Option<[f64; 4]>,
    /// The layer that marks where normal (not tall) art goes.
    pub art_ref: Option<String>,
    /// How far the type line moves down from where the template has it.
    pub type_dy: f64,
    /// Switch on the drop shadows of the name, type line and P/T, and of the rules text.
    pub shadows: bool,
    pub shadow_rules: bool,
    /// Whether rules text is drawn at all.
    pub textless: bool,
    /// Sideways nudges of the type line and the set symbol.
    pub type_dx: f64,
    pub symbol_dx: f64,
    /// Power and toughness are written at the end of the type line instead of in a box.
    pub pt_in_type: bool,
    /// How far the P/T numbers move to sit in a differently placed box.
    pub pt_shift: (f64, f64),
    /// Size of the bevel on a basic land's watermark.
    pub watermark_bevel: f64,
}

/// Frame layers of a template whose colors are pre-drawn layers.
fn frame_raster(build: &mut Builder, card: &CardData) -> Plan {
    let b = &build.template.bindings;
    let frame = &card.frame;
    // Name and type boxes.
    if let Some(id) = build.toggle("twins", &frame.twins) {
        build.show(&id);
    }

    // P/T box. A vehicle's is dark, so its numbers are white.
    let vehicle = card.is_vehicle && frame.background == "Vehicle";
    let mut pt_white = false;
    if card.is_creature {
        let own = vehicle.then(|| b.toggles.get("ptBox").and_then(|set| build.bound(set, "Vehicle"))).flatten();
        pt_white = own.is_some();
        if let Some(id) = own.or_else(|| build.toggle("ptBox", &frame.twins)) {
            build.show(&id);
        }
    }

    // Pinlines and textbox: lands have their own set.
    let pinlines = build.toggle(if card.is_land { "landPinlines" } else { "pinlines" }, &frame.pinlines);
    if let Some(id) = &pinlines {
        build.show(id);
    }

    // Color indicator, for cards whose color is not in their cost.
    if !card.color_indicator.is_empty() {
        if let Some(id) = build.toggle("colorIndicator", &card.color_indicator) {
            build.show(&id);
        }
    }

    // Background. Only vehicles get the vehicle one; enchantment creatures get the starry Nyx one where there is one.
    let background = if !card.is_vehicle && frame.background == "Vehicle" { "Artifact" } else { frame.background.as_str() };
    let nyx = card.is_nyx.then(|| b.toggles.get("nyx").and_then(|set| build.bound(set, background))).flatten();
    let plain_background = if vehicle { b.toggles.get("background").and_then(|set| build.bound(set, "Vehicle").or_else(|| build.bound(set, "Artifact"))) } else { None };
    if let Some(id) = nyx.or(plain_background).or_else(|| build.toggle("background", background)) {
        build.show(&id);
    }

    // Legendary crown, and the border that makes room for it.
    if card.is_legendary {
        if let Some(crown) = build.toggle("crown", &frame.pinlines) {
            build.show(&crown);
            if let Some(borders) = b.toggles.get("border") {
                if let (Some(normal), Some(legendary)) = (build.bound(borders, "Normal Border"), build.bound(borders, "Legendary Border")) {
                    build.hide(&normal);
                    build.show(&legendary);
                }
            }
            // Nyx and companion cards have a hollow crown: masks cut the frame away behind it.
            if card.is_nyx || card.is_companion {
                if let Some(shadows) = build.bound(&b.layers, "shadows") {
                    build.enable_mask(&shadows);
                }
                for inside in [Some(&crown), pinlines.as_ref()].into_iter().flatten() {
                    if let Some(group) = build.parent_of(inside) {
                        build.enable_mask(&group);
                    }
                }
                if let Some(shadow) = build.bound(&b.layers, "crownShadow") {
                    build.show(&shadow);
                }
            }
            if card.is_companion {
                if let Some(id) = b.toggles.get("companion").and_then(|set| build.bound(set, &frame.pinlines)) {
                    build.show(&id);
                }
            }
        }
    }


    let rules = build.bound(&b.text, if card.is_creature { "rulesCreature" } else { "rulesNoncreature" });
    let textbox = build.bound(&b.text_refs, "textbox").and_then(|id| build.area(&id));
    Plan { pt_white, rules, textbox, art_ref: build.bound(&b.art_refs, "normal"), type_dy: 0.0, shadows: false, shadow_rules: false, textless: false, type_dx: 0.0, symbol_dx: 0.0, pt_in_type: false, pt_shift: (0.0, 0.0), watermark_bevel: 28.0 }
}

/// Work out the edits that make `card` out of `template`.
///
/// `fonts` are used to measure text (a name shrinks to stay clear of the mana
/// cost; rules text moves up to stay clear of the P/T box). Without them the
/// text is still filled in, unfitted, and a warning says which font is missing.
pub fn build_card(template: &Template, card: &CardData, fonts: &Fonts, options: &Options) -> Result<Built, String> {
    let b = &template.bindings;
    if b.kind != "raster" && b.kind != "vector" {
        return Err(format!("\"{}\" has no layer bindings the card engine understands; import it again with `pnpm import:psd`.", template.id));
    }
    let mut build = Builder { template, fonts, doc: template.doc.clone(), edits: Vec::new(), warnings: Vec::new() };
    let frame = &card.frame;
    // Photoshop lengths in Proxyshop are for a 3264 px wide card.
    let scale = template.doc.width as f64 / 3264.0;
    let step = 0.4 * template.doc.dpi / 72.0;

    // ---- frame layers ----------------------------------------------------------------

    let plan = if b.kind == "vector" { vector::frame(&mut build, card, options) } else { frame_raster(&mut build, card) };

    // ---- collector line --------------------------------------------------------------

    let collector_line = card.collector_line(options.set_size);
    let collector = build.bound(&b.text, "collector");
    let (artist, set) = (build.bound(&b.text, "artist"), build.bound(&b.text, "set"));
    let fill_basic = |build: &mut Builder, layer: &str, find: &str, with: &str| {
        if let Some(mut text) = build.text(layer) {
            if replace_text(&mut text.content, find, with) {
                build.set(layer, json!({ "content": text.content }));
            }
        }
    };
    let children: Vec<(String, String)> = collector.as_deref().and_then(|id| tree::find(&build.doc, id)).map(|group| match group {
        Layer::Group(g) => g.children.iter().map(|c| (c.base().name.clone(), c.id().to_string())).collect(),
        _ => Vec::new(),
    }).unwrap_or_default();
    let child = |name: &str| children.iter().find(|(n, _)| n == name).map(|(_, id)| id.clone());
    match (collector.as_deref(), child("Top"), child("Bottom")) {
        // Number and rarity above; set, language and artist below.
        (Some(group), Some(top), Some(bottom)) if !collector_line.is_empty() => {
            for id in [&artist, &set].into_iter().flatten() {
                build.hide(id);
            }
            build.show(group);
            build.write(&top, &plain(&collector_line), "card.collector", json!({}));
            if card.lang != "EN" {
                fill_basic(&mut build, &bottom, "EN", &card.lang);
            }
            fill_basic(&mut build, &bottom, "SET", &card.set);
            fill_basic(&mut build, &bottom, "Artist", &card.artist);
            build.set(&bottom, json!({ "role": "card.legal" }));
        }
        // No collector number: set and artist only.
        _ => {
            if let Some(id) = &artist {
                fill_basic(&mut build, id, "Artist", &card.artist);
                build.set(id, json!({ "role": "card.artist" }));
            }
            if let Some(id) = &set {
                if card.lang != "EN" {
                    fill_basic(&mut build, id, "EN", &card.lang);
                }
                if let Some(mut text) = build.text(id) {
                    if let Some(first) = text.content.runs.first_mut() {
                        first.text.insert_str(0, &card.set);
                    }
                    build.set(id, json!({ "content": text.content, "role": "card.set" }));
                }
            }
        }
    }

    // ---- name, mana cost, type line --------------------------------------------------

    let format = RulesFormat { font_italic: options.font_italic.clone(), font_bold: options.font_bold.clone(), font_symbol: options.font_symbol.clone(), ..RulesFormat::for_dpi(template.doc.dpi) };
    let gap = (scale * 30.0).floor();
    let mana = build.bound(&b.text, "mana");
    if let Some(id) = &mana {
        let symbols = format_rules_text(&card.mana_cost, "", &format);
        for symbol in &symbols.unknown_symbols {
            build.warn(format!("The mana cost has a symbol there is no glyph for: {symbol}"));
        }
        build.write(id, &RichText { runs: symbols.content.runs, paragraphs: Vec::new() }, "card.mana", json!({}));
    }

    // A line of text that shrinks to stay `gap` clear of whatever is to its right.
    let fitted_line = |build: &mut Builder, id: &str, text: &str, role: &str, right: Option<f64>| {
        let mut more = json!({});
        if let (Some(right), Some(layer)) = (right, build.text(id)) {
            let left = layer.origin.map_or(layer.bounds.x, |o| o.x) - layer.style.size;
            // Nothing to stay clear of when the reference is to the left of the text.
            if right - gap > left + layer.style.size {
                more = json!({ "box": { "x": left, "y": layer.bounds.y, "w": right - gap - left, "h": layer.bounds.h }, "fit": "shrinkWidth", "fitStep": step });
            }
        }
        build.write(id, &plain(text), role, more);
    };
    if let Some(id) = build.bound(&b.text, "name") {
        // Basic lands have no mana cost to avoid.
        let right = match (&mana, card.is_basic_land) {
            (Some(mana), false) => build.layout(mana).and_then(|l| l.ink).map(|ink| ink[0]).or_else(|| build.text(mana).and_then(|t| t.origin).map(|o| o.x)),
            _ => None,
        };
        fitted_line(&mut build, &id, &card.name, "card.name", right);
        if plan.shadows {
            build.set(&id, json!({ "effectsEnabled": true }));
        }
    }
    let shifted = (!card.color_indicator.is_empty()).then(|| build.bound(&b.text, "typeShift")).flatten();
    if let (Some(shift), Some(usual)) = (&shifted, build.bound(&b.text, "type")) {
        build.hide(&usual);
        build.show(shift);
    }
    // The set symbol: as tall as its reference area, against the area's right edge, centered on its height.
    let expansion = build.bound(&b.text_refs, "expansion");
    let symbol_area = expansion.as_deref().and_then(|id| build.area(id)).map(|a| [a[0] + plan.symbol_dx, a[1] + plan.type_dy, a[2] + plan.symbol_dx, a[3] + plan.type_dy]);
    let mut symbol_left = None;
    if let (Some(symbol), Some(area), Some(reference)) = (&options.symbol, symbol_area, &expansion) {
        for skipped in &symbol.unsupported {
            build.warn(format!("The set symbol uses something that is not drawn: {skipped}."));
        }
        let size = (area[3] - area[1]) / symbol.height;
        let left = area[2] - symbol.width * size;
        let layers = symbol.layers([size, 0.0, 0.0, size, left, area[1]], "card-symbol");
        let group = json!({ "kind": "group", "id": "card-symbol", "name": "Expansion Symbol", "visible": true, "opacity": 1, "blend": "normal", "clip": false, "role": "card.symbol", "passThrough": true, "children": layers });
        let at = tree::locate(&build.doc, reference).and_then(|path| path.split_last().map(|(at, parent)| (*at, parent.to_vec())));
        match (serde_json::from_value::<Layer>(group), at) {
            (Ok(layer), Some((at, parent))) => {
                let parent = if parent.is_empty() { build.doc.root.base.id.clone() } else { tree::layer_at(&build.doc, &parent).id().to_string() };
                build.edit(Edit::AddLayer { parent, layer, index: Some(at + 1) });
                symbol_left = Some(left);
            }
            (Err(e), _) => build.warn(format!("The set symbol could not be placed: {e}")),
            _ => {}
        }
    }
    if let Some(id) = shifted.or_else(|| build.bound(&b.text, "type")) {
        // The type line stops short of the symbol, or of where one would be.
        let right = if card.is_basic_land { None } else { symbol_left.or(symbol_area.map(|a| a[0])) };
        // Templates with several textbox heights keep one type line and move it.
        build.translate(&id, plan.type_dx, plan.type_dy);
        // With no box for them, power and toughness end the type line.
        let line = if plan.pt_in_type && card.is_creature { format!("{} — {}/{}", card.type_line, card.power, card.toughness) } else { card.type_line.clone() };
        fitted_line(&mut build, &id, &line, "card.type", right);
        if plan.shadows {
            build.set(&id, json!({ "effectsEnabled": true }));
        }
    }

    // ---- rules text and P/T ----------------------------------------------------------

    let oracle = if options.remove_reminder { strip_reminder_text(&card.oracle_text) } else { card.oracle_text.clone() };
    let flavor = if options.remove_flavor { String::new() } else { card.flavor_text.clone() };
    let divider = build.bound(&b.layers, "divider").filter(|_| options.flavor_divider && !oracle.is_empty() && !flavor.is_empty());
    // A few words alone in the box are centered.
    let centered = flavor.chars().count() <= 1 && oracle.chars().count() <= 70 && !oracle.contains('\n');
    // A basic land shows its land type's symbol, large, instead of rules text.
    let mut watermarked = false;
    if let (true, Some(mark), Some(area)) = (card.is_basic_land, &options.basic_watermark, plan.textbox) {
        let color = match frame.pinlines.as_str() {
            "W" => [248.0, 249.0, 243.0],
            "U" => [0.0, 115.0, 178.0],
            "B" => [6.0, 0.0, 0.0],
            "R" => [212.0, 39.0, 44.0],
            "G" => [1.0, 131.0, 69.0],
            _ => [165.0, 150.0, 132.0],
        };
        // Three quarters of the textbox's height, centered in it, in the land's color.
        let size = 0.75 * (area[3] - area[1]) / mark.height;
        let (left, top) = ((area[0] + area[2] - mark.width * size) / 2.0, (area[1] + area[3] - mark.height * size) / 2.0);
        let shapes: Vec<Value> = mark.layers([size, 0.0, 0.0, size, left, top], "card-watermark").iter().filter_map(|l| serde_json::to_value(l).ok()).map(|mut shape| {
            shape["fill"] = json!({ "type": "solid", "color": [color[0] / 255.0, color[1] / 255.0, color[2] / 255.0, 1.0] });
            shape.as_object_mut().map(|s| s.remove("effects"));
            shape
        }).collect();
        let tone = |c: f64, blend: &str, opacity: f64| json!({ "color": [c, c, c, 1], "blend": blend, "opacity": opacity });
        let group = json!({
            "kind": "group", "id": "card-watermark", "name": "Watermark", "visible": true, "opacity": 1, "blend": "normal", "clip": false, "role": "card.watermark", "passThrough": false, "children": shapes,
            "effects": [{ "type": "bevel", "enabled": true, "style": "inner", "technique": "chiselHard", "direction": "up", "depth": 1, "size": plan.watermark_bevel * scale, "soften": 14.0 * scale, "angle": 45, "altitude": 22, "highlight": tone(1.0, "screen", 0.70), "shadow": tone(0.0, "multiply", 0.72) }]
        });
        // Under the text, over the frame.
        let below = build.bound(&b.groups, "text").and_then(|id| tree::locate(&build.doc, &id)).and_then(|path| path.split_last().map(|(at, parent)| (*at, parent.to_vec())));
        match (serde_json::from_value::<Layer>(group), below) {
            (Ok(layer), Some((at, parent))) => {
                let parent = if parent.is_empty() { build.doc.root.base.id.clone() } else { tree::layer_at(&build.doc, &parent).id().to_string() };
                build.edit(Edit::AddLayer { parent, layer, index: Some(at) });
                watermarked = true;
            }
            (Err(e), _) => build.warn(format!("The basic land watermark could not be placed: {e}")),
            _ => {}
        }
    }
    let (rules, textbox) = (plan.rules.clone().filter(|_| !plan.textless && !watermarked), plan.textbox);
    if let (Some(id), Some(layer)) = (&rules, rules.as_deref().and_then(|id| build.text(id))) {
        let formatted = format_rules_text(&oracle, &flavor, &RulesFormat { centered, divider: divider.is_some(), ..format.clone() });
        for symbol in &formatted.unknown_symbols {
            build.warn(format!("The rules text has a symbol there is no glyph for: {symbol}"));
        }
        // As wide as the template's rules layer; as tall as the textbox less the room Proxyshop leaves spare, and centered in it.
        let spare = (scale * 64.0).floor();
        let area = textbox.map_or(layer.bounds.clone(), |t| cardsmith_core::doc::Rect { x: layer.bounds.x, y: t[1] + spare / 2.0, w: layer.bounds.w, h: (t[3] - t[1] - spare).max(1.0) });
        let mut area = json!({ "x": area.x, "y": area.y, "w": area.w, "h": area.h });
        let fit = json!({ "fit": FitMode::ShrinkHeight, "fitStep": step, "verticalAlign": VerticalAlign::Center });
        let mut props = fit.clone();
        props["box"] = area.clone();
        build.write(id, &formatted.content, "card.rules", props);
        if plan.shadow_rules {
            build.set(id, json!({ "effectsEnabled": true }));
        }

        // Lift the text clear of the P/T box if its last lines run into it.
        //
        // Proxyshop stops there, which pushes a full box of text up over the
        // type line. Here, if the lifted text would leave the top of the
        // textbox, the room it is fitted to is made shorter and it is fitted
        // again, until it clears the P/T box and stays inside.
        let pt_area = card.is_creature.then(|| build.bound(&b.text_refs, "pt")).flatten().and_then(|id| build.area(&id));
        if let Some(pt) = pt_area {
            // The same margin from the top that fitting leaves.
            let top = textbox.map_or(layer.bounds.y, |t| t[1] + spare / 2.0);
            let (start_y, mut height) = (area["y"].as_f64().unwrap_or(0.0), area["h"].as_f64().unwrap_or(1.0));
            for _ in 0..40 {
                let Some(layout) = build.layout(id) else { break };
                // The lowest ink inside the P/T area decides how far up the text must go.
                let lowest = layout.line_ink.iter().flatten().filter(|ink| ink[2] > pt[0] && ink[0] < pt[2] && ink[3] > pt[1] && ink[1] < pt[3]).map(|ink| ink[3].min(pt[3])).fold(f64::MIN, f64::max);
                if lowest <= pt[1] {
                    break;
                }
                let lift = (pt[1] - lowest).floor();
                let overflow = top - (layout.ink.map_or(top, |ink| ink[1]) + lift);
                if overflow <= 0.0 || height <= step {
                    area["y"] = json!(area["y"].as_f64().unwrap_or(0.0) + lift);
                    build.set(id, json!({ "box": area }));
                    break;
                }
                height = (height - overflow.max(step)).max(step);
                area["y"] = json!(start_y);
                area["h"] = json!(height);
                build.set(id, json!({ "box": area }));
            }
        }

        // The divider sits midway between the rules and the flavor text.
        if let (Some(divider), Some(at), Some(layout)) = (&divider, formatted.flavor_paragraph, build.layout(id)) {
            let above = layout.paragraph_ink[..at.min(layout.paragraph_ink.len())].iter().rev().flatten().next().copied();
            let below = layout.paragraph_ink.get(at..).and_then(|rest| rest.iter().flatten().next().copied());
            let height = match tree::find(&build.doc, divider) {
                Some(Layer::Pixel(pixel)) => build.doc.assets.get(&pixel.asset).and_then(|a| a.height).map(|h| (pixel.transform, h as f64)),
                _ => None,
            };
            if let (Some(above), Some(below), Some((transform, height))) = (above, below, height) {
                let middle = (above[3] + below[1]) / 2.0;
                let mut moved = transform;
                moved[5] = (middle - height / 2.0).round();
                build.set(divider, json!({ "transform": moved, "visible": true, "role": "card.divider" }));
            }
        }
    }
    if card.is_creature && !plan.pt_in_type {
        if let Some(id) = build.bound(&b.text, "pt") {
            build.translate(&id, plan.pt_shift.0, plan.pt_shift.1);
            let more = if plan.pt_white { json!({ "style": build.text(&id).map(|t| { let mut s = t.style; s.color = [1.0, 1.0, 1.0, 1.0]; s }) }) } else { json!({}) };
            build.write(&id, &plain(&format!("{}/{}", card.power, card.toughness)), "card.pt", more);
            if plan.shadows {
                build.set(&id, json!({ "effectsEnabled": true }));
            }
        }
    }

    // ---- art -------------------------------------------------------------------------

    let mut fill = None;
    if let Some(art) = &options.art {
        // Tall art, or a full-art frame when asked, goes in the full-art area if the template has one.
        let tall = art.height as f64 > art.width as f64 * 1.1;
        let fullart = (frame.is_colorless || b.kind == "vector") && options.vertical_fullart;
        let reference = (tall || fullart).then(|| build.bound(&b.art_refs, "fullArt")).flatten().or(plan.art_ref.clone()).and_then(|id| build.area(&id));
        match reference {
            Some(r) if art.width > 0 && art.height > 0 => {
                // Cover the area, centered.
                let (w, h) = (r[2] - r[0], r[3] - r[1]);
                let s = (w / art.width as f64).max(h / art.height as f64);
                let (tx, ty) = (r[0] + (w - art.width as f64 * s) / 2.0, r[1] + (h - art.height as f64 * s) / 2.0);
                build.edit(Edit::SetAsset { id: art.asset.clone(), asset: serde_json::from_value(json!({ "kind": "image", "path": art.path, "width": art.width, "height": art.height })).ok() });
                let layer: Result<Layer, _> = serde_json::from_value(json!({
                    "kind": "art", "id": "card-art", "name": "Art", "visible": true, "opacity": 1, "blend": "normal", "clip": false, "role": "card.art",
                    "frameRef": { "x": r[0], "y": r[1], "w": w, "h": h }, "asset": art.asset, "transform": [s, 0.0, tx, 0.0, s, ty], "fit": "cover"
                }));
                // Just above the template's own art placeholder, under the frame.
                let placeholder = b.art.as_deref().and_then(|id| tree::locate(&build.doc, id));
                let (parent, index) = match placeholder.as_deref().and_then(|p| p.split_last()) {
                    Some((at, [])) => (build.doc.root.base.id.clone(), at + 1),
                    Some((at, parent)) => (tree::layer_at(&build.doc, parent).id().to_string(), at + 1),
                    None => (build.doc.root.base.id.clone(), 0),
                };
                match layer {
                    Ok(layer) => {
                        build.edit(Edit::AddLayer { parent: parent.clone(), layer, index: Some(index) });
                        // Extended and borderless frames always show past the art; a full-art frame (a colorless card) does
                        // when its art sits in the ordinary art area.
                        let always = b.kind == "vector" || b.art_fill.as_deref() == Some("always") || template.id.ends_with("-extended");
                        let in_full_area = (tall || fullart) && build.bound(&b.art_refs, "fullArt").is_some();
                        if always || (frame.is_colorless && !in_full_area) {
                            let (cw, ch) = (template.doc.width as f64, template.doc.height as f64);
                            let shown = [tx.max(0.0), ty.max(0.0), (tx + art.width as f64 * s).min(cw), (ty + art.height as f64 * s).min(ch)];
                            // Proxyshop's selection: the art, pulled in 10 px and its corners rounded by 5, except along the card's own edges.
                            let (pull, round) = ((10.0 * scale).round(), (5.0 * scale).round());
                            let keep = [
                                if shown[0] > 0.0 { shown[0] + pull } else { 0.0 },
                                if shown[1] > 0.0 { shown[1] + pull } else { 0.0 },
                                if shown[2] < cw { shown[2] - pull } else { cw },
                                if shown[3] < ch { shown[3] - pull } else { ch },
                            ];
                            let covers = keep[0] <= 0.0 && keep[1] <= 0.0 && keep[2] >= cw && keep[3] >= ch;
                            if !covers && keep[2] - keep[0] > 2.0 * round && keep[3] - keep[1] > 2.0 * round {
                                // The art shows only where it is trusted; the fill underneath shows everywhere else.
                                let outline = kurbo::Shape::to_path(&kurbo::RoundedRect::new(keep[0], keep[1], keep[2], keep[3], round), 0.05);
                                build.set("card-art", json!({ "vectorMask": { "path": { "d": svg::path_data(&outline) } } }));
                                fill = Some(FillPlan { art_asset: art.asset.clone(), art_position: (tx, ty), art_scale: s, canvas: (template.doc.width, template.doc.height), keep, parent, index });
                            }
                        }
                    }
                    Err(e) => build.warn(format!("The art layer could not be made: {e}")),
                }
            }
            _ => build.warn("The template has no art area, so the art was not placed.".into()),
        }
    }

    // ---- the document knows what card it is -----------------------------------------

    let mut props = Map::new();
    props.insert("card".into(), serde_json::to_value(card).map_err(|e| e.to_string())?);
    props.insert("templateId".into(), json!(template.id));
    build.edit(Edit::SetDocProps { props });

    Ok(Built { edit: Edit::Batch { edits: build.edits }, fill, warnings: build.warnings })
}
