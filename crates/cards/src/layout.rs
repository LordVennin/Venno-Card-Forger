// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported from Proxyshop src/layouts.py (NormalLayout, the double-faced and
// planeswalker layouts' identity, and layout_map) and src/cards.py
// (process_card_data).

//! From Scryfall's data for a card to the card as it will be printed: which
//! layout it has, which face is meant, and every field a template needs.
//!
//! Two steps, as in Proxyshop. [`process_card_data`] settles the layout
//! (Scryfall has no "planeswalker" or "mutate" layout, and describes a meld
//! pair as three separate cards). [`card_data`] then reads the fields.
//!
//! Not ported yet, because nothing uses them until their layouts are built:
//! the extra fields of the split, adventure, leveler, saga, class, prototype,
//! battle and planeswalker layouts, alternate-language text, set symbols and
//! watermarks (which need set data), and the collector line's card count.

use crate::frame_logic::{get_frame_details, get_ordered_colors, get_special_rarity, FrameDetails};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The layouts Proxyshop has a class for. A template says which of these it can draw.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LayoutKind {
    Normal,
    Split,
    Transform,
    ModalDoubleFaced,
    Leveler,
    Class,
    Saga,
    Adventure,
    Mutate,
    Prototype,
    Battle,
    Planar,
    Token,
    Planeswalker,
    PlaneswalkerMdfc,
    PlaneswalkerTransform,
}

impl LayoutKind {
    /// The layout for a Scryfall `layout` value, after [`process_card_data`].
    /// Layouts Proxyshop does not draw specially fall back as it does; ones
    /// it does not know at all are treated as normal cards.
    pub fn from_scryfall(layout: &str) -> LayoutKind {
        match layout {
            "split" => LayoutKind::Split,
            "transform" | "meld" | "flip" | "double_faced_token" | "reversible_card" => LayoutKind::Transform,
            "modal_dfc" => LayoutKind::ModalDoubleFaced,
            "leveler" => LayoutKind::Leveler,
            "class" => LayoutKind::Class,
            "saga" => LayoutKind::Saga,
            "adventure" => LayoutKind::Adventure,
            "mutate" => LayoutKind::Mutate,
            "prototype" => LayoutKind::Prototype,
            "battle" => LayoutKind::Battle,
            "planar" => LayoutKind::Planar,
            "token" | "emblem" | "art_series" => LayoutKind::Token,
            "planeswalker" => LayoutKind::Planeswalker,
            "planeswalker_mdfc" => LayoutKind::PlaneswalkerMdfc,
            "planeswalker_tf" => LayoutKind::PlaneswalkerTransform,
            _ => LayoutKind::Normal,
        }
    }

    /// The name of Proxyshop's class for this layout.
    pub fn class_name(self) -> &'static str {
        match self {
            LayoutKind::Normal => "NormalLayout",
            LayoutKind::Split => "SplitLayout",
            LayoutKind::Transform => "TransformLayout",
            LayoutKind::ModalDoubleFaced => "ModalDoubleFacedLayout",
            LayoutKind::Leveler => "LevelerLayout",
            LayoutKind::Class => "ClassLayout",
            LayoutKind::Saga => "SagaLayout",
            LayoutKind::Adventure => "AdventureLayout",
            LayoutKind::Mutate => "MutateLayout",
            LayoutKind::Prototype => "PrototypeLayout",
            LayoutKind::Battle => "BattleLayout",
            LayoutKind::Planar => "PlanarLayout",
            LayoutKind::Token => "TokenLayout",
            LayoutKind::Planeswalker => "PlaneswalkerLayout",
            LayoutKind::PlaneswalkerMdfc => "PlaneswalkerMDFCLayout",
            LayoutKind::PlaneswalkerTransform => "PlaneswalkerTransformLayout",
        }
    }

    pub fn is_transform(self) -> bool {
        matches!(self, LayoutKind::Transform | LayoutKind::Battle | LayoutKind::PlaneswalkerTransform)
    }

    pub fn is_mdfc(self) -> bool {
        matches!(self, LayoutKind::ModalDoubleFaced | LayoutKind::PlaneswalkerMdfc)
    }
}

/// Transform icons Scryfall lists among a card's frame effects.
const TRANSFORM_ICONS: [&str; 8] = ["mooneldrazidfc", "compasslanddfc", "upsidedowndfc", "originpwdfc", "convertdfc", "sunmoondfc", "fandfc", "meld"];
const CARD_TYPES: [&str; 13] = ["Artifact", "Battle", "Conspiracy", "Creature", "Enchantment", "Instant", "Land", "Phenomenon", "Plane", "Planeswalker", "Scheme", "Sorcery", "Tribal"];
const SUPERTYPES: [&str; 7] = ["Basic", "Elite", "Host", "Legendary", "Ongoing", "Snow", "World"];

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

fn list(value: &Value) -> Vec<String> {
    value.as_array().map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// A name reduced to its letters and digits, lower case and without accents,
/// so "Lim-Dûl's Vault" and "lim duls vault" compare equal.
pub fn normalize_name(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à'..='å' => 'a',
            'ç' => 'c',
            'è'..='ë' => 'e',
            'ì'..='ï' => 'i',
            'ñ' => 'n',
            'ò'..='ö' | 'ø' => 'o',
            'ù'..='ü' => 'u',
            'ý' | 'ÿ' => 'y',
            other => other,
        })
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Settle a card's layout before its fields are read. `name` is the name the
/// card was asked for by, which says which face is meant.
///
/// A meld card comes from Scryfall as one of three separate cards. To treat
/// it as a two-faced card the other cards are needed: `meld_face` is given an
/// entry of the card's `all_parts` and returns that card's data (from the
/// network, the cache, or wherever the caller keeps it).
pub fn process_card_data(data: &mut Value, name: &str, meld_face: &mut dyn FnMut(&Value) -> Option<Value>) {
    let wanted = normalize_name(name);

    // A meld card becomes a transform card: the half that was asked for (or
    // the first half, when the melded side was) in front, the result behind.
    if text(data, "layout") == "meld" {
        let parts = data["all_parts"].as_array().cloned().unwrap_or_default();
        let halves: Vec<&Value> = parts.iter().filter(|p| text(p, "component") == "meld_part").collect();
        let result = parts.iter().find(|p| text(p, "component") == "meld_result");
        if let (Some(first), Some(result)) = (halves.first(), result) {
            let is = |part: &Value| normalize_name(text(part, "name")) == wanted;
            let front = if is(result) || is(first) { *first } else { halves.get(1).copied().unwrap_or(first) };
            let faces: Vec<Value> = [front, result]
                .into_iter()
                .filter_map(|part| {
                    let mut face = meld_face(part)?;
                    face["object"] = json!("card_face");
                    Some(face)
                })
                .collect();
            if faces.len() == 2 {
                data["card_faces"] = Value::Array(faces);
                let mut effects = list(&data["frame_effects"]);
                if !effects.iter().any(|e| TRANSFORM_ICONS.contains(&e.as_str())) {
                    effects.push("meld".into());
                }
                data["frame_effects"] = json!(effects);
                data["layout"] = json!("transform");
            }
        }
    }

    // A card with faces: note which one is meant, and refine the layout by its type.
    if let Some(faces) = data["card_faces"].as_array().filter(|f| f.len() >= 2).cloned() {
        let index = if normalize_name(text(&faces[0], "name")) == wanted { 0 } else { 1 };
        let type_line = text(&faces[index], "type_line").to_string();
        data["front"] = json!(index == 0);
        if type_line.contains("Planeswalker") {
            data["layout"] = json!(if text(data, "layout") == "transform" { "planeswalker_tf" } else { "planeswalker_mdfc" });
        }
        if type_line.contains("Saga") {
            data["layout"] = json!("saga");
        }
        if type_line.contains("Battle") {
            data["layout"] = json!("battle");
        }
        return;
    }

    if list(&data["keywords"]).iter().any(|k| k == "Mutate") {
        data["layout"] = json!("mutate");
    } else if text(data, "type_line").contains("Planeswalker") {
        data["layout"] = json!("planeswalker");
    }
}

/// What the user supplied about a card, usually through the art file's name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CardInput {
    /// The name the card was asked for by. For a two-faced card, the face wanted.
    pub name: String,
    /// The artist to credit, if not Scryfall's.
    pub artist: String,
    /// Who made the proxy.
    pub creator: String,
}

/// What a double-faced card shows of its other face.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct OtherFace {
    pub name: String,
    pub twins: String,
    pub mana_cost: String,
    pub type_line: String,
    pub oracle_text: String,
    pub power: String,
    pub toughness: String,
    /// The hint at the bottom of a modal double-faced card: the other face's
    /// last type on the left, and its mana cost or mana ability on the right.
    pub hint_left: String,
    pub hint_right: String,
}

/// A card, ready for the card engine: everything Proxyshop's `NormalLayout` offers a template.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardData {
    pub layout: LayoutKind,
    pub name: String,
    /// A printed alternate name (Godzilla series and the like), "" for none.
    pub nickname: String,
    pub mana_cost: String,
    pub type_line: String,
    pub supertypes: Vec<String>,
    pub types: Vec<String>,
    pub subtypes: Vec<String>,
    pub oracle_text: String,
    pub flavor_text: String,
    pub power: String,
    pub toughness: String,
    pub loyalty: String,
    /// Scryfall's color identity for the face, in its order.
    pub color_identity: Vec<String>,
    /// The color indicator dot, in frame order, "" for none.
    pub color_indicator: String,
    pub frame: FrameDetails,

    /// Set code, upper case.
    pub set: String,
    pub lang: String,
    /// "common", "uncommon", "rare", "mythic" or "timeshifted".
    pub rarity: String,
    pub artist: String,
    /// The collector number as printed ("12a", "★7"), and its digits as a number.
    pub collector_number_raw: String,
    pub collector_number: u32,
    pub creator: String,
    /// Release date of the printing, as Scryfall gives it (YYYY-MM-DD).
    pub released_at: String,
    pub watermark: String,
    pub frame_effects: Vec<String>,
    pub keywords: Vec<String>,

    pub is_creature: bool,
    pub is_land: bool,
    pub is_basic_land: bool,
    pub is_legendary: bool,
    pub is_artifact: bool,
    pub is_vehicle: bool,
    pub is_snow: bool,
    pub is_token: bool,
    pub is_emblem: bool,
    pub is_promo: bool,
    /// An enchantment creature, or printed on the starry Nyx frame.
    pub is_nyx: bool,
    pub is_companion: bool,
    pub is_miracle: bool,
    /// For a two-faced card: whether this is the front.
    pub is_front: bool,
    pub is_transform: bool,
    pub is_mdfc: bool,
    /// The icon beside a transforming card's name ("" when the card has one face).
    pub transform_icon: String,
    pub other_face: Option<OtherFace>,
}

impl CardData {
    /// The first letter of the rarity, upper case, as the collector line shows it.
    pub fn rarity_letter(&self) -> String {
        self.rarity.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default()
    }

    /// The collector line when the size of the set is not known: "R 0123".
    pub fn collector_data(&self) -> String {
        self.collector_line(None)
    }

    /// The collector line: "012/280 R" when the set's size is known and the
    /// card's number is within it, otherwise "R 0012".
    pub fn collector_line(&self, set_size: Option<u32>) -> String {
        if self.collector_number_raw.is_empty() {
            return String::new();
        }
        match set_size.filter(|size| *size >= self.collector_number) {
            Some(size) => format!("{:03}/{:03} {}", self.collector_number, size, self.rarity_letter()),
            None => format!("{} {:04}", self.rarity_letter(), self.collector_number),
        }
    }
}

/// Scryfall's artist credit, with a shared surname written once:
/// "Jack Smith & Jill Smith" becomes "Jack & Jill Smith".
fn artist_credit(artist: &str) -> String {
    if !artist.contains('&') {
        return artist.to_string();
    }
    let mut words: Vec<&str> = Vec::new();
    for word in artist.split(' ') {
        words.retain(|w| *w != word);
        words.push(word);
    }
    words.join(" ")
}

/// Read a card from Scryfall data that [`process_card_data`] has been through.
pub fn card_data(scryfall: &Value, input: &CardInput) -> CardData {
    let layout = LayoutKind::from_scryfall(text(scryfall, "layout"));
    let wanted = normalize_name(&input.name);
    let faces = scryfall["card_faces"].as_array().map(Vec::as_slice).unwrap_or_default();
    // The face that was asked for, or the card itself.
    let card = faces.iter().find(|face| normalize_name(text(face, "name")) == wanted).unwrap_or(scryfall);
    let frame = get_frame_details(card);
    let frame_effects = list(&scryfall["frame_effects"]);
    let (type_line, power, toughness) = (text(card, "type_line"), text(card, "power"), text(card, "toughness"));

    // "Legendary Creature — Human Wizard" into its three kinds of type.
    let words: Vec<&str> = type_line.split(' ').filter(|w| !w.is_empty() && *w != "—").collect();
    let pick = |keep: &dyn Fn(&str) -> bool| words.iter().filter(|w| keep(w)).map(|w| w.to_string()).collect::<Vec<_>>();
    let is_type = |w: &str| CARD_TYPES.contains(&w);
    let is_super = |w: &str| SUPERTYPES.contains(&w);

    let rarity = match text(scryfall, "rarity") {
        "" => "common",
        known @ ("common" | "uncommon" | "rare" | "mythic" | "timeshifted") => known,
        other => get_special_rarity(other, scryfall),
    };
    let collector_number_raw = text(scryfall, "collector_number").to_string();
    let digits: String = collector_number_raw.chars().filter(char::is_ascii_digit).collect();

    let is_creature = !power.is_empty() && !toughness.is_empty();
    let is_emblem = type_line.contains("Emblem");
    let is_front = scryfall["front"].as_bool().unwrap_or(true);
    let two_faced = layout.is_transform() || layout.is_mdfc();

    let other = faces.iter().find(|face| text(face, "name") != text(card, "name")).filter(|_| !faces.is_empty());
    let other_face = other.map(|face| {
        let (face_type, face_text) = (text(face, "type_line"), text(face, "oracle_text"));
        // A land shows what it taps for; anything else shows its cost.
        let hint_right = if !face_type.contains("Land") {
            text(face, "mana_cost").to_string()
        } else {
            face_text.split('\n').find(|line| line.starts_with("{T}")).map(|line| format!("{}.", line.split('.').next().unwrap_or(line))).unwrap_or_else(|| face_text.to_string())
        };
        OtherFace {
            name: text(face, "name").into(),
            twins: get_frame_details(face).twins,
            mana_cost: text(face, "mana_cost").into(),
            type_line: face_type.into(),
            oracle_text: face_text.into(),
            power: text(face, "power").into(),
            toughness: text(face, "toughness").into(),
            hint_left: face_type.split(' ').next_back().unwrap_or("").into(),
            hint_right,
        }
    });

    // The icon named in the frame effects; one card is printed upside down; otherwise the plain arrow.
    let transform_icon = if !two_faced {
        String::new()
    } else if let Some(icon) = frame_effects.iter().find(|e| TRANSFORM_ICONS.contains(&e.as_str())) {
        icon.clone()
    } else if text(scryfall, "oracle_id") == "e71ac446-02a4-4468-8d29-f28b21617665" {
        "upsidedowndfc".into()
    } else {
        "convertdfc".into()
    };

    CardData {
        layout,
        name: text(card, "name").into(),
        nickname: text(card, "flavor_name").into(),
        mana_cost: text(card, "mana_cost").into(),
        type_line: type_line.into(),
        supertypes: pick(&is_super),
        types: pick(&is_type),
        subtypes: pick(&|w| !is_type(w) && !is_super(w)),
        // Planeswalker costs use a real minus sign on Scryfall; the fonts want a hyphen.
        oracle_text: if type_line.contains("Planeswalker") { text(card, "oracle_text").replace('\u{2212}', "-") } else { text(card, "oracle_text").into() },
        flavor_text: text(card, "flavor_text").into(),
        power: power.into(),
        toughness: toughness.into(),
        loyalty: text(card, "loyalty").into(),
        color_identity: list(&card["color_identity"]),
        color_indicator: get_ordered_colors(&list(&card["color_indicator"]).concat()),
        frame,
        set: if text(scryfall, "set").is_empty() { "MTG".into() } else { text(scryfall, "set").to_uppercase() },
        lang: if text(scryfall, "lang").is_empty() { "EN".into() } else { text(scryfall, "lang").to_uppercase() },
        rarity: rarity.into(),
        artist: if input.artist.is_empty() { artist_credit(if text(card, "artist").is_empty() { "Unknown" } else { text(card, "artist") }) } else { input.artist.clone() },
        collector_number: digits.parse().unwrap_or(0),
        collector_number_raw,
        creator: input.creator.clone(),
        released_at: text(scryfall, "released_at").into(),
        watermark: text(card, "watermark").into(),
        keywords: list(&scryfall["keywords"]),
        is_creature,
        is_land: type_line.contains("Land"),
        is_basic_land: type_line.starts_with("Basic"),
        is_legendary: type_line.contains("Legendary"),
        is_artifact: type_line.contains("Artifact"),
        is_vehicle: type_line.contains("Vehicle"),
        is_snow: type_line.contains("Snow"),
        is_token: type_line.contains("Token") || is_emblem,
        is_emblem,
        is_promo: scryfall["promo"].as_bool().unwrap_or(false) || text(scryfall, "set_type") == "promo" || !list(&scryfall["promo_types"]).is_empty(),
        is_nyx: frame_effects.iter().any(|e| e == "nyxtouched") || is_creature && type_line.contains("Enchantment"),
        is_companion: frame_effects.iter().any(|e| e == "companion"),
        is_miracle: frame_effects.iter().any(|e| e == "Miracle" || e == "miracle"),
        is_front,
        is_transform: layout.is_transform(),
        is_mdfc: layout.is_mdfc(),
        transform_icon,
        other_face,
        frame_effects,
    }
}
