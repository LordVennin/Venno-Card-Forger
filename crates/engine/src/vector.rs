// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported from Proxyshop src/templates/normal.py (BorderlessVectorTemplate),
// src/templates/_vector.py (VectorTemplate), src/helpers/colors.py
// (get_pinline_gradient) and src/schema/colors.py (pinlines_color_map).

//! The frame of Proxyshop's borderless vector template.
//!
//! Its frame is drawn with shapes that carry no color of their own. The card
//! picks which shapes show (the textbox comes in four heights) and each part
//! of the frame is then colored by a layer clipped to it: a flat color, or a
//! left-to-right blend for a two-color card.
//!
//! Layers are found by name, as Proxyshop's script finds them, starting from
//! the groups the importer bound.
//!
//! Built so far: cards of the normal layout with rules text. Not yet:
//! textless cards (built with the short textbox, with a warning), the color
//! indicator, nicknames, tokens, and the transform and modal faces.

use super::{plain, Builder, Options, Plan};
use cardsmith_cards::frame_logic::GOLD;
use cardsmith_cards::layout::CardData;
use cardsmith_cards::rules_text::strip_reminder_text;
use cardsmith_core::commands::Edit;
use cardsmith_core::doc::{build::rect_path, Color, Layer};
use cardsmith_core::text::layout_text;
use cardsmith_core::tree;
use serde_json::{json, Value};

fn rgb(r: u8, g: u8, b: u8) -> Color {
    [r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, 1.0]
}

fn hex(code: u32) -> Color {
    rgb((code >> 16) as u8, (code >> 8) as u8, code as u8)
}

/// Name boxes, P/T box and textbox: dark, so the white text reads.
fn dark(key: &str) -> Option<Color> {
    Some(hex(match key {
        "W" => 0x958676,
        "U" => 0x045482,
        "B" => 0x282523,
        "R" => 0x93362a,
        "G" => 0x134f23,
        "Gold" => 0x9a883f,
        "Land" | "Hybrid" => 0xa79c8e,
        "Artifact" => 0x48555c,
        "Colorless" => 0x74726b,
        "Vehicle" => 0x4c3314,
        _ => return None,
    }))
}

fn pinlines(key: &str) -> Option<Color> {
    Some(match key {
        "W" => rgb(246, 246, 239),
        "U" => rgb(0, 117, 190),
        "B" => rgb(39, 38, 36),
        "R" => rgb(239, 56, 39),
        "G" => rgb(0, 123, 67),
        "Gold" => rgb(246, 210, 98),
        "Land" => rgb(174, 151, 135),
        "Artifact" | "Colorless" => rgb(230, 236, 242),
        "Vehicle" => rgb(77, 45, 5),
        _ => return None,
    })
}

fn crown(key: &str) -> Option<Color> {
    Some(hex(match key {
        "W" => 0xf8f4f0,
        "U" => 0x006dae,
        "B" => 0x393431,
        "R" => 0xde3c23,
        "G" => 0x006d42,
        "Gold" => 0xefd16b,
        "Land" => 0xa59684,
        "Artifact" => 0xb5c5cd,
        "Colorless" => 0xd6d6dc,
        _ => return None,
    }))
}

/// What a part of the frame is colored with.
#[derive(Debug, PartialEq)]
enum Paint {
    Flat(Color),
    /// Stops across the card from left to right, as (position 0..1, color).
    Blend(Vec<(f64, Color)>),
}

/// The color or blend for a color string: a name ("Gold", "Land"), one color
/// letter, or two to four letters blended left to right. Unknown colors are black.
fn paint(colors: &str, map: fn(&str) -> Option<Color>) -> Paint {
    const BLACK: Color = [0.0, 0.0, 0.0, 1.0];
    let of = |key: &str| map(key).unwrap_or(BLACK);
    if colors.is_empty() {
        return Paint::Flat(of("Artifact"));
    }
    let letters: Vec<String> = colors.chars().map(String::from).collect();
    // Where one color has fully given way to the next, as shares of the width.
    let places: &[f64] = match letters.len() {
        2 => &[0.40, 0.60],
        3 => &[0.29, 0.40, 0.60, 0.71],
        4 if colors != "Land" && colors != GOLD => &[0.20, 0.30, 0.45, 0.55, 0.70, 0.80],
        _ => return Paint::Flat(of(colors)),
    };
    // Each color holds from the end of the blend before it to the start of the one after.
    Paint::Blend(places.iter().enumerate().map(|(i, at)| (*at, of(&letters[i.div_ceil(2)]))).collect())
}

/// Color a group: a layer just above it, clipped to it, covering the card.
fn color(build: &mut Builder, group: &str, what: &str, paint: Paint) {
    let Some(path) = tree::locate(&build.doc, group) else { return };
    let Some((at, parent)) = path.split_last() else { return };
    let parent = if parent.is_empty() { build.doc.root.base.id.clone() } else { tree::layer_at(&build.doc, parent).id().to_string() };
    let (w, h) = (build.doc.width as f64, build.doc.height as f64);
    let slug: String = what.to_lowercase().replace(' ', "-");
    let mut layer = json!({
        "kind": "shape", "id": format!("card-color-{slug}"), "name": format!("{what} color"), "visible": true, "opacity": 1, "blend": "normal",
        "clip": true, "role": "frame.color", "path": { "d": rect_path(0.0, 0.0, w, h) }, "fill": { "type": "solid", "color": [0, 0, 0, 1] }
    });
    match paint {
        Paint::Flat(color) => layer["fill"]["color"] = json!(color),
        Paint::Blend(stops) => {
            let stops: Vec<Value> = stops.iter().map(|(offset, color)| json!({ "offset": offset, "color": color })).collect();
            layer["effects"] = json!([{ "type": "gradientOverlay", "enabled": true, "blend": "normal", "opacity": 1, "gradient": { "stops": stops }, "style": "linear", "angle": 0, "scale": 1 }]);
        }
    }
    match serde_json::from_value::<Layer>(layer) {
        Ok(layer) => build.edit(Edit::AddLayer { parent, layer, index: Some(at + 1) }),
        Err(e) => build.warn(format!("The {what} color layer could not be made: {e}")),
    }
}

/// Give `to` the mask of `from`, set to hide the layer's effects too.
fn copy_mask(build: &mut Builder, from: &str, to: &str) {
    let Some(mut mask) = tree::find(&build.doc, from).and_then(|l| serde_json::to_value(l).ok()).map(|v| v["mask"].clone()).filter(Value::is_object) else { return };
    mask["enabled"] = json!(true);
    mask["hidesEffects"] = json!(true);
    build.set(to, json!({ "mask": mask }));
}

fn center_y(build: &mut Builder, id: Option<&String>) -> Option<f64> {
    build.area(id?).map(|a| (a[1] + a[3]) / 2.0)
}

/// Switch on and color the frame of the borderless vector template for a card.
pub(crate) fn frame(build: &mut Builder, card: &CardData, options: &Options) -> Plan {
    let b = &build.template.bindings;
    let f = &card.frame;
    let kind = if card.is_creature { "rulesCreature" } else { "rulesNoncreature" };

    // ---- how tall a textbox ----------------------------------------------------------

    let oracle = if options.remove_reminder { strip_reminder_text(&card.oracle_text) } else { card.oracle_text.clone() };
    let flavor = if options.remove_flavor { "" } else { card.flavor_text.as_str() };
    let textless = oracle.is_empty() && flavor.is_empty() && !card.is_basic_land;
    if textless {
        build.warn("Cards with no rules text are not built without a textbox yet on this template; the short textbox is used.".into());
    }
    let size = if card.is_basic_land || textless {
        "Short"
    } else {
        // By how many lines the text takes in the tallest textbox, counting one more for flavor text.
        let test = build.bound(&b.text, &format!("{kind}.tall")).and_then(|id| build.text(&id));
        let lines = test.map_or(0, |mut layer| {
            layer.content = plain(&if flavor.is_empty() { oracle.clone() } else { format!("{oracle}\n{flavor}") });
            layout_text(&layer, build.fonts).lines + usize::from(!flavor.is_empty())
        });
        match lines {
            0..=4 => "Short",
            5 => "Medium",
            6 => "Normal",
            _ => "Tall",
        }
    };
    let key = size.to_lowercase();

    // ---- which shapes ----------------------------------------------------------------

    let group = |name: &str| build.bound(&b.groups, name);
    let (pin, twins, textbox, pt, crown_outer, masks) = (group("pinlinesShapes"), group("twinsShapes"), group("textboxShapes"), group("ptBoxShapes"), group("crownShapes"), group("masks"));
    let type_shape = build.named(pin.as_deref(), &["Shape", "Typeline", size]);
    let shapes = [
        build.named(pin.as_deref(), &["Shape", "Card Name", "Normal"]),
        type_shape.clone(),
        build.named(pin.as_deref(), &["Shape", "Textbox", size]),
        build.named(twins.as_deref(), &["Shape", "Card Name", "Normal"]),
        build.named(twins.as_deref(), &["Shape", "Typeline", size]),
        build.named(textbox.as_deref(), &["Shape", size]),
    ];
    for shape in &shapes {
        match shape {
            Some(id) => build.show(id),
            None => build.warn(format!("The template is missing one of the shapes of its {size} textbox.")),
        }
    }

    // The pinlines are cut where this textbox height needs them cut, stroke and all.
    let cut = build.named(masks.as_deref(), &["Pinlines", size]);
    if let (Some(cut), Some(pin)) = (&cut, &pin) {
        copy_mask(build, cut, pin);
    }

    // ---- colors ----------------------------------------------------------------------

    // One or two colors are shown as themselves; more than that is gold.
    let few = (1..3).contains(&f.identity.chars().count());
    let lines = if f.is_hybrid || few {
        f.identity.as_str()
    } else if (2..=5).contains(&f.pinlines.chars().count()) && f.pinlines.chars().all(|c| "WUBRG".contains(c)) {
        GOLD
    } else {
        f.pinlines.as_str()
    };
    let boxes = if f.is_hybrid { f.identity.as_str() } else { f.twins.as_str() };
    let pt_color = match (f.is_hybrid, boxes) {
        (true, _) => &f.identity[f.identity.len().saturating_sub(1)..],
        (false, "Artifact") if card.is_vehicle => "Vehicle",
        (false, other) => other,
    };
    // An artifact's textbox stays artifact-colored whatever its color.
    let text_color = if card.is_artifact { "Artifact" } else if f.is_hybrid || few { f.identity.as_str() } else { f.twins.as_str() };

    if card.is_creature {
        if let Some(pt) = &pt {
            build.show(pt);
            color(build, pt, "PT Box", paint(pt_color, dark));
        }
    }
    if let Some(pin) = &pin {
        build.show(pin);
        color(build, pin, "Pinlines", paint(lines, pinlines));
    }
    if let Some(twins) = &twins {
        color(build, twins, "Name and Title Boxes", paint(boxes, dark));
    }
    if let Some(textbox) = &textbox {
        color(build, textbox, "Textbox", paint(text_color, dark));
    }
    if card.is_legendary {
        if let Some(outer) = &crown_outer {
            // The crown is cut by the same mask as the pinlines, and the pinlines make way for it.
            if let Some(cut) = &cut {
                copy_mask(build, cut, outer);
            }
            if let Some(mut vector) = pin.as_deref().and_then(|id| tree::find(&build.doc, id)).and_then(|l| serde_json::to_value(l).ok()).map(|v| v["vectorMask"].clone()).filter(Value::is_object) {
                vector["enabled"] = json!(true);
                build.set(pin.as_deref().unwrap_or_default(), json!({ "vectorMask": vector }));
            }
            match build.named(Some(outer), &["Legendary Crown"]) {
                Some(inner) => {
                    build.show(&inner);
                    color(build, &inner, "Legendary Crown", paint(lines, crown));
                }
                None => build.warn("The template's legendary crown could not be found.".into()),
            }
        }
    }
    if !card.color_indicator.is_empty() {
        build.warn("The color indicator is not built yet on this template.".into());
    }

    // ---- what the later steps need ---------------------------------------------------

    // One type line serves every textbox height: it moves with its bar.
    let tall_shape = build.named(pin.as_deref(), &["Shape", "Typeline", "Tall"]);
    let type_dy = match (center_y(build, type_shape.as_ref()), center_y(build, tall_shape.as_ref())) {
        (Some(this), Some(tall)) => this - tall,
        _ => 0.0,
    };
    let textbox_ref = build.bound(&b.text_refs, "textbox").and_then(|id| build.named(Some(&id), &[size])).and_then(|id| build.area(&id));
    Plan {
        pt_white: false,
        rules: build.bound(&b.text, &format!("{kind}.{key}")),
        textbox: textbox_ref,
        art_ref: build.bound(&b.art_refs, &key).or_else(|| build.bound(&b.art_refs, "normal")),
        type_dy,
        // White text on art needs its shadow; a basic land has no rules text to shadow.
        shadows: true,
        shadow_rules: !card.is_basic_land,
        textless: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_color_or_a_name_is_flat_and_several_colors_blend_left_to_right() {
        assert_eq!(paint("U", pinlines), Paint::Flat(rgb(0, 117, 190)));
        assert_eq!(paint("Gold", pinlines), Paint::Flat(rgb(246, 210, 98)));
        assert_eq!(paint("Land", dark), Paint::Flat(hex(0xa79c8e)));
        assert_eq!(paint("", crown), Paint::Flat(hex(0xb5c5cd)));
        assert_eq!(paint("WU", pinlines), Paint::Blend(vec![(0.40, rgb(246, 246, 239)), (0.60, rgb(0, 117, 190))]));
        let three = paint("WUB", pinlines);
        assert_eq!(three, Paint::Blend(vec![(0.29, rgb(246, 246, 239)), (0.40, rgb(0, 117, 190)), (0.60, rgb(0, 117, 190)), (0.71, rgb(39, 38, 36))]));
        // A color the map does not have is black rather than a failure.
        assert_eq!(paint("X", pinlines), Paint::Flat([0.0, 0.0, 0.0, 1.0]));
    }
}
