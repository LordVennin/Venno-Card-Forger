// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported from Proxyshop src/templates/transform.py (TransformMod,
// VectorTransformMod, TransformTemplate), src/templates/mdfc.py (MDFCMod,
// VectorMDFCMod) and src/templates/_core.py (dfc_group, is_name_shifted,
// is_flipside_creature, the "Divider TF" layer).

//! The parts of a two-faced card's frame and text that a one-faced card has
//! not: the transform icon beside the name, the other face's power and
//! toughness on the front, and a modal card's hint at its other face.

use super::{plain, Builder, Plan};
use crate::classes::Class;
use cardsmith_cards::layout::CardData;
use cardsmith_cards::rules_text::{format_rules_text, RulesFormat};
use cardsmith_core::doc::RichText;
use serde_json::json;

/// Whether the other face is a creature, so the front shows its power and toughness.
pub(crate) fn flipside_creature(card: &CardData) -> bool {
    card.other_face.as_ref().is_some_and(|o| !o.power.is_empty() && !o.toughness.is_empty())
}

/// Whether the class draws the transform or modal parts of the frame.
fn draws_faces(class: Class) -> bool {
    !matches!(class, Class::Ixalan)
}

/// The group with this face's transform icons or modal hint.
fn face_group(build: &Builder, card: &CardData) -> Option<String> {
    let text = build.text_group();
    let face = if card.is_front { "Front" } else { "Back" };
    if card.is_transform {
        build.named(text.as_deref(), &["Transform", face])
    } else if card.is_mdfc {
        build.named(text.as_deref(), &[&format!("MDFC {face}")])
    } else {
        None
    }
}

/// Switch on the two-faced card's frame layers, and settle what the text steps do differently.
pub(crate) fn frame(build: &mut Builder, card: &CardData, class: Class, plan: &mut Plan) {
    if !(card.is_transform || card.is_mdfc) || !draws_faces(class) {
        return;
    }
    let text = build.text_group();
    let group = face_group(build, card);
    let vector = class.is_vector();

    if card.is_transform {
        // Vector templates keep the whole transform group hidden until it is wanted.
        if vector {
            if let Some(parent) = group.as_deref().and_then(|g| build.parent_of(g)) {
                build.show(&parent);
            }
        }
        if let Some(icon) = group.as_deref().and_then(|g| build.named(Some(g), &[&card.transform_icon])) {
            build.show(&icon);
        }
        if card.is_front && flipside_creature(card) {
            // A front whose back is a creature: shorter rules text, the back's P/T, and its own divider.
            let name = if card.is_creature { "Rules Text - Creature Flip" } else { "Rules Text - Noncreature Flip" };
            if let Some(rules) = build.named(text.as_deref(), &[name]) {
                plan.rules = Some(rules);
            }
            if let Some(divider) = build.named(text.as_deref(), &["Divider TF"]) {
                plan.divider = Some(divider);
            }
            let other = card.other_face.as_ref().expect("a flipside creature has another face");
            if let Some(id) = build.named(text.as_deref(), &["Flipside Power / Toughness"]) {
                build.write(&id, &plain(&format!("{}/{}", other.power, other.toughness)), "card.flipsidePt", json!({}));
            }
        }
        if !card.is_front {
            if card.transform_icon == "mooneldrazidfc" {
                // The Eldrazi moon's back is pale: dark text on it, on the raster templates.
                if !vector {
                    plan.black_text = true;
                }
            } else if vector {
                // Vector backs are dark: white text with its effects.
                plan.white_name = true;
                plan.white_type = true;
                plan.pt_white = true;
                plan.pt_effects = true;
            }
        }
    }

    if card.is_mdfc {
        if let Some(group) = &group {
            if vector {
                build.show(group);
            }
            // The icon in this face's colors, the bar in the other face's.
            let icon = if card.frame.twins == "Land" { "Colorless".to_string() } else { card.frame.twins.clone() };
            let bar = card.other_face.as_ref().map(|o| if o.twins == "Land" { "Colorless".to_string() } else { o.twins.clone() }).unwrap_or_default();
            for (part, key) in [("Top", icon), ("Bottom", bar)] {
                match build.named(Some(group), &[part, &key]) {
                    Some(id) => build.show(&id),
                    None => build.missing(part, &key),
                }
            }
        }
    }
}

/// The modal card's hint at its other face: its last type on the left, and its cost or mana ability on the right.
pub(crate) fn hint(build: &mut Builder, card: &CardData, class: Class, format: &RulesFormat, gap: f64, step: f64) {
    if !card.is_mdfc || !draws_faces(class) {
        return;
    }
    let (Some(group), Some(other)) = (face_group(build, card), card.other_face.as_ref()) else { return };
    let right = build.named(Some(&group), &["Right"]);
    let left = build.named(Some(&group), &["Left"]);
    if let Some(id) = &right {
        let formatted = format_rules_text(&other.hint_right, "", format);
        // Proxyshop's templates set this in Times New Roman, which few have: without it, the rules text's font.
        let mut more = json!({});
        if let Some(layer) = build.text(id) {
            if !build.fonts.names().contains(&layer.style.font.as_str()) {
                let rules = build.bound(&build.template.bindings.text, "rulesNoncreature").and_then(|r| build.text(&r));
                if let Some(rules) = rules.filter(|r| build.fonts.names().contains(&r.style.font.as_str())) {
                    let mut style = layer.style.clone();
                    style.font = rules.style.font;
                    more = json!({ "style": style });
                }
            }
        }
        build.write(id, &RichText { runs: formatted.content.runs, paragraphs: Vec::new() }, "card.mdfcRight", more);
    }
    if let Some(id) = &left {
        // As wide as it can be and still clear the right-hand text.
        let clear_of = right.as_deref().and_then(|r| build.layout(r)).and_then(|l| l.ink).map(|ink| ink[0]);
        let mut more = json!({});
        if let (Some(limit), Some(layer)) = (clear_of, build.text(id)) {
            let start = layer.origin.map_or(layer.bounds.x, |o| o.x);
            if limit - gap > start + layer.style.size {
                more = json!({ "box": { "x": start, "y": layer.bounds.y, "w": limit - gap - start, "h": layer.bounds.h }, "fit": "shrinkWidth", "fitStep": step });
            }
        }
        build.write(id, &plain(&other.hint_left), "card.mdfcLeft", more);
    }
}
