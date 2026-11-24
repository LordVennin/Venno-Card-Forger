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
//! Only templates whose frame is pre-drawn in layers per color ("raster"
//! templates, such as Proxyshop's `normal` and `normal-extended`) are built so
//! far. Not done yet: the set symbol, watermarks, and vector templates.

use cardsmith_cards::layout::CardData;
use cardsmith_cards::rules_text::{format_rules_text, strip_reminder_text, RulesFormat};
use cardsmith_core::commands::{apply, Edit};
use cardsmith_core::doc::{Doc, FitMode, Layer, RichText, TextLayer, TextRun, VerticalAlign};
use cardsmith_core::text::{layout_text, Fonts, TextLayout};
use cardsmith_core::tree;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

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
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Show the divider between rules and flavor text.
    pub flavor_divider: bool,
    pub remove_flavor: bool,
    pub remove_reminder: bool,
    /// Frame the art in the full-art area whenever the frame is a full-art one.
    pub vertical_fullart: bool,
    pub art: Option<Art>,
    /// PostScript names of the fonts for italic text, bold rules text and symbols.
    pub font_italic: String,
    pub font_bold: String,
    pub font_symbol: String,
}

impl Default for Options {
    fn default() -> Options {
        let fonts = RulesFormat::for_dpi(72.0);
        Options { flavor_divider: true, remove_flavor: false, remove_reminder: false, vertical_fullart: false, art: None, font_italic: fonts.font_italic, font_bold: fonts.font_bold, font_symbol: fonts.font_symbol }
    }
}

/// What building a card gives back.
#[derive(Clone, Debug)]
pub struct Built {
    /// One step that turns the template into the card. Apply it with the core's command system.
    pub edit: Edit,
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

struct Builder<'a> {
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
            _ => None,
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

/// Work out the edits that make `card` out of `template`.
///
/// `fonts` are used to measure text (a name shrinks to stay clear of the mana
/// cost; rules text moves up to stay clear of the P/T box). Without them the
/// text is still filled in, unfitted, and a warning says which font is missing.
pub fn build_card(template: &Template, card: &CardData, fonts: &Fonts, options: &Options) -> Result<Built, String> {
    let b = &template.bindings;
    if b.kind != "raster" {
        return Err(format!("\"{}\" is a {} template; only raster templates can be built so far.", template.id, if b.kind.is_empty() { "unbound" } else { &b.kind }));
    }
    let mut build = Builder { template, fonts, doc: template.doc.clone(), edits: Vec::new(), warnings: Vec::new() };
    let frame = &card.frame;
    // Photoshop lengths in Proxyshop are for a 3264 px wide card.
    let scale = template.doc.width as f64 / 3264.0;
    let step = 0.4 * template.doc.dpi / 72.0;

    // ---- frame layers ----------------------------------------------------------------

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

    // ---- collector line --------------------------------------------------------------

    let collector_line = card.collector_data();
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
    }
    let shifted = (!card.color_indicator.is_empty()).then(|| build.bound(&b.text, "typeShift")).flatten();
    if let (Some(shift), Some(usual)) = (&shifted, build.bound(&b.text, "type")) {
        build.hide(&usual);
        build.show(shift);
    }
    if let Some(id) = shifted.or_else(|| build.bound(&b.text, "type")) {
        let right = match (build.bound(&b.text_refs, "expansion"), card.is_basic_land) {
            (Some(reference), false) => build.area(&reference).map(|a| a[0]),
            _ => None,
        };
        fitted_line(&mut build, &id, &card.type_line, "card.type", right);
    }

    // ---- rules text and P/T ----------------------------------------------------------

    let oracle = if options.remove_reminder { strip_reminder_text(&card.oracle_text) } else { card.oracle_text.clone() };
    let flavor = if options.remove_flavor { String::new() } else { card.flavor_text.clone() };
    let divider = build.bound(&b.layers, "divider").filter(|_| options.flavor_divider && !oracle.is_empty() && !flavor.is_empty());
    // A few words alone in the box are centered.
    let centered = flavor.chars().count() <= 1 && oracle.chars().count() <= 70 && !oracle.contains('\n');
    let rules = build.bound(&b.text, if card.is_creature { "rulesCreature" } else { "rulesNoncreature" });
    let textbox = build.bound(&b.text_refs, "textbox").and_then(|id| build.area(&id));
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

        // Lift the text clear of the P/T box if its last lines run into it.
        let pt_area = card.is_creature.then(|| build.bound(&b.text_refs, "pt")).flatten().and_then(|id| build.area(&id));
        if let (Some(pt), Some(layout)) = (pt_area, build.layout(id)) {
            let lowest = layout.line_ink.iter().flatten().filter(|ink| ink[2] > pt[0] && ink[0] < pt[2] && ink[3] > pt[1] && ink[1] < pt[3]).map(|ink| ink[3]).fold(f64::MIN, f64::max);
            if lowest > pt[1] {
                area["y"] = json!(area["y"].as_f64().unwrap_or(0.0) + (pt[1] - lowest).floor());
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
    if card.is_creature {
        if let Some(id) = build.bound(&b.text, "pt") {
            let more = if pt_white { json!({ "style": build.text(&id).map(|t| { let mut s = t.style; s.color = [1.0, 1.0, 1.0, 1.0]; s }) }) } else { json!({}) };
            build.write(&id, &plain(&format!("{}/{}", card.power, card.toughness)), "card.pt", more);
        }
    }

    // ---- art -------------------------------------------------------------------------

    if let Some(art) = &options.art {
        // Tall art, or a full-art frame when asked, goes in the full-art area if the template has one.
        let tall = art.height as f64 > art.width as f64 * 1.1;
        let fullart = frame.is_colorless && options.vertical_fullart;
        let reference = (tall || fullart).then(|| build.bound(&b.art_refs, "fullArt")).flatten().or_else(|| build.bound(&b.art_refs, "normal")).and_then(|id| build.area(&id));
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
                    Ok(layer) => build.edit(Edit::AddLayer { parent, layer, index: Some(index) }),
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

    Ok(Built { edit: Edit::Batch { edits: build.edits }, warnings: build.warnings })
}
