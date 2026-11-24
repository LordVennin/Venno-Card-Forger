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
    // And it does not leave the top of the textbox to do so: it is fitted again in less room.
    assert!(layout.ink.unwrap()[1] >= 1619.0 && layout.scale < 1.0, "{:?} {}", layout.ink, layout.scale);
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
fn a_template_with_no_bindings_is_refused_with_a_reason() {
    let mut template = template();
    template.bindings.kind = String::new();
    let error = build_card(&template, &card(json!({})), &fonts(), &Options::default()).unwrap_err();
    assert!(error.contains("no layer bindings"), "{error}");
}

// ---- the borderless vector template --------------------------------------------

/// A cut-down copy of the borderless vector template's structure: shapes by
/// textbox height, the groups that get colored, and a pinlines mask per height.
fn vector_template() -> Template {
    let sizes = ["Tall", "Normal", "Medium", "Short"];
    let by_size = |prefix: &str, top: f64| -> Vec<Value> { sizes.iter().enumerate().map(|(i, s)| rect(&format!("{prefix}-{s}"), s, false, [200.0, top + 100.0 * i as f64, 1800.0, top + 100.0 * i as f64 + 100.0])).collect() };
    let masked = |name: &str| { let mut l = with(base("pixel", &format!("cut-{name}"), name, true), json!({ "asset": "m", "transform": [1, 0, 0, 0, 1, 0] })); l["mask"] = json!({ "asset": format!("cut-{name}"), "offset": { "x": 0, "y": 0 } }); l };
    let rules = |size: &str| group(&format!("t-{size}"), size, true, vec![
        text(&format!("rules-n-{size}"), "Rules Text - Noncreature", false, "Words", json!({ "box": { "x": 250, "y": 1650, "w": 1500, "h": 900 }, "effectsEnabled": false })),
        text(&format!("rules-c-{size}"), "Rules Text - Creature", false, "Words", json!({ "box": { "x": 250, "y": 1650, "w": 1500, "h": 900 }, "effectsEnabled": false })),
    ]);
    let mut assets = json!({ "m": { "kind": "image", "path": "assets/m.png", "width": 4, "height": 4 }, "ref": { "kind": "image", "path": "assets/ref.png", "width": 1600, "height": 800 } });
    for size in sizes {
        assets[format!("cut-{size}")] = json!({ "kind": "image", "path": format!("assets/cut-{size}.png"), "width": 4, "height": 4 });
    }
    let doc = json!({
        "version": 1, "width": 2000, "height": 3000, "dpi": 720, "assets": assets,
        "root": with(base("group", "root", "Root", true), json!({ "passThrough": true, "children": [
            with(base("pixel", "placeholder", "Layer 1", true), json!({ "asset": "m", "transform": [1, 0, 0, 0, 1, 0] })),
            rect("art-short", "Art Frame Short", false, [0.0, 0.0, 2000.0, 2000.0]),
            rect("art-tall", "Art Frame Tall", false, [0.0, 0.0, 2000.0, 1500.0]),
            group("textbox", "Textbox", true, vec![group("textbox-shape", "Shape", true, by_size("tb", 1600.0))]),
            with(group("pinlines", "Pinlines", true, vec![group("pin-shape", "Shape", true, vec![
                group("pin-name", "Card Name", true, vec![group("pin-name-normal", "Normal", false, vec![])]),
                group("pin-type", "Typeline", true, by_size("pin-type", 1400.0)),
                group("pin-textbox", "Textbox", true, by_size("pin-tb", 1600.0)),
            ])]), json!({ "vectorMask": { "path": { "d": "M0 0L2000 0L2000 500L0 500Z" }, "inverted": true, "enabled": false } })),
            group("crown", "Legendary Crown", true, vec![group("crown-inner", "Legendary Crown", false, vec![])]),
            group("twins", "Name & Title Boxes", true, vec![group("twins-shape", "Shape", true, vec![
                group("twins-name", "Card Name", true, vec![rect("twins-name-normal", "Normal", false, [200.0, 200.0, 1800.0, 350.0])]),
                group("twins-type", "Typeline", true, by_size("twins-type", 1400.0)),
            ])]),
            group("ptbox", "PT Box", false, vec![]),
            group("g-text", "Text and Icons", true, vec![
                rules("Tall"), rules("Normal"), rules("Medium"), rules("Short"),
                group("refs", "Textbox Reference", true, sizes.iter().enumerate().map(|(i, s)| with(base("pixel", &format!("ref-{s}"), s, false), json!({ "asset": "ref", "transform": [1, 0, 200, 0, 1, 1700 + 100 * i] }))).collect()),
                rect("ref-pt", "PT Reference", false, [1500.0, 2400.0, 1800.0, 2600.0]),
                text("type", "Typeline", true, "Type", json!({ "origin": { "x": 250, "y": 1470 }, "effectsEnabled": false })),
                text("pt-text", "Power / Toughness", false, "0/0", json!({ "origin": { "x": 1650, "y": 2550 }, "effectsEnabled": false })),
                text("mana", "Mana Cost", true, "Mana", json!({ "origin": { "x": 1800, "y": 300 } })),
                text("name", "Card Name", true, "Name", json!({ "origin": { "x": 250, "y": 300 }, "effectsEnabled": false })),
            ]),
            group("masks", "Masks", true, vec![group("mask-pins", "Pinlines", true, sizes.iter().map(|s| masked(s)).collect())]),
        ] }))
    });
    let text_ids: serde_json::Map<String, Value> = sizes.iter().flat_map(|s| [(format!("rulesCreature.{}", s.to_lowercase()), json!(format!("rules-c-{s}"))), (format!("rulesNoncreature.{}", s.to_lowercase()), json!(format!("rules-n-{s}")))]).chain([("name", "name"), ("mana", "mana"), ("type", "type"), ("pt", "pt-text")].map(|(k, v)| (k.to_string(), json!(v)))).collect();
    let manifest = json!({ "id": "mini-vector", "bindings": {
        "kind": "vector", "layouts": ["normal"],
        "artRefs": { "short": "art-short", "tall": "art-tall" },
        "groups": { "text": "g-text", "masks": "masks", "pinlinesShapes": "pinlines", "textboxShapes": "textbox", "twinsShapes": "twins", "ptBoxShapes": "ptbox", "crownShapes": "crown" },
        "text": text_ids, "textRefs": { "textbox": "refs", "pt": "ref-pt" }, "toggles": {}, "layers": {}, "art": "placeholder"
    } });
    Template::from_json(&doc.to_string(), &manifest.to_string()).unwrap()
}

fn build_vector(card: &CardData) -> (Doc, Vec<String>) {
    let template = vector_template();
    let options = Options { font_italic: "Blocks-Regular".into(), art: Some(Art { asset: "pic".into(), path: "assets/pic.png".into(), width: 400, height: 300 }), ..Options::default() };
    let built = build_card(&template, card, &fonts(), &options).unwrap();
    let mut doc = template.doc.clone();
    let undo = apply(&mut doc, &built.edit).unwrap();
    // Like every build, one step that undoes back to the template.
    let mut back = doc.clone();
    apply(&mut back, &undo).unwrap();
    assert_eq!(back, template.doc);
    (doc, built.warnings)
}

fn color_of(doc: &Doc, id: &str) -> Value {
    let layer = serde_json::to_value(layer(doc, id)).unwrap();
    assert_eq!((layer["clip"].as_bool(), layer["kind"].as_str()), (Some(true), Some("shape")), "{id}");
    if layer["effects"].is_array() { layer["effects"][0]["gradient"]["stops"].clone() } else { layer["fill"]["color"].clone() }
}

fn shade(r: u8, g: u8, b: u8) -> Value {
    json!([r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, 1.0])
}

#[test]
fn a_vector_template_gets_the_shapes_for_its_textbox_height_and_a_color_layer_on_each_part() {
    // One line of rules text: the short textbox.
    let (doc, warnings) = build_vector(&card(json!({ "mana_cost": "{U}", "type_line": "Legendary Creature — Cat" })));
    assert!(warnings.iter().all(|w| w.contains("Proxyglyph")), "{warnings:?}");
    for shown in ["pin-name-normal", "pin-type-Short", "pin-tb-Short", "twins-name-normal", "twins-type-Short", "tb-Short", "ptbox", "crown-inner", "rules-c-Short"] {
        assert!(layer(&doc, shown).base().visible, "{shown}");
    }
    assert!(!layer(&doc, "tb-Tall").base().visible && !layer(&doc, "rules-c-Tall").base().visible);
    // Each colored part has its color clipped just above it.
    let order: Vec<&str> = doc.root.children.iter().map(|l| l.id()).collect();
    for (part, color) in [("textbox", "card-color-textbox"), ("pinlines", "card-color-pinlines"), ("twins", "card-color-name-and-title-boxes"), ("ptbox", "card-color-pt-box")] {
        let at = order.iter().position(|id| *id == part).unwrap();
        assert_eq!(order[at + 1], color);
    }
    assert_eq!(color_of(&doc, "card-color-pinlines"), shade(0, 117, 190));
    assert_eq!(color_of(&doc, "card-color-textbox"), shade(0x04, 0x54, 0x82));
    assert_eq!(color_of(&doc, "card-color-legendary-crown"), shade(0x00, 0x6d, 0xae));
    // The pinlines and the crown are cut by the mask for this height, effects and all, and the pinlines make way for the crown.
    for cut in ["pinlines", "crown"] {
        let mask = layer(&doc, cut).base().mask.clone().unwrap();
        assert_eq!((mask.asset.as_str(), mask.hides_effects, mask.enabled), ("cut-Short", Some(true), Some(true)), "{cut}");
    }
    assert_eq!(layer(&doc, "pinlines").base().vector_mask.as_ref().unwrap().enabled, Some(true));
    // Text shadows are switched on; the type line moved down with its bar (300 px below the tall one).
    for text in ["name", "type", "pt-text", "rules-c-Short"] {
        assert_eq!(layer(&doc, text).base().effects_enabled, Some(true), "{text}");
    }
    let Layer::Text(type_line) = layer(&doc, "type") else { panic!() };
    assert_eq!(type_line.origin.unwrap().y, 1770.0);
    // Rules text is fitted to this height's reference area, and the art to this height's frame.
    let Layer::Text(rules) = layer(&doc, "rules-c-Short") else { panic!() };
    assert!(rules.bounds.y >= 2000.0 && rules.bounds.y + rules.bounds.h <= 2800.0, "{:?}", rules.bounds);
    let Layer::Art(art) = layer(&doc, "card-art") else { panic!() };
    assert_eq!(art.frame_ref.h, 2000.0);
}

#[test]
fn a_vector_template_grows_its_textbox_with_the_text_and_blends_two_colors() {
    let lines = |n: usize| vec!["a"; n].join("\n");
    let tall = |n: usize, flavor: &str| build_vector(&card(json!({ "oracle_text": lines(n), "flavor_text": flavor, "power": null, "toughness": null, "type_line": "Instant" }))).0;
    assert!(layer(&tall(4, ""), "tb-Short").base().visible);
    assert!(layer(&tall(5, ""), "tb-Medium").base().visible);
    assert!(layer(&tall(6, ""), "tb-Normal").base().visible);
    assert!(layer(&tall(7, ""), "tb-Tall").base().visible);
    // Flavor text counts its own lines and one more.
    assert!(layer(&tall(3, "d"), "tb-Medium").base().visible);
    // In the tallest textbox nothing moves, and a noncreature has no P/T box.
    let doc = tall(7, "");
    let Layer::Text(type_line) = layer(&doc, "type") else { panic!() };
    assert_eq!(type_line.origin.unwrap().y, 1470.0);
    assert!(!layer(&doc, "ptbox").base().visible && tree::find(&doc, "card-color-pt-box").is_none());

    // Two colors: pinlines and textbox blend from the first to the second; the name boxes are gold.
    let (doc, _) = build_vector(&card(json!({ "mana_cost": "{B}{G}", "type_line": "Enchantment", "power": null, "toughness": null })));
    let stops = color_of(&doc, "card-color-pinlines");
    assert_eq!((stops[0]["offset"].as_f64(), stops[0]["color"].clone(), stops[1]["offset"].as_f64(), stops[1]["color"].clone()), (Some(0.40), shade(39, 38, 36), Some(0.60), shade(0, 123, 67)));
    assert_eq!(color_of(&doc, "card-color-textbox")[1]["color"], shade(0x13, 0x4f, 0x23));
    assert_eq!(color_of(&doc, "card-color-name-and-title-boxes"), shade(0x9a, 0x88, 0x3f));
    // Three colors: gold throughout.
    let (doc, _) = build_vector(&card(json!({ "mana_cost": "{W}{U}{B}", "type_line": "Instant", "power": null, "toughness": null })));
    assert_eq!(color_of(&doc, "card-color-pinlines"), shade(246, 210, 98));
}
