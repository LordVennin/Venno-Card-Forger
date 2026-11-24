// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported from Proxyshop src/frame_logic.py.

//! Which frame a card gets: the color of its background, its pinlines and its
//! name and type boxes ("twins"), worked out from its mana cost, type line and
//! rules text.
//!
//! Two checks differ from Proxyshop, because Oracle text has been reworded
//! since it was written (cards say "this land" for their own name, and
//! "enters" for "enters the battlefield") and three of its own test cases no
//! longer pass against today's Scryfall data without them. Both are marked below.
//!
//! The values are the names Proxyshop's templates use for their layers: a
//! color letter ("W"), a pair in frame order ("GW"), or "Gold", "Land",
//! "Artifact", "Colorless", "Vehicle".

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const GOLD: &str = "Gold";
pub const LAND: &str = "Land";
pub const ARTIFACT: &str = "Artifact";
pub const COLORLESS: &str = "Colorless";
pub const VEHICLE: &str = "Vehicle";
pub const WUBRG: &str = "WUBRG";

/// The five colors, in the order mana costs are checked.
const COLORS: [&str; 5] = ["W", "U", "B", "R", "G"];

/// Color combinations in the order a frame shows them.
const ORDERED: [&str; 26] = [
    "WU", "UB", "BR", "RG", "GW", "WB", "BG", "GU", "UR", "RW", // two
    "GWU", "WUB", "UBR", "BRG", "RGW", "WBG", "URW", "BGU", "RWB", "GUR", // three
    "WUBR", "UBRG", "BRGW", "RGWU", "GWUB", // four
    "WUBRG",
];

const LAND_TYPES: [(&str, &str); 5] = [("Plains", "W"), ("Island", "U"), ("Swamp", "B"), ("Mountain", "R"), ("Forest", "G")];
const MONO_SYMBOLS: [&str; 5] = ["{W}", "{U}", "{B}", "{R}", "{G}"];
const HYBRID_SYMBOLS: [&str; 10] = ["W/U", "U/B", "B/R", "R/G", "G/W", "W/B", "B/G", "G/U", "U/R", "R/W"];

/// The frame makeup of one card face.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FrameDetails {
    pub background: String,
    pub pinlines: String,
    pub twins: String,
    /// The colors the frame stands for, in frame order ("" for none).
    pub identity: String,
    pub is_colorless: bool,
    pub is_hybrid: bool,
}

fn sorted(text: &str) -> String {
    let mut letters: Vec<char> = text.chars().collect();
    letters.sort_unstable();
    letters.into_iter().collect()
}

/// Color letters in the order a frame shows them: "WG" becomes "GW". Letters
/// that are not a known combination give "".
pub fn get_ordered_colors(text: &str) -> String {
    match text.chars().count() {
        0 => String::new(),
        1 => text.to_string(),
        2..=4 => ORDERED.iter().find(|known| known.len() == text.len() && sorted(known) == sorted(text)).map(|known| known.to_string()).unwrap_or_default(),
        _ => WUBRG.to_string(),
    }
}

/// The colors named anywhere in a mana cost, in WUBRG order.
pub fn get_mana_cost_colors(mana_cost: &str) -> String {
    COLORS.iter().filter(|color| mana_cost.contains(**color)).copied().collect()
}

fn text<'a>(card: &'a Value, key: &str) -> &'a str {
    card[key].as_str().unwrap_or("")
}

fn letters(value: &Value) -> String {
    value.as_array().map(|list| list.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
}

/// The color identity a nonland card's frame shows, which is not always
/// Scryfall's: Noble Hierarch is white, blue and green there, but its frame is green.
fn get_color_identity_nonland(card: &Value) -> String {
    let (mana_cost, type_line, oracle_text) = (text(card, "mana_cost"), text(card, "type_line"), text(card, "oracle_text"));
    // Transguild Courier.
    if oracle_text.contains(" is all colors.") {
        return WUBRG.to_string();
    }
    // No mana cost: the color indicator, then the listed colors.
    if mana_cost.is_empty() || mana_cost == "{0}" && !type_line.contains(ARTIFACT) {
        let indicator = letters(&card["color_indicator"]);
        if !indicator.is_empty() {
            return get_ordered_colors(&indicator);
        }
        // A face lists its own colors; a whole card its color identity.
        let listed = if card["color_identity"].is_array() { letters(&card["color_identity"]) } else { letters(&card["colors"]) };
        return get_ordered_colors(&listed);
    }
    get_ordered_colors(&get_mana_cost_colors(mana_cost))
}

/// A two-color card whose cost has only hybrid symbols (Maelstrom Muse is not; Bant Sureblade is not).
fn check_hybrid_color_card(identity: &str, mana_cost: &str, is_face: bool) -> bool {
    if identity.len() != 2 || MONO_SYMBOLS.iter().any(|symbol| mana_cost.contains(symbol)) {
        return false;
    }
    // No cost at all, on a card that is not one face of two: Asmoranomardicadaistinaculdacar.
    if mana_cost.is_empty() && !is_face {
        return true;
    }
    HYBRID_SYMBOLS.iter().any(|symbol| mana_cost.contains(symbol))
}

/// A simpler hybrid check for a lone mana cost, such as an Adventure's.
pub fn check_hybrid_mana_cost(identity: &str, mana_cost: &str) -> bool {
    identity.len() == 2 && !MONO_SYMBOLS.iter().any(|symbol| mana_cost.contains(symbol)) && HYBRID_SYMBOLS.iter().any(|symbol| mana_cost.contains(symbol))
}

/// The frame of a card or card face, from its Scryfall data.
pub fn get_frame_details(card: &Value) -> FrameDetails {
    if text(card, "type_line").contains("Land") {
        get_frame_details_land(card)
    } else {
        get_frame_details_nonland(card)
    }
}

/// Python's `str.find`, as a signed position, so "not found" sorts before every position.
fn find(line: &str, what: &str) -> i64 {
    line.find(what).map_or(-1, |at| at as i64)
}

fn get_frame_details_land(card: &Value) -> FrameDetails {
    let (type_line, oracle_text) = (text(card, "type_line"), text(card, "oracle_text"));
    let mut result = FrameDetails { background: LAND.into(), pinlines: LAND.into(), twins: LAND.into(), ..FrameDetails::default() };
    let set = |result: &mut FrameDetails, pinlines: &str, twins: Option<&str>, identity: &str| {
        result.pinlines = pinlines.to_string();
        if let Some(twins) = twins {
            result.twins = twins.to_string();
        }
        result.identity = identity.to_string();
    };
    let gold = |result: &mut FrameDetails| set(result, GOLD, Some(GOLD), WUBRG);

    // Basic land types in the type line.
    let mut twins = String::new();
    let typed: String = LAND_TYPES.iter().filter(|(name, _)| type_line.contains(name)).map(|(_, color)| *color).collect();
    if typed.len() == 1 {
        // One type: the boxes take its color; the pinlines are still to be decided (Murmuring Bosk).
        twins = typed;
    } else if typed.len() == 2 {
        let identity = get_ordered_colors(&typed);
        set(&mut result, &identity, None, &identity);
        return result;
    }

    let mut fetched = String::new();
    let mut colors_tapped = String::new();
    for line in oracle_text.split('\n') {
        let lower = line.to_lowercase();
        // A fetch land of some kind.
        if lower.contains("search your library") {
            if !lower.contains("cycling") {
                // The basic land types its search names.
                fetched.extend(LAND_TYPES.iter().filter(|(name, _)| line.contains(name)).map(|(_, color)| *color));
            }
            match fetched.len() {
                1 => {
                    set(&mut result, &fetched, Some(&fetched), &fetched);
                    return result;
                }
                2 => {
                    let identity = get_ordered_colors(&fetched);
                    set(&mut result, &identity, None, &identity);
                    return result;
                }
                // Three basics: the Panoramas stay colorless.
                3 => return result,
                // Cards now call themselves "this land", which is not a land being searched for.
                _ if line.replace("this land", "").contains("land") => {
                    // Fetches any basic, unless it is Ash Barrens ("into your hand"),
                    // Demolition Field ("Destroy") or the land comes in tapped for good.
                    if (!line.contains("tapped") || line.contains("untap")) && !line.contains("into your hand") && !line.contains("Destroy") {
                        gold(&mut result);
                    }
                    return result;
                }
                _ => {}
            }
        }

        // Adds mana of any color.
        if lower.contains("add") && line.contains("mana") && ["color ", "colors ", "color.", "colors.", "any type"].iter().any(|t| line.contains(t)) {
            // One-off mana is not what the land is for. "enters" covers both the old
            // wording ("enters the battlefield") and the current one ("When this land enters, ...").
            let exceptions = ["enters", "Remove a charge counter", "Sacrifice", "luck counter"];
            if exceptions.iter().all(|case| !line.contains(case)) {
                gold(&mut result);
                return result;
            }
        }

        // Thran Portal.
        if line.contains("choose a basic land type") {
            gold(&mut result);
            return result;
        }

        // Urborg, Tomb of Yawgmoth.
        if line.contains("Each land is a ") {
            if let Some((_, color)) = LAND_TYPES.iter().find(|(name, _)| line.contains(&format!("Each land is a {name}"))) {
                set(&mut result, color, Some(color), color);
                return result;
            }
        }

        // The colors this line taps for.
        if find(line, "{T}") < find(line, ":") && lower.contains("add ") {
            for color in COLORS {
                if line.contains(&format!("{{{color}}}")) && !colors_tapped.contains(color) {
                    colors_tapped.push_str(color);
                }
            }
        }
    }

    let identity = get_ordered_colors(&colors_tapped);
    let or = |fallback: &str| if twins.is_empty() { fallback.to_string() } else { twins.clone() };
    if identity.len() == 1 {
        result.twins = or(&colors_tapped);
        result.pinlines = identity.clone();
        result.identity = identity;
    } else if identity.len() == 2 {
        result.twins = or(LAND);
        result.pinlines = identity.clone();
        result.identity = identity;
    } else if colors_tapped.len() > 2 {
        result.twins = or(GOLD);
        result.pinlines = GOLD.into();
        result.identity = identity;
    }
    result
}

fn get_frame_details_nonland(card: &Value) -> FrameDetails {
    let (mana_cost, type_line, oracle_text) = (text(card, "mana_cost"), text(card, "type_line"), text(card, "oracle_text"));
    let identity = get_color_identity_nonland(card);
    let mut result = FrameDetails { background: identity.clone(), pinlines: identity.clone(), twins: identity.clone(), identity: identity.clone(), is_colorless: false, is_hybrid: false };

    // Full-art colorless cards and devoid cards.
    let devoid = oracle_text.contains("Devoid") && !identity.is_empty();
    if devoid || identity.is_empty() && !type_line.contains(ARTIFACT) || mana_cost.is_empty() && type_line.contains("Eldrazi") {
        if devoid && identity.len() > 1 {
            // Two-color devoid: gold boxes on the devoid background.
            result.twins = GOLD.into();
            result.background = GOLD.into();
        } else if !devoid {
            result.twins = COLORLESS.into();
            result.background = COLORLESS.into();
            result.pinlines = COLORLESS.into();
        }
        result.is_colorless = true;
        return result;
    }

    let hybrid = check_hybrid_color_card(&identity, mana_cost, text(card, "object") == "card_face");
    result.is_hybrid = hybrid;

    // Background.
    if type_line.contains(VEHICLE) {
        result.background = VEHICLE.into();
    } else if type_line.contains(ARTIFACT) {
        result.background = ARTIFACT.into();
    } else if identity.len() >= 2 && !hybrid {
        result.background = GOLD.into();
    }

    // Pinlines.
    if identity.is_empty() {
        result.pinlines = ARTIFACT.into();
    } else if identity.len() > 2 {
        result.pinlines = GOLD.into();
    }

    // Name and type boxes.
    if identity.is_empty() {
        result.twins = ARTIFACT.into();
    } else if hybrid {
        result.twins = LAND.into();
    } else if identity.len() >= 2 {
        result.twins = GOLD.into();
    }
    result
}

/// The rarity a set symbol is drawn in, for rarities that have no symbol color of their own.
pub fn get_special_rarity(rarity: &str, card: &Value) -> &'static str {
    if rarity == "special" {
        // Timeshifted cards.
        if text(card, "frame") == "1997" {
            return "timeshifted";
        }
        // Championship cards and masterpieces.
        if text(card, "set_name").contains("Champion") || text(card, "set_type") == "masterpiece" {
            return "mythic";
        }
        // The Prismatic Piper and the like.
        return "common";
    }
    // Bonus and anything else.
    "mythic"
}
