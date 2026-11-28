// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported from Proxyshop src/templates/_core.py (NormalTemplate: its frame
// layer lookups, enable_frame_layers, enable_crown, enable_hollow_crown),
// src/templates/_cosmetic.py (NyxMod, CompanionMod) and src/templates/normal.py
// (FullartTemplate, StargazingTemplate, InventionTemplate, ExpeditionTemplate,
// SnowTemplate, MiracleTemplate, ClassicTemplate).

//! The frame of a template whose colors are pre-drawn layers, one per color
//! in a group per part of the frame: the card shows the right one of each.
//!
//! Layers are looked up by the names Proxyshop's scripts use, in the groups
//! they use, so each template class can choose differently.

use super::{Builder, Plan};
use crate::classes::Class;
use cardsmith_cards::layout::CardData;

/// Proxyshop's choices of color for the parts of the frame, as a class makes them.
struct Colors {
    twins: String,
    pinlines: String,
    background: String,
    is_land: bool,
    is_legendary: bool,
    is_vehicle: bool,
    is_nyx: bool,
}

fn colors(card: &CardData, class: Class) -> Colors {
    let f = &card.frame;
    let mut c = Colors {
        twins: f.twins.clone(),
        pinlines: f.pinlines.clone(),
        // Only vehicles get the vehicle background.
        background: if !card.is_vehicle && f.background == "Vehicle" { "Artifact".into() } else { f.background.clone() },
        is_land: card.is_land,
        is_legendary: card.is_legendary,
        is_vehicle: card.is_vehicle,
        is_nyx: card.is_nyx,
    };
    match class {
        // The accent is a setting in Proxyshop, silver unless changed; lands are framed like anything else.
        Class::Invention => {
            c.twins = "Silver".into();
            c.background = "Silver".into();
            c.is_land = false;
        }
        Class::Expedition => c.is_land = false,
        Class::Miracle => {
            c.is_legendary = false;
            c.is_vehicle = false;
        }
        Class::Stargazing => c.is_nyx = true,
        _ => {}
    }
    c
}

/// Whether the class has Proxyshop's Nyx background and hollow crown (`NyxMod`).
fn nyx_mod(class: Class) -> bool {
    matches!(class, Class::M15 | Class::Fullart | Class::Stargazing | Class::Extended | Class::Miracle)
}

/// Whether the class has the companion crown (`CompanionMod`).
fn companion_mod(class: Class) -> bool {
    matches!(class, Class::M15 | Class::Fullart | Class::Stargazing | Class::Extended)
}

/// Switch on the frame layers of a raster template for a card.
pub(crate) fn frame(build: &mut Builder, card: &CardData, class: Class) -> Plan {
    let c = colors(card, class);
    let get = |build: &Builder, group: &str, name: &str| build.named(None, &[group, name]);
    let mut plan = Plan::new(build, card);

    if class == Class::Classic {
        return classic(build, card, &c, plan);
    }

    // Name and type boxes (Stargazing has none: its glass shows the art).
    let twins = if class == Class::Stargazing { None } else { get(build, "Name & Title Boxes", &c.twins) };
    match &twins {
        Some(id) => build.show(id),
        None if class != Class::Stargazing => build.missing("Name & Title Boxes", &c.twins),
        None => {}
    }

    // P/T box. A vehicle's is dark, so its numbers are white.
    if card.is_creature {
        let vehicle = (c.is_vehicle && c.background == "Vehicle").then(|| get(build, "PT Box", "Vehicle")).flatten();
        plan.pt_white = vehicle.is_some();
        let pt = vehicle.or_else(|| get(build, if class == Class::Stargazing { "PT Box Dark" } else { "PT Box" }, &c.twins));
        match pt {
            Some(id) => build.show(&id),
            None => build.missing("PT Box", &c.twins),
        }
    }

    // Pinlines and textbox: lands have their own set.
    // The transform template frames a colored land like any other card.
    let land_frame = c.is_land && !(class == Class::Transform && c.pinlines != "Land");
    let pinlines_group = if land_frame { "Land Pinlines & Textbox" } else { "Pinlines & Textbox" };
    let pinlines = get(build, pinlines_group, &c.pinlines);
    match &pinlines {
        Some(id) => build.show(id),
        None => build.missing(pinlines_group, &c.pinlines),
    }

    // Color indicator, for cards whose color is not in their cost.
    if !card.color_indicator.is_empty() {
        match get(build, "Color Indicator", &card.color_indicator) {
            Some(id) => build.show(&id),
            None => build.missing("Color Indicator", &card.color_indicator),
        }
    }

    // Background: the starry Nyx one for enchantment creatures where there is one. Expeditions have one fixed background.
    let background = if class == Class::Expedition {
        None
    } else {
        let nyx = (c.is_nyx && nyx_mod(class)).then(|| get(build, "Nyx", &c.background)).flatten();
        let vehicle = (c.is_vehicle && c.background == "Vehicle").then(|| get(build, "Background", "Vehicle").or_else(|| get(build, "Background", "Artifact"))).flatten();
        let found = nyx.or(vehicle).or_else(|| get(build, "Background", &c.background));
        if found.is_none() {
            build.missing("Background", &c.background);
        }
        found
    };
    if let Some(id) = &background {
        build.show(id);
    }

    // Legendary crown, and the border that makes room for it.
    let crown = c.is_legendary.then(|| get(build, "Legendary Crown", &c.pinlines)).flatten();
    if let Some(crown) = &crown {
        build.show(crown);
        swap_border(build);
        if class == Class::Expedition {
            // No hollow crown: the background and pinlines are cut away for the crown.
            if let Some(id) = build.named(None, &["Background"]) {
                build.enable_mask(&id);
            }
            if let Some(group) = pinlines.as_deref().and_then(|id| build.parent_of(id)) {
                build.enable_mask(&group);
            }
        } else {
            let hollow = (nyx_mod(class) && c.is_nyx) || (companion_mod(class) && card.is_companion);
            if hollow {
                // Masks cut the frame away behind the crown.
                if let Some(shadows) = build.named(None, &["Shadows"]) {
                    build.enable_mask(&shadows);
                }
                for inside in [Some(crown), pinlines.as_ref()].into_iter().flatten() {
                    if let Some(group) = build.parent_of(inside) {
                        build.enable_mask(&group);
                    }
                }
                if let Some(shadow) = build.named(None, &["Hollow Crown Shadow"]) {
                    build.show(&shadow);
                }
            }
        }
        if class == Class::Stargazing {
            if let Some(group) = build.parent_of(crown) {
                build.enable_vector_mask(&group);
            }
        }
    }
    if companion_mod(class) && c.is_legendary && card.is_companion {
        if let Some(id) = get(build, "Companion", &c.pinlines) {
            build.show(&id);
        }
    }

    // Full-art looks: glass over the art in place of the textbox and type bar, and white text on it.
    match class {
        Class::Fullart => {
            if let Some(group) = pinlines.as_deref().and_then(|id| build.parent_of(id)) {
                build.enable_vector_mask(&group);
            }
            if let Some(group) = twins.as_deref().and_then(|id| build.parent_of(id)) {
                build.enable_mask(&group);
            }
            if let Some(overlay) = build.named(None, &["Overlay"]) {
                build.show(&overlay);
            }
            plan.white_type = true;
            plan.white_rules = true;
        }
        Class::Stargazing => {
            for layer in [background.as_ref(), pinlines.as_ref()].into_iter().flatten() {
                if let Some(group) = build.parent_of(layer) {
                    build.enable_vector_mask(&group);
                }
            }
            if let Some(overlay) = build.named(None, &["Overlay"]) {
                build.show(&overlay);
                if let Some(name) = build.named(Some(&overlay), &["Name"]) {
                    build.show(&name);
                }
            }
            plan.white_type = true;
            plan.white_rules = true;
            plan.white_name = true;
            plan.pt_white = card.is_creature;
        }
        _ => {}
    }

    if matches!(class, Class::Expedition) {
        plan.rules = build.named(build.text_group().as_deref(), &["Rules Text"]);
        plan.pt_ref = false;
    }
    plan
}

/// Hide the normal border and show the one with room for a legendary crown.
fn swap_border(build: &mut Builder) {
    let Some(border) = build.named(None, &["Border"]) else { return };
    if let (Some(normal), Some(legendary)) = (build.named(Some(&border), &["Normal Border"]), build.named(Some(&border), &["Legendary Border"])) {
        build.hide(&normal);
        build.show(&legendary);
    }
}

/// The 1997 frame: one picture per color for the whole frame, no crown, no indicator.
fn classic(build: &mut Builder, card: &CardData, c: &Colors, mut plan: Plan) -> Plan {
    let f = &card.frame;
    // Two-color cards are gold here, unless hybrid or a land.
    let key = if c.pinlines.chars().count() == 2 && !f.is_hybrid && !c.is_land { c.background.clone() } else { c.pinlines.clone() };
    let group = if c.is_land { "Land" } else { "Nonland" };
    match build.named(None, &[group, &key]) {
        Some(id) => build.show(&id),
        None => build.missing(group, &key),
    }
    let text = build.text_group();
    plan.rules = build.named(text.as_deref(), &["Rules Text"]);
    plan.pt_ref = false;
    if c.is_land {
        if let Some(area) = build.named(text.as_deref(), &["Textbox Reference Land"]).and_then(|id| build.area(&id)) {
            plan.textbox = Some(area);
        }
        plan.expansion = build.named(text.as_deref(), &["Expansion Reference Land"]);
        plan.symbol_dy = 8.0 * build.doc.width as f64 / 3264.0;
    }
    plan.symbol_scale = 0.9;
    plan.classic_collector = true;
    plan
}
