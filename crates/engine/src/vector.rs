// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported from Proxyshop src/templates/_vector.py (VectorTemplate),
// src/templates/_core.py (create_blended_layer, create_blended_solid_color,
// generate_layer), src/templates/normal.py (EtchedTemplate,
// ClassicRemasteredTemplate, UniversesBeyondTemplate, LOTRTemplate),
// src/helpers/colors.py (get_pinline_gradient), src/schema/colors.py (the
// color maps) and src/_state.py (masks, gradient_locations).

//! Proxyshop's vector templates: the frame is shapes, each part of it a
//! group whose look comes from texture layers inside it (one per color,
//! blended through masks for two colors) or from a flat color or a
//! left-to-right blend laid over it.
//!
//! The steps are VectorTemplate's; each template class changes which shapes,
//! masks and colors are used, as its Proxyshop class does.

use super::borderless::{color, copy_mask, rgb, Paint};
use super::{Builder, Plan};
use crate::classes::Class;
use cardsmith_cards::layout::CardData;
use cardsmith_core::commands::Edit;
use cardsmith_core::doc::Color;
use cardsmith_core::tree;
use serde_json::json;

/// A color map: a frame color by its name ("W", "Gold", "Land").
type Map = &'static [(&'static str, [u8; 3])];

const PINLINES: Map = &[("W", [246, 246, 239]), ("U", [0, 117, 190]), ("B", [39, 38, 36]), ("R", [239, 56, 39]), ("G", [0, 123, 67]), ("Gold", [246, 210, 98]), ("Land", [174, 151, 135]), ("Artifact", [230, 236, 242]), ("Colorless", [230, 236, 242]), ("Vehicle", [77, 45, 5])];
const INDICATOR: Map = &[("W", [248, 244, 240]), ("U", [0, 109, 174]), ("B", [57, 52, 49]), ("R", [222, 60, 35]), ("G", [0, 109, 66]), ("Artifact", [181, 197, 205]), ("Colorless", [214, 214, 220]), ("Land", [165, 150, 132])];
const CROWN: Map = &[("W", [248, 244, 240]), ("U", [0, 109, 174]), ("B", [57, 52, 49]), ("R", [222, 60, 35]), ("G", [0, 109, 66]), ("Gold", [239, 209, 107]), ("Land", [165, 150, 132]), ("Artifact", [181, 197, 205]), ("Colorless", [214, 214, 220])];
const ETCHED: Map = &[("W", [252, 254, 255]), ("U", [0, 117, 190]), ("B", [39, 38, 36]), ("R", [239, 56, 39]), ("G", [0, 123, 67]), ("Gold", [246, 210, 98]), ("Land", [136, 120, 98]), ("Artifact", [194, 210, 221]), ("Colorless", [194, 210, 221]), ("Vehicle", [77, 45, 5])];
const UNIVERSES: Map = &[("W", [246, 247, 241]), ("U", [0, 131, 193]), ("B", [44, 40, 33]), ("R", [237, 66, 31]), ("G", [5, 129, 64]), ("Gold", [239, 209, 107]), ("Land", [165, 150, 132]), ("Artifact", [227, 228, 230]), ("Colorless", [227, 228, 230]), ("Vehicle", [77, 45, 5])];
const LOTR_LIGHT: Map = &[("W", [230, 220, 185]), ("U", [72, 142, 191]), ("B", [126, 128, 127]), ("R", [217, 94, 76]), ("G", [97, 143, 102]), ("Gold", [246, 213, 125]), ("Land", [181, 162, 149]), ("Artifact", [210, 219, 227]), ("Colorless", [210, 219, 227]), ("Vehicle", [77, 45, 5])];
const LOTR_DARK: Map = &[("W", [134, 123, 105]), ("U", [44, 51, 103]), ("B", [44, 43, 39]), ("R", [118, 34, 34]), ("G", [13, 68, 45]), ("Gold", [158, 124, 78]), ("Land", [103, 90, 74]), ("Artifact", [61, 88, 109]), ("Colorless", [78, 91, 101]), ("Vehicle", [76, 51, 20])];

fn lookup(map: Map, key: &str) -> Color {
    map.iter().find(|(k, _)| *k == key).map_or([0.0, 0.0, 0.0, 1.0], |(_, c)| rgb(c[0], c[1], c[2]))
}

/// Proxyshop's `get_pinline_gradient`: one color flat, two to four blended left to right.
pub(crate) fn gradient(colors: &str, map: Map) -> Paint {
    if colors.is_empty() {
        return Paint::Flat(lookup(map, "Artifact"));
    }
    let letters: Vec<String> = colors.chars().map(String::from).collect();
    let places: &[f64] = match letters.len() {
        2 if multicolor(colors) => &[0.40, 0.60],
        3 if multicolor(colors) => &[0.26, 0.36, 0.64, 0.74],
        4 if multicolor(colors) => &[0.20, 0.30, 0.45, 0.55, 0.70, 0.80],
        5 if multicolor(colors) => &[0.20, 0.25, 0.35, 0.45, 0.55, 0.65, 0.75, 0.80],
        _ => return Paint::Flat(lookup(map, colors)),
    };
    Paint::Blend(places.iter().enumerate().map(|(i, at)| (*at, lookup(map, &letters[i.div_ceil(2)]))).collect())
}

/// A string of color letters ("WU"), not a name ("Gold").
fn multicolor(colors: &str) -> bool {
    !colors.is_empty() && colors.chars().all(|c| "WUBRG".contains(c))
}

/// What a part of the frame is made of.
enum Look {
    /// Texture layers in the group, by name: one, or several blended through masks.
    Textures(String),
    /// Several named texture layers, blended.
    TextureList(Vec<String>),
    /// A flat color or blend laid over the group.
    Paint(Paint),
}

struct Vector<'a, 'b> {
    build: &'a mut Builder<'b>,
    /// Colors at or above this count fall back to the pinlines color (gold).
    color_limit: usize,
}

impl Vector<'_, '_> {
    fn group(&self, name: &str) -> Option<String> {
        self.build.named(None, &[name])
    }

    fn under(&self, group: Option<&str>, path: &[&str]) -> Option<String> {
        group.and_then(|g| self.build.named(Some(g), path))
    }

    /// The masks that split a part of the frame between `n` colors.
    fn masks(&self, n: usize) -> Vec<String> {
        let names: &[&str] = match n {
            2 => &["1/2"],
            3 => &["1/3", "2/3"],
            4 => &["1/4", "1/2", "3/4"],
            5 => &["1/5", "2/5", "3/5", "4/5"],
            _ => &[],
        };
        let masks = self.group("Masks");
        names.iter().filter_map(|name| self.under(masks.as_deref(), &[name])).collect()
    }

    /// Proxyshop's `create_blended_layer`: show the texture for each color and blend them through the masks.
    fn blend_textures(&mut self, group: &str, colors: Vec<String>, masks: &[String], pinlines: &str) {
        let colors = if colors.len() >= self.color_limit { vec![pinlines.to_string()] } else { colors };
        let mut shown: Vec<String> = Vec::new();
        for (i, key) in colors.iter().enumerate() {
            let Some(layer) = self.build.named(Some(group), &[key]) else {
                let name = tree::find(&self.build.doc, group).map(|l| l.base().name.clone()).unwrap_or_default();
                self.build.missing(&name, key);
                continue;
            };
            self.build.show(&layer);
            if let (Some(previous), Some(mask)) = (shown.last(), i.checked_sub(1).and_then(|m| masks.get(m))) {
                // Under the one before, which is cut by the mask to let it show.
                if let Some(path) = tree::locate(&self.build.doc, previous) {
                    let index = *path.last().unwrap_or(&0);
                    let parent = self.build.parent_of(previous).unwrap_or_else(|| self.build.doc.root.base.id.clone());
                    let from = tree::locate(&self.build.doc, &layer).and_then(|p| p.last().copied()).unwrap_or(0);
                    // Removing a layer below the target shifts the target down by one.
                    let index = if from < index { index - 1 } else { index };
                    self.build.edit(Edit::MoveLayer { id: layer.clone(), parent, index });
                }
                copy_mask(self.build, mask, previous, false);
            }
            shown.push(layer);
        }
    }

    /// Proxyshop's `generate_layer`.
    fn generate(&mut self, group: Option<&str>, what: &str, look: Look, masks: &[String], pinlines: &str) {
        let Some(group) = group else { return };
        match look {
            Look::Textures(colors) => {
                let list = if multicolor(&colors) { colors.chars().map(String::from).collect() } else { vec![colors] };
                self.blend_textures(group, list, masks, pinlines);
            }
            Look::TextureList(list) => self.blend_textures(group, list, masks, pinlines),
            Look::Paint(paint) => {
                color(self.build, group, what, paint);
            }
        }
    }

    /// Proxyshop's `create_blended_solid_color` for the color indicator: the last color at the bottom, the others cut by masks above it.
    fn indicator(&mut self, card: &CardData) {
        let letters: Vec<String> = card.color_indicator.chars().rev().map(String::from).collect();
        let Some(dot) = self.group("Color Indicator") else { return };
        let Some(shape) = self.build.named(Some(&dot), &["Shape"]) else { return };
        self.build.show(&dot);
        let (outline, masks): (Option<&str>, &[&str]) = match letters.len() {
            2 => (Some("2"), &["1/2"]),
            3 => (Some("3"), &["1/3", "2/3"]),
            _ => (None, &[]),
        };
        if let Some(id) = outline.and_then(|name| self.build.named(Some(&dot), &[name])) {
            self.build.show(&id);
        }
        let mask_group = self.group("Masks");
        let masks: Vec<String> = masks.iter().filter_map(|name| self.under(mask_group.as_deref(), &["Color Indicator", name])).collect();
        // Each new color is laid just above the shape, under the ones before it, and the one before is cut by the next mask.
        let mut previous: Option<String> = None;
        for (i, letter) in letters.iter().enumerate() {
            let Some(layer) = color(self.build, &shape, &format!("Color Indicator {letter} {i}"), Paint::Flat(lookup(INDICATOR, letter))) else { continue };
            if let (Some(previous), Some(mask)) = (&previous, i.checked_sub(1).and_then(|m| masks.get(m))) {
                copy_mask(self.build, mask, previous, false);
            }
            previous = Some(layer);
        }
    }
}

/// Copy a layer's effects onto another.
fn copy_effects(build: &mut Builder, from: &str, to: &str) {
    let Some(effects) = tree::find(&build.doc, from).and_then(|l| serde_json::to_value(&l.base().effects).ok()) else { return };
    build.set(to, json!({ "effects": effects, "effectsEnabled": true }));
}

/// Switch on and color the frame of a vector template for a card.
pub(crate) fn frame(build: &mut Builder, card: &CardData, class: Class) -> Plan {
    let mut plan = Plan::new(build, card);
    let f = &card.frame;
    let identity = f.identity.clone();
    let pinlines = f.pinlines.clone();
    let twins = f.twins.clone();
    let is_vehicle = card.is_vehicle && class != Class::ClassicRemastered;
    let background = if !is_vehicle && f.background == "Vehicle" { "Artifact".to_string() } else { f.background.clone() };
    // Classic Remastered blends up to three colors (Proxyshop's "Max.Colors" setting, 3 unless changed).
    let color_limit = if class == Class::ClassicRemastered { 4 } else { 3 };
    let mut v = Vector { build, color_limit };
    let within = |n: usize| 1 < n && n < color_limit;
    let n = identity.chars().count();
    let blended = if within(n) { identity.clone() } else { pinlines.clone() };
    let half = v.masks(2);
    let default_masks = if half.is_empty() { Vec::new() } else { half.clone() };

    // ---- groups ------------------------------------------------------------------------

    let mut pinlines_group = v.group(if class == Class::ClassicRemastered { "Pinlines & Textbox" } else { "Pinlines" });
    let twins_group = if class == Class::ClassicRemastered { None } else { v.group("Name & Title Boxes") };
    let mut textbox_group = v.group("Textbox");
    let background_group = match class {
        Class::Etched => None,
        Class::UniversesBeyond if f.is_colorless => None,
        _ => v.group("Background"),
    };
    let crown_group = v.group("Legendary Crown");
    let pt_group = v.group("PT Box");
    let border_group = v.group("Border");

    // ---- colors ------------------------------------------------------------------------

    let pinlines_map = match class {
        Class::Etched => ETCHED,
        Class::UniversesBeyond => UNIVERSES,
        Class::Lotr => LOTR_LIGHT,
        _ => PINLINES,
    };
    let has_pinlines = card.is_land || card.is_artifact; // Classic Remastered: only lands and artifacts have colored pinlines
    let pinlines_look: Option<Paint> = match class {
        Class::Etched => Some(gradient(if card.is_artifact && !card.is_land { "Artifact" } else { &pinlines }, ETCHED)),
        Class::ClassicRemastered if !has_pinlines => None,
        Class::ClassicRemastered if n >= color_limit => Some(gradient(if n == 2 { "Gold" } else { &pinlines }, PINLINES)),
        Class::ClassicRemastered => Some(gradient(&identity, PINLINES)),
        _ => Some(gradient(&blended, pinlines_map)),
    };
    let twins_look = match class {
        Class::Lotr => Look::Paint(gradient(&twins, LOTR_DARK)),
        _ => Look::Textures(twins.clone()),
    };
    // Classic Remastered's background and textbox are lists of texture names.
    let cr_background: Vec<String> = if within(n) && !has_pinlines {
        identity.chars().map(String::from).collect()
    } else if n == 2 && !has_pinlines {
        vec!["Gold".into()]
    } else {
        vec![background.clone()]
    };
    let textbox_look = match class {
        Class::Etched => Look::Textures(pinlines.clone()),
        Class::Lotr => Look::Textures(twins.clone()),
        Class::ClassicRemastered => Look::TextureList(if card.is_land {
            if within(n) {
                identity.chars().map(|c| format!("{c} Land")).collect()
            } else if n == 2 {
                vec!["Gold Land".into()]
            } else if pinlines == "Land" {
                vec!["Land".into()]
            } else {
                vec![format!("{pinlines} Land")]
            }
        } else if f.is_hybrid {
            if within(n) {
                identity.chars().map(String::from).collect()
            } else if n == 2 {
                vec!["Gold".into()]
            } else {
                vec![pinlines.clone()]
            }
        } else {
            vec![background.clone()]
        }),
        _ => Look::Textures(blended.clone()),
    };
    let background_look = match class {
        Class::Lotr => Look::Textures(pinlines.clone()),
        Class::ClassicRemastered => Look::TextureList(cr_background.clone()),
        _ => Look::Textures(background.clone()),
    };
    let pt_look = match class {
        Class::Lotr => Look::Paint(gradient(if is_vehicle { "Vehicle" } else { &twins }, LOTR_DARK)),
        // The last of the background's colors (Proxyshop's "Gold.PT" setting off).
        Class::ClassicRemastered => Look::Textures(cr_background.last().cloned().unwrap_or(twins.clone())),
        // Proxyshop compares a flag with a name here, so the vehicle box is never chosen.
        Class::UniversesBeyond => Look::Textures(twins.clone()),
        _ if is_vehicle && background == "Vehicle" => {
            plan.pt_white = true;
            Look::Textures("Vehicle".into())
        }
        _ => Look::Textures(twins.clone()),
    };
    let masks = match class {
        Class::ClassicRemastered => {
            if within(n) {
                v.masks(n)
            } else {
                Vec::new()
            }
        }
        _ => default_masks,
    };

    // ---- Classic Remastered's nested groups --------------------------------------------

    let mut pinlines_groups: Vec<String> = pinlines_group.iter().cloned().collect();
    if class == Class::ClassicRemastered {
        let outer = pinlines_group.clone();
        textbox_group = v.under(outer.as_deref(), &["Textbox"]);
        if let Some(textbox) = &textbox_group {
            if let Some(effects) = v.build.named(Some(textbox), &[if pinlines_look.is_some() { "EFFECTS LAND" } else { "EFFECTS" }]) {
                copy_effects(v.build, &effects, textbox);
            }
        }
        pinlines_groups = if pinlines_look.is_some() {
            let mut groups: Vec<String> = ["Pinlines Top", "Pinlines Bottom"].iter().filter_map(|name| v.under(outer.as_deref(), &[name])).collect();
            if card.is_legendary {
                groups.extend(v.under(crown_group.as_deref(), &["Pinlines"]));
            }
            groups
        } else {
            Vec::new()
        };
        pinlines_group = pinlines_groups.first().cloned();
        if pinlines_look.is_some() {
            if let Some(id) = v.build.named(None, &["Text and Icons", "Textbox Reference Pinlines"]).and_then(|id| v.build.area(&id)) {
                plan.textbox = Some(id);
            }
        }
    }
    if class == Class::Lotr && card.is_legendary {
        pinlines_groups.extend(crown_group.clone());
    }

    // ---- shapes ------------------------------------------------------------------------

    let shapes: Vec<Option<String>> = match class {
        Class::Etched => vec![card.is_legendary.then(|| v.under(pinlines_group.as_deref(), &["Shape", "Legendary"])).flatten()],
        Class::Lotr => Vec::new(),
        Class::ClassicRemastered => vec![v.under(pinlines_group.as_deref(), &["Shape", "Normal"]), v.under(textbox_group.as_deref(), &["Shape", "Normal"])],
        _ => vec![
            v.under(border_group.as_deref(), &[if card.is_legendary { "Legendary" } else { "Normal" }]),
            v.under(pinlines_group.as_deref(), &["Shape", "Normal"]),
            v.under(twins_group.as_deref(), &["Shape", "Normal"]),
            v.under(textbox_group.as_deref(), &["Shape", "Normal"]),
        ],
    };
    for id in shapes.into_iter().flatten() {
        v.build.show(&id);
    }

    // ---- masks -------------------------------------------------------------------------

    match class {
        Class::Etched if card.is_legendary => {
            if let Some(id) = v.under(pinlines_group.as_deref(), &["Shape", "Normal"]) {
                v.build.enable_mask(&id);
            }
        }
        Class::Lotr if card.is_legendary => {
            if let Some(id) = &pinlines_group {
                v.build.enable_mask(id);
            }
        }
        Class::UniversesBeyond => {
            let mask = v.group("Masks");
            if let (Some(from), Some(to)) = (v.under(mask.as_deref(), &["Pinlines", "Normal"]), &pinlines_group) {
                copy_mask(v.build, &from, to, false);
            }
        }
        _ => {}
    }

    // ---- frame parts -------------------------------------------------------------------

    if card.is_creature {
        if let Some(pt) = &pt_group {
            v.build.show(pt);
            v.generate(Some(pt), "PT Box", pt_look, &masks, &pinlines);
        }
    }
    if !card.color_indicator.is_empty() {
        v.indicator(card);
    }
    if let Some(look) = pinlines_look {
        for group in pinlines_groups.clone() {
            v.build.show(&group);
            v.generate(Some(&group), &format!("Pinlines {}", tree::find(&v.build.doc, &group).map(|l| l.base().name.clone()).unwrap_or_default()), Look::Paint(clone_paint(&look)), &masks, &pinlines);
        }
    }
    v.generate(twins_group.as_deref(), "Name and Title Boxes", twins_look, &masks, &pinlines);
    v.generate(textbox_group.as_deref(), "Textbox", textbox_look, &masks, &pinlines);
    v.generate(background_group.as_deref(), "Background", background_look, &masks, &pinlines);

    // Colorless Universes Beyond cards show the art through their boxes.
    if class == Class::UniversesBeyond && f.is_colorless {
        for group in [&twins_group, &textbox_group].into_iter().flatten() {
            v.build.set(group, json!({ "fillOpacity": 0.6 }));
        }
    }

    // ---- legendary crown ---------------------------------------------------------------

    if card.is_legendary {
        match class {
            Class::Etched => {
                if let Some(id) = v.group("Legendary Shadows") {
                    v.build.show(&id);
                }
            }
            Class::Lotr => {
                if let Some(id) = &crown_group {
                    v.build.show(id);
                }
            }
            Class::UniversesBeyond => {
                if let Some(id) = &crown_group {
                    v.build.show(id);
                    v.generate(Some(id), "Legendary Crown", Look::Paint(gradient(&pinlines, CROWN)), &[], &pinlines);
                }
                if let Some(id) = v.under(pinlines_group.as_deref(), &["Shape", "Legendary"]) {
                    v.build.show(&id);
                }
            }
            _ => {
                if let Some(id) = &crown_group {
                    v.build.show(id);
                    let look = match class {
                        Class::ClassicRemastered => Look::TextureList(cr_background.clone()),
                        _ => Look::Textures(blended.clone()),
                    };
                    v.generate(Some(id), "Legendary Crown", look, &masks, &pinlines);
                }
            }
        }
    }

    // ---- what the text steps need ------------------------------------------------------

    if class == Class::Etched {
        // The divider is a group here.
        plan.divider = v.build.named(None, &["Text and Icons", "Divider"]);
    }
    plan
}

fn clone_paint(paint: &Paint) -> Paint {
    match paint {
        Paint::Flat(c) => Paint::Flat(*c),
        Paint::Blend(stops) => Paint::Blend(stops.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradients_follow_proxyshops_locations_and_names_stay_flat() {
        assert_eq!(gradient("U", PINLINES), Paint::Flat(rgb(0, 117, 190)));
        assert_eq!(gradient("Gold", PINLINES), Paint::Flat(rgb(246, 210, 98)));
        assert_eq!(gradient("WU", PINLINES), Paint::Blend(vec![(0.40, rgb(246, 246, 239)), (0.60, rgb(0, 117, 190))]));
        assert_eq!(gradient("WUB", ETCHED), Paint::Blend(vec![(0.26, rgb(252, 254, 255)), (0.36, rgb(0, 117, 190)), (0.64, rgb(0, 117, 190)), (0.74, rgb(39, 38, 36))]));
        assert_eq!(gradient("", PINLINES), Paint::Flat(rgb(230, 236, 242)));
    }
}
