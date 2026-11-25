//! A card being made in an open document.
//!
//! [`build_card`](crate::build_card) turns a pristine template into a card.
//! In the editor the card is built again and again as its fields change, in a
//! document that has a history and that the user may have touched. The
//! workbench does that: each build is one undoable step that takes the
//! previous build out and puts the new one in, and a fill that arrives later
//! (they take seconds to make) is added only if the card it was made for is
//! still the one showing.
//!
//! Which build is showing is written in the document itself (`card._build`),
//! so undo and redo need no bookkeeping here.

use crate::{build_card, svg, Art, FillPlan, Options, Template};
use cardsmith_cards::layout::{card_data, CardData, CardInput};
use cardsmith_core::commands::{apply, Edit};
use cardsmith_core::doc::Doc;
use cardsmith_core::render::decode_image;
use cardsmith_core::session::Session;
use cardsmith_core::tree;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// What to build: the card, and what the engine cannot work out from it.
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BuildRequest {
    /// The card, ready for the engine...
    #[serde(default)]
    pub card: Option<CardData>,
    /// ...or Scryfall's JSON for it (as looked up, or as edited in the card
    /// form), from which the card is read here, so that the frame follows
    /// whatever was changed. `name` says which face of a two-faced card.
    #[serde(default)]
    pub scryfall: Option<Value>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub creator: String,
    /// The set symbol as SVG text, already the one for the card's rarity.
    #[serde(default)]
    pub symbol_svg: Option<String>,
    /// A basic land's land-type symbol as SVG text.
    #[serde(default)]
    pub watermark_svg: Option<String>,
    #[serde(default)]
    pub set_size: Option<u32>,
    /// Whether the card's text casts a shadow; left out, the template decides.
    #[serde(default)]
    pub text_shadows: Option<bool>,
    #[serde(default)]
    pub remove_flavor: bool,
    #[serde(default)]
    pub remove_reminder: bool,
    #[serde(default)]
    pub hide_flavor_divider: bool,
    #[serde(default)]
    pub vertical_fullart: bool,
}

/// Where the art sits and what of it to trust, for whoever makes the fill.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FillInfo {
    pub art_position: (f64, f64),
    pub art_scale: f64,
    pub canvas: (u32, u32),
    pub keep: [f64; 4],
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Names this build. Give it back with the fill made for it.
    pub build: u64,
    pub warnings: Vec<String>,
    /// Present when the art leaves space that should be filled.
    pub fill: Option<FillInfo>,
    /// The art sits exactly where it did, so the fill already made for it was
    /// kept and no new one is needed (unless a different one is wanted).
    pub fill_kept: bool,
}

pub struct Workbench {
    template: Template,
    art: Option<Art>,
    /// How to take each build out again, and where its fill goes.
    inverses: BTreeMap<u64, Edit>,
    plans: BTreeMap<u64, FillPlan>,
    /// The document as each recent build left it, to tell afterwards what was changed by hand.
    built: BTreeMap<u64, Doc>,
    next: u64,
}

const FILL_LAYER: &str = "card-fill";

impl Workbench {
    pub fn new(template: Template) -> Workbench {
        Workbench { template, art: None, inverses: BTreeMap::new(), plans: BTreeMap::new(), built: BTreeMap::new(), next: 1 }
    }

    /// The document to open a session with: the template as imported.
    pub fn doc(&self) -> Doc {
        self.template.doc.clone()
    }

    pub fn template_id(&self) -> &str {
        &self.template.id
    }

    /// The build showing in a document, if any.
    fn showing(doc: &Doc) -> Option<u64> {
        doc.card.as_ref()?.get("_build")?.as_u64()
    }

    /// Use this picture as the card's art from the next build on. Returns its size.
    pub fn set_art(&mut self, session: &mut Session, bytes: Vec<u8>) -> Result<(u32, u32), String> {
        let image = decode_image(&bytes).ok_or("That file is not a picture this app can read (PNG, JPEG or WebP).")?;
        let id = format!("card-art-{}", self.next);
        self.next += 1;
        // Each picture gets its own asset, so undoing to a card built with an earlier one shows that one.
        session.stash_asset(&id, bytes);
        let size = (image.w as u32, image.h as u32);
        self.art = Some(Art { path: format!("assets/{id}.png"), asset: id, width: size.0, height: size.1 });
        Ok(size)
    }

    pub fn clear_art(&mut self) {
        self.art = None;
    }

    /// Build the card into the session's document as one undoable step,
    /// replacing the build that is showing, if one is.
    pub fn build(&mut self, session: &mut Session, request: &BuildRequest) -> Result<Report, String> {
        let card = match (&request.card, &request.scryfall) {
            (Some(card), _) => card.clone(),
            (None, Some(scryfall)) => {
                let name = if request.name.is_empty() { scryfall["name"].as_str().unwrap_or_default().split(" // ").next().unwrap_or_default().to_string() } else { request.name.clone() };
                card_data(scryfall, &CardInput { name, artist: request.artist.clone(), creator: request.creator.clone() })
            }
            (None, None) => return Err("There is no card to build.".into()),
        };
        let read = |svg: &Option<String>| svg.as_deref().and_then(|text| svg::parse_svg(text).ok());
        let options = Options {
            flavor_divider: !request.hide_flavor_divider,
            remove_flavor: request.remove_flavor,
            remove_reminder: request.remove_reminder,
            vertical_fullart: request.vertical_fullart,
            art: self.art.clone(),
            symbol: read(&request.symbol_svg),
            basic_watermark: read(&request.watermark_svg),
            set_size: request.set_size,
            text_shadows: request.text_shadows,
            ..Options::default()
        };
        let built = build_card(&self.template, &card, session.fonts(), &options)?;
        let mut warnings = built.warnings;
        if request.symbol_svg.is_some() && options.symbol.is_none() {
            warnings.push("The set symbol could not be read, so the card has none.".into());
        }

        // The engine's edit is always made from the bare template, so what takes it out
        // again is what undoes it there. Layers added by hand are not touched by either.
        let mut scratch = self.template.doc.clone();
        let inverse = apply(&mut scratch, &built.edit).map_err(|e| e.to_string())?;

        let build = self.next;
        self.next += 1;
        let mut edits = Vec::new();
        let showing = Self::showing(session.doc());
        // A fill made for art that has not moved is still right: typing in the rules box
        // must not make it again. It comes out with the old build and goes straight back in.
        let mut kept = None;
        if let Some(inverse) = showing.and_then(|n| self.inverses.get(&n)) {
            // The fill was added after the build it belongs to, so it comes out first.
            if let Some(layer) = tree::find(session.doc(), FILL_LAYER) {
                let same_place = built.fill.is_some() && showing.and_then(|n| self.plans.get(&n)) == built.fill.as_ref();
                if same_place {
                    kept = Some(layer.clone());
                }
                edits.push(Edit::RemoveLayer { id: FILL_LAYER.into() });
            }
            edits.push(inverse.clone());
        } else if showing.is_some() {
            return Err("This document holds a card built in an earlier session. Open its template again to change the card.".into());
        }
        edits.push(built.edit);
        // What was changed by hand on the card's layers since the last build (the art dragged
        // somewhere, a text restyled, a layer hidden) is put back on top of the new build,
        // wherever the new build did not itself change the same thing.
        if let Some(before) = showing.and_then(|n| self.built.get(&n)) {
            edits.extend(by_hand(before, session.doc(), &scratch));
        }
        let fill_kept = kept.is_some();
        if let (Some(layer), Some(plan)) = (kept, &built.fill) {
            edits.push(Edit::AddLayer { parent: plan.parent.clone(), layer, index: Some(plan.index) });
        }
        let mut noted = serde_json::to_value(&card).map_err(|e| e.to_string())?;
        noted["_build"] = json!(build);
        edits.push(Edit::SetDocProps { props: json!({ "card": noted }).as_object().cloned().unwrap_or_default() });

        let label = if showing.is_some() { format!("Update {}", card.name) } else { format!("Build {}", card.name) };
        session.execute(Edit::Batch { edits }, &label).map_err(|e| format!("The card could not be rebuilt, because a layer it sets was changed by hand ({e}). Undo that change, or open the template again."))?;
        self.inverses.insert(build, inverse);
        self.built.insert(build, scratch);
        // Enough to step back through a long run of edits; older builds are simply rebuilt plainly.
        while self.built.len() > 24 {
            self.built.pop_first();
        }
        let fill = built.fill.map(|plan| {
            let info = FillInfo { art_position: plan.art_position, art_scale: plan.art_scale, canvas: plan.canvas, keep: plan.keep };
            self.plans.insert(build, plan);
            info
        });
        Ok(Report { build, warnings, fill, fill_kept })
    }

    /// Add a finished fill (PNG bytes of the whole card area) for a build, on
    /// its own layer beneath the art. Returns false, changing nothing, when
    /// that build is no longer the one showing.
    pub fn add_fill(&mut self, session: &mut Session, build: u64, png: Vec<u8>, source: &str, params: Value) -> Result<bool, String> {
        // The card may have been rebuilt while the fill was being made. If its art has
        // not moved, the fill is as good for the card now showing.
        let Some(showing) = Self::showing(session.doc()) else { return Ok(false) };
        let plan = self.plans.get(&build).ok_or("That build needs no fill.")?;
        if showing != build && self.plans.get(&showing) != Some(plan) {
            return Ok(false);
        }
        let image = decode_image(&png).ok_or("The fill is not a readable picture.")?;
        let asset = format!("card-fill-{}", self.next);
        self.next += 1;
        let scale = plan.canvas.0 as f64 / image.w as f64;
        let mut edits = Vec::new();
        // A second fill for the same card (another tier, another seed) replaces the first.
        if tree::find(session.doc(), FILL_LAYER).is_some() {
            edits.push(Edit::RemoveLayer { id: FILL_LAYER.into() });
        }
        edits.push(plan.edit(&asset, &format!("assets/{asset}.png"), image.w as u32, image.h as u32, scale, source, params)?);
        session.stash_asset(&asset, png);
        session.execute(Edit::Batch { edits }, "Fill around the art").map_err(|e| e.to_string())?;
        Ok(true)
    }
}

/// Edits that carry hand-made changes over to a new build: for every layer
/// the last build and the new one both have, each property that differs now
/// from what the last build left, and that the new build leaves as the last
/// one did. Where the engine itself changes a property, the engine's value stands.
fn by_hand(last_build: &Doc, now: &Doc, new_build: &Doc) -> Vec<Edit> {
    let fields = |layer: &cardsmith_core::doc::Layer| {
        let mut own = serde_json::to_value(layer).ok().and_then(|v| v.as_object().cloned()).unwrap_or_default();
        own.remove("children");
        own
    };
    let mut edits = Vec::new();
    tree::walk(&last_build.root, &mut |layer| {
        let (Some(mine), Some(theirs)) = (tree::find(now, layer.id()), tree::find(new_build, layer.id())) else { return };
        let (base, mine, theirs) = (fields(layer), fields(mine), fields(theirs));
        let mut props = serde_json::Map::new();
        for key in base.keys().chain(mine.keys()) {
            if key == "id" || key == "kind" || props.contains_key(key) {
                continue;
            }
            if mine.get(key) != base.get(key) && theirs.get(key) == base.get(key) {
                props.insert(key.clone(), mine.get(key).cloned().unwrap_or(Value::Null));
            }
        }
        if !props.is_empty() {
            edits.push(Edit::SetLayerProps { id: layer.id().to_string(), props });
        }
    });
    edits
}
