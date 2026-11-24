//! The card engine on a small made-up template, with the rectangle test font
//! (see crates/core/tests/text.rs for its measurements).

use cardsmith_cards::layout::{card_data, CardData, CardInput};
use cardsmith_core::commands::apply;
use cardsmith_core::doc::{Doc, Layer};
use cardsmith_core::text::{layout_text, Fonts};
use cardsmith_core::tree;
use cardsmith_engine::{build_card, Art, Options, Template};
use serde_json::{json, Value};

const FONT: &[u8] = include_bytes!("../../core/tests/fixtures/blocks.ttf");

fn fonts() -> Fonts {
    let mut fonts = Fonts::new();
    fonts.add(FONT.to_vec()).unwrap();
    fonts
}

fn base(kind: &str, id: &str, name: &str, visible: bool) -> Value {
    json!({ "kind": kind, "id": id, "name": name, "visible": visible, "opacity": 1, "blend": "normal", "clip": false })
}

fn with(mut layer: Value, more: Value) -> Value {
    layer.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
    layer
}

fn group(id: &str, name: &str, visible: bool, children: Vec<Value>) -> Value {
    with(base("group", id, name, visible), json!({ "children": children, "passThrough": true }))
}

fn rect(id: &str, name: &str, visible: bool, r: [f64; 4]) -> Value {
    let d = format!("M{} {}L{} {}L{} {}L{} {}Z", r[0], r[1], r[2], r[1], r[2], r[3], r[0], r[3]);
    with(base("shape", id, name, visible), json!({ "path": { "d": d }, "fill": { "type": "solid", "color": [0.5, 0.5, 0.5, 1] } }))
}

fn text(id: &str, name: &str, visible: bool, body: &str, more: Value) -> Value {
    let layer = with(base("text", id, name, visible), json!({
        "box": { "x": 0, "y": 0, "w": 0, "h": 0 }, "content": { "runs": [{ "text": body }] },
        "style": { "font": "Blocks-Regular", "size": 100, "color": [0, 0, 0, 1], "leading": 1, "align": "left" }, "fit": "none"
    }));
    with(layer, more)
}

/// A 2000 x 3000 template with one layer per thing the engine looks for.
fn template() -> Template {
    let colors = |prefix: &str, names: &[&str]| -> Vec<Value> { names.iter().map(|n| rect(&format!("{prefix}-{n}"), n, false, [0.0, 0.0, 10.0, 10.0])).collect() };
    let ids = |prefix: &str, names: &[&str]| -> Value { names.iter().map(|n| (n.to_string(), json!(format!("{prefix}-{n}")))).collect::<serde_json::Map<_, _>>().into() };
    let (twins, pins, bg) = (["W", "U", "Gold", "Land", "Artifact"], ["W", "U", "WU", "Gold", "Artifact", "Land"], ["W", "U", "Gold", "Artifact", "Vehicle", "Land", "Colorless"]);
    let mask = json!({ "asset": "m", "enabled": false, "outside": "reveal", "offset": { "x": 0, "y": 0 } });
    let doc = json!({
        "version": 1, "width": 2000, "height": 3000, "dpi": 720,
        "assets": { "m": { "kind": "image", "path": "assets/m.png", "width": 4, "height": 4 }, "d": { "kind": "image", "path": "assets/d.png", "width": 1000, "height": 20 } },
        "root": with(base("group", "root", "Root", true), json!({ "passThrough": true, "children": [
            with(base("pixel", "placeholder", "Layer 1", true), json!({ "asset": "m", "transform": [1, 0, 0, 0, 1, 0] })),
            rect("art-normal", "Art Frame", false, [200.0, 400.0, 1800.0, 1400.0]),
            rect("art-full", "Full Art Frame", false, [100.0, 100.0, 1900.0, 2900.0]),
            group("g-bg", "Background", true, colors("bg", &bg)),
            group("g-nyx", "Nyx", true, colors("nyx", &["U"])),
            with(base("pixel", "shadows", "Shadows", true), json!({ "asset": "m", "transform": [1, 0, 0, 0, 1, 0], "mask": mask })),
            rect("crown-shadow", "Hollow Crown Shadow", false, [0.0, 0.0, 10.0, 10.0]),
            with(group("g-pins", "Pinlines & Textbox", true, colors("pin", &pins)), json!({ "mask": mask })),
            group("g-landpins", "Land Pinlines & Textbox", true, colors("landpin", &pins)),
            with(group("g-crown", "Legendary Crown", true, colors("crown", &pins)), json!({ "mask": mask })),
            group("g-border", "Border", true, vec![rect("border-normal", "Normal Border", true, [0.0, 0.0, 10.0, 10.0]), rect("border-legend", "Legendary Border", false, [0.0, 0.0, 10.0, 10.0])]),
            group("g-twins", "Name & Title Boxes", true, colors("twins", &twins)),
            group("g-pt", "PT Box", true, [colors("pt", &twins), colors("pt", &["Vehicle"])].concat()),
            group("g-ci", "Color Indicator", true, colors("ci", &["U", "WU"])),
            group("g-text", "Text and Icons", true, vec![
                rect("ref-textbox", "Textbox Reference", false, [200.0, 1600.0, 1800.0, 2600.0]),
                rect("ref-pt", "PT Reference", false, [1500.0, 2400.0, 1800.0, 2600.0]),
                rect("ref-expansion", "Expansion Reference", false, [1600.0, 1450.0, 1750.0, 1550.0]),
                with(base("pixel", "divider", "Divider", false), json!({ "asset": "d", "transform": [1, 0, 500, 0, 1, 0] })),
                text("rules-c", "Rules Text - Creature", false, "Words", json!({ "box": { "x": 250, "y": 1650, "w": 1500, "h": 900 } })),
                text("rules-n", "Rules Text - Noncreature", false, "Words", json!({ "box": { "x": 250, "y": 1650, "w": 1500, "h": 900 } })),
                text("type-shift", "Typeline Shift", false, "Type", json!({ "origin": { "x": 400, "y": 1550 } })),
                text("type", "Typeline", true, "Type", json!({ "origin": { "x": 250, "y": 1550 } })),
                text("pt-text", "Power / Toughness", false, "0/0", json!({ "origin": { "x": 1650, "y": 2550 }, "style": { "font": "Blocks-Regular", "size": 100, "color": [0, 0, 0, 1], "align": "center" } })),
                text("mana", "Mana Cost", true, "Mana", json!({ "origin": { "x": 1800, "y": 300 }, "style": { "font": "Blocks-Regular", "size": 100, "color": [0, 0, 0, 1], "align": "right" } })),
                text("name", "Card Name", true, "Name", json!({ "origin": { "x": 250, "y": 300 } })),
            ]),
            group("g-legal", "Legal", true, vec![
                group("collector", "Collector", false, vec![
                    with(text("col-bottom", "Bottom", true, "SET", json!({ "origin": { "x": 200, "y": 2900 } })), json!({ "content": { "runs": [{ "text": "SET" }, { "text": " * " }, { "text": "EN" }, { "text": " Artist", "size": 50 }] } })),
                    text("col-top", "Top", true, "001/999 M", json!({ "origin": { "x": 200, "y": 2800 } })),
                ]),
                text("set", "Set", true, " * EN", json!({ "origin": { "x": 200, "y": 2900 } })),
                text("artist", "Artist", true, "Artist", json!({ "origin": { "x": 200, "y": 2800 } })),
            ]),
        ] }))
    });
    let manifest = json!({ "id": "mini", "bindings": {
        "kind": "raster", "layouts": ["normal"],
        "artRefs": { "normal": "art-normal", "fullArt": "art-full" },
        "groups": { "text": "g-text", "legal": "g-legal", "border": "g-border" },
        "text": { "name": "name", "mana": "mana", "type": "type", "typeShift": "type-shift", "pt": "pt-text", "artist": "artist", "set": "set", "collector": "collector", "rulesCreature": "rules-c", "rulesNoncreature": "rules-n" },
        "textRefs": { "textbox": "ref-textbox", "pt": "ref-pt", "expansion": "ref-expansion" },
        "toggles": {
            "background": ids("bg", &bg), "nyx": ids("nyx", &["U"]), "pinlines": ids("pin", &pins), "landPinlines": ids("landpin", &pins), "twins": ids("twins", &twins),
            "ptBox": ids("pt", &["W", "U", "Gold", "Land", "Artifact", "Vehicle"]), "crown": ids("crown", &pins), "colorIndicator": ids("ci", &["U", "WU"]),
            "border": { "Normal Border": "border-normal", "Legendary Border": "border-legend" }
        },
        "layers": { "divider": "divider", "crownShadow": "crown-shadow", "shadows": "shadows" },
        "art": "placeholder"
    } });
    Template::from_json(&doc.to_string(), &manifest.to_string()).unwrap()
}

fn card(more: Value) -> CardData {
    let mut data = json!({ "object": "card", "layout": "normal", "name": "ab", "mana_cost": "{W}", "type_line": "Creature — Cat", "oracle_text": "a", "power": "2", "toughness": "3", "set": "tst", "collector_number": "7", "rarity": "rare", "artist": "bb", "lang": "en" });
    data.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
    card_data(&data, &CardInput { name: data["name"].as_str().unwrap().into(), ..CardInput::default() })
}

fn build(card: &CardData, options: &Options) -> (Doc, Vec<String>) {
    let template = template();
    let built = build_card(&template, card, &fonts(), options).unwrap();
    let mut doc = template.doc.clone();
    apply(&mut doc, &built.edit).unwrap();
    (doc, built.warnings)
}

fn layer<'a>(doc: &'a Doc, id: &str) -> &'a Layer {
    tree::find(doc, id).unwrap_or_else(|| panic!("no layer {id}"))
}

fn visible(doc: &Doc) -> Vec<String> {
    let template = template();
    let mut changed = Vec::new();
    tree::walk(&doc.root, &mut |l| {
        let before = tree::find(&template.doc, l.id()).map(|t| t.base().visible);
        if before == Some(false) && l.base().visible {
            changed.push(l.id().to_string());
        }
    });
    changed.sort();
    changed
}

fn runs(doc: &Doc, id: &str) -> String {
    match layer(doc, id) {
        Layer::Text(t) => t.content.runs.iter().map(|r| r.text.as_str()).collect(),
        _ => panic!("{id} is not text"),
    }
}

fn ink(doc: &Doc, id: &str) -> [f64; 4] {
    match layer(doc, id) {
        Layer::Text(t) => layout_text(t, &fonts()).ink.unwrap(),
        _ => panic!("{id} is not text"),
    }
}

#[test]
fn a_plain_creature_gets_its_frame_layers_text_and_collector_line() {
    let (doc, warnings) = build(&card(json!({})), &Options::default());
    assert!(warnings.iter().all(|w| w.contains("Proxyglyph")), "{warnings:?}");
    assert_eq!(visible(&doc), ["bg-W", "collector", "pin-W", "pt-W", "pt-text", "rules-c", "twins-W"]);
    assert_eq!((runs(&doc, "name"), runs(&doc, "type"), runs(&doc, "pt-text")), ("ab".into(), "Creature — Cat".into(), "2/3".into()));
    // The mana cost is the symbol font's characters, colored.
    let Layer::Text(mana) = layer(&doc, "mana") else { panic!() };
    assert_eq!(mana.content.runs.iter().map(|r| (r.text.as_str(), r.font.as_deref())).collect::<Vec<_>>(), [("o", Some("Proxyglyph")), ("w", Some("Proxyglyph"))]);
    // Number and rarity on top; set, language and artist below; the plain set and artist lines are hidden.
    assert_eq!((runs(&doc, "col-top"), runs(&doc, "col-bottom")), ("R 0007".into(), "TST * EN bb".into()));
    assert!(!layer(&doc, "artist").base().visible && !layer(&doc, "set").base().visible);
    // Layers say what part of the card they are, and the document knows its card.
    assert_eq!(layer(&doc, "name").base().role.as_deref(), Some("card.name"));
    assert_eq!(layer(&doc, "rules-c").base().role.as_deref(), Some("card.rules"));
    assert_eq!((doc.card.as_ref().unwrap()["name"].as_str(), doc.template_id.as_deref()), (Some("ab"), Some("mini")));
}

#[test]
fn building_is_one_step_that_undoes_back_to_the_template() {
    let template = template();
    let built = build_card(&template, &card(json!({ "type_line": "Legendary Enchantment Creature — God", "mana_cost": "{U}" })), &fonts(), &Options { art: Some(Art { asset: "pic".into(), path: "assets/pic.png".into(), width: 400, height: 300 }), ..Options::default() }).unwrap();
    let mut doc = template.doc.clone();
    let undo = apply(&mut doc, &built.edit).unwrap();
    assert_ne!(doc, template.doc);
    apply(&mut doc, &undo).unwrap();
    assert_eq!(doc, template.doc);
}

#[test]
fn frame_layers_follow_the_cards_colors_and_kind() {
    // Gold card: gold background and boxes, two-color pinlines.
    let (doc, _) = build(&card(json!({ "mana_cost": "{W}{U}", "type_line": "Instant", "power": null, "toughness": null })), &Options::default());
    assert_eq!(visible(&doc), ["bg-Gold", "collector", "pin-WU", "rules-n", "twins-Gold"]);
    // A land uses the land pinlines; a legendary enchantment creature gets the Nyx background, the crown, its border and the hollow-crown masks.
    let (doc, _) = build(&card(json!({ "mana_cost": "", "type_line": "Land", "oracle_text": "{T}: Add {U}.", "power": null, "toughness": null })), &Options::default());
    assert_eq!(visible(&doc), ["bg-Land", "collector", "landpin-U", "rules-n", "twins-U"]);
    let (doc, _) = build(&card(json!({ "mana_cost": "{U}", "type_line": "Legendary Enchantment Creature — God" })), &Options::default());
    assert_eq!(visible(&doc), ["border-legend", "collector", "crown-U", "crown-shadow", "nyx-U", "pin-U", "pt-U", "pt-text", "rules-c", "twins-U"]);
    assert!(!layer(&doc, "border-normal").base().visible);
    for masked in ["shadows", "g-crown", "g-pins"] {
        assert_eq!(layer(&doc, masked).base().mask.as_ref().unwrap().enabled, Some(true), "{masked}");
    }
    // A legendary card that is not Nyx keeps its masks off.
    let (doc, _) = build(&card(json!({ "type_line": "Legendary Creature — Cat" })), &Options::default());
    assert!(layer(&doc, "g-crown").base().mask.as_ref().unwrap().enabled == Some(false) && !layer(&doc, "crown-shadow").base().visible);
    // A vehicle: its own background and P/T box, with white numbers.
    let (doc, _) = build(&card(json!({ "mana_cost": "{2}", "type_line": "Artifact — Vehicle" })), &Options::default());
    assert!(layer(&doc, "bg-Vehicle").base().visible && layer(&doc, "pt-Vehicle").base().visible);
    let Layer::Text(pt) = layer(&doc, "pt-text") else { panic!() };
    assert_eq!(pt.style.color, [1.0, 1.0, 1.0, 1.0]);
    // A color indicator moves the type line over and shows the dot.
    let (doc, _) = build(&card(json!({ "mana_cost": "", "color_indicator": ["U"], "colors": ["U"] })), &Options::default());
    assert!(layer(&doc, "ci-U").base().visible && layer(&doc, "type-shift").base().visible && !layer(&doc, "type").base().visible);
    assert_eq!(runs(&doc, "type-shift"), "Creature — Cat");
    // A color the template has no layer for is reported, not fatal.
    let (_, warnings) = build(&card(json!({ "mana_cost": "{B}" })), &Options::default());
    assert!(warnings.iter().any(|w| w.contains("no \"B\" layer among its twins layers")), "{warnings:?}");
}

#[test]
fn a_long_name_shrinks_to_stay_clear_of_the_mana_cost_and_a_long_type_of_the_set_symbol() {
    // The test template is 2000 px wide, so Proxyshop's 30 px gap is 18. With no symbol font the cost has no ink, and its anchor (x 1800) is the edge.
    let (doc, _) = build(&card(json!({ "name": "bbbbbbbbbbbbbbbbbbbb" })), &Options::default());
    let Layer::Text(name) = layer(&doc, "name") else { panic!() };
    let layout = layout_text(name, &fonts());
    assert!(layout.scale < 1.0 && layout.ink.unwrap()[2] <= 1782.0, "{} {:?}", layout.scale, layout.ink);
    // A short one is left alone.
    let (doc, _) = build(&card(json!({})), &Options::default());
    let Layer::Text(name) = layer(&doc, "name") else { panic!() };
    assert_eq!(layout_text(name, &fonts()).scale, 1.0);
    // The type line stops short of the expansion reference at x 1600.
    let (doc, _) = build(&card(json!({ "type_line": "bbbbbbbbbbbbbbbbbbbb" })), &Options::default());
    assert!(ink(&doc, "type")[2] <= 1582.0);
    // A basic land has nothing to avoid.
    let (doc, _) = build(&card(json!({ "name": "bbbbbbbbbbbbbbbbbbbb", "type_line": "Basic Land — Island", "mana_cost": "", "power": null, "toughness": null })), &Options::default());
    let Layer::Text(name) = layer(&doc, "name") else { panic!() };
    assert_eq!(layout_text(name, &fonts()).scale, 1.0);
}

#[test]
fn rules_text_is_centered_in_the_textbox_shrunk_to_fit_and_lifted_off_the_pt_box() {
    // One short line: centered both ways in the textbox (y 1600 to 2600).
    let (doc, _) = build(&card(json!({ "oracle_text": "a", "power": null, "toughness": null, "type_line": "Instant" })), &Options::default());
    let i = ink(&doc, "rules-n");
    assert!(((i[1] + i[3]) / 2.0 - 2100.0).abs() <= 1.0 && ((i[0] + i[2]) / 2.0 - 1000.0).abs() <= 6.0, "{i:?}");
    // A lot of text is shrunk until it fits the textbox less the spare room (39 px here).
    let long = vec!["ab ab ab ab ab ab ab ab ab ab"; 12].join("\n");
    let (doc, _) = build(&card(json!({ "oracle_text": long, "power": null, "toughness": null, "type_line": "Instant" })), &Options::default());
    let Layer::Text(rules) = layer(&doc, "rules-n") else { panic!() };
    let layout = layout_text(rules, &fonts());
    let i = layout.ink.unwrap();
    assert!(layout.scale < 1.0 && i[3] - i[1] <= 961.0 && i[1] >= 1600.0 && i[3] <= 2600.0, "{} {i:?}", layout.scale);
    // On a creature, lines that would run into the P/T box (x from 1500, y from 2400) are lifted clear of it.
    let wide = vec!["bbbbbbbbbbbbbb"; 8].join("\n");
    let (doc, _) = build(&card(json!({ "oracle_text": wide })), &Options::default());
    let Layer::Text(rules) = layer(&doc, "rules-c") else { panic!() };
    let layout = layout_text(rules, &fonts());
    assert!(layout.line_ink.iter().flatten().all(|l| l[2] <= 1500.0 || l[3] <= 2400.0), "{:?}", layout.line_ink);
}

#[test]
fn flavor_text_brings_the_divider_midway_and_options_can_drop_text() {
    let flavored = card(json!({ "oracle_text": "a (b)", "flavor_text": "d", "power": null, "toughness": null, "type_line": "Instant" }));
    // The test font stands in for the italic one, so the flavor text can be measured.
    let options = Options { font_italic: "Blocks-Regular".into(), ..Options::default() };
    let (doc, _) = build(&flavored, &options);
    let Layer::Text(rules) = layer(&doc, "rules-n") else { panic!() };
    let layout = layout_text(rules, &fonts());
    let (above, below) = (layout.paragraph_ink[0].unwrap(), layout.paragraph_ink[1].unwrap());
    let Layer::Pixel(divider) = layer(&doc, "divider") else { panic!() };
    assert!(divider.base.visible);
    // The divider image is 20 px tall.
    assert!((divider.transform[5] + 10.0 - (above[3] + below[1]) / 2.0).abs() <= 1.0);
    assert_eq!(divider.transform[2], 500.0);

    let (doc, _) = build(&flavored, &Options { flavor_divider: false, ..options.clone() });
    assert!(!layer(&doc, "divider").base().visible);
    let (doc, _) = build(&flavored, &Options { remove_flavor: true, remove_reminder: true, ..Options::default() });
    assert_eq!(runs(&doc, "rules-n"), "a");
    assert!(!layer(&doc, "divider").base().visible);
}

#[test]
fn art_covers_its_area_centered_just_above_the_placeholder() {
    let wide = Options { art: Some(Art { asset: "pic".into(), path: "assets/pic.png".into(), width: 400, height: 300 }), ..Options::default() };
    let (doc, _) = build(&card(json!({})), &wide);
    assert_eq!(doc.root.children[1].id(), "card-art");
    let Layer::Art(art) = layer(&doc, "card-art") else { panic!() };
    // The normal area is 1600 x 1000: the 400 x 300 picture is scaled by 4 and overhangs top and bottom by 100.
    assert_eq!(art.transform, [4.0, 0.0, 200.0, 0.0, 4.0, 300.0]);
    assert_eq!((art.base.role.as_deref(), art.asset.as_deref()), (Some("card.art"), Some("pic")));
    assert_eq!(doc.assets["pic"].path, "assets/pic.png");
    // Tall art goes in the full-art area (1800 x 2800).
    let tall = Options { art: Some(Art { asset: "pic".into(), path: "assets/pic.png".into(), width: 300, height: 600 }), ..Options::default() };
    let (doc, _) = build(&card(json!({})), &tall);
    let Layer::Art(art) = layer(&doc, "card-art") else { panic!() };
    assert_eq!(art.transform, [6.0, 0.0, 100.0, 0.0, 6.0, -300.0]);
}

#[test]
fn a_card_with_no_collector_number_keeps_the_plain_set_and_artist_lines() {
    let (doc, _) = build(&card(json!({ "collector_number": null })), &Options::default());
    assert!(!layer(&doc, "collector").base().visible);
    assert_eq!((runs(&doc, "set"), runs(&doc, "artist")), ("TST * EN".into(), "bb".into()));
}

#[test]
fn vector_templates_are_refused_for_now() {
    let mut template = template();
    template.bindings.kind = "vector".into();
    let error = build_card(&template, &card(json!({})), &fonts(), &Options::default()).unwrap_err();
    assert!(error.contains("only raster templates"), "{error}");
}
