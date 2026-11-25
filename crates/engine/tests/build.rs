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
    for size in sizes.iter().chain(["Textless", "1/2"].iter()) {
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
                group("pin-type", "Typeline", true, [by_size("pin-type", 1400.0), vec![rect("pin-type-Textless", "Textless", false, [200.0, 2700.0, 1800.0, 2800.0])]].concat()),
                group("pin-textbox", "Textbox", true, by_size("pin-tb", 1600.0)),
            ])]), json!({ "vectorMask": { "path": { "d": "M0 0L2000 0L2000 500L0 500Z" }, "inverted": true, "enabled": false } })),
            group("crown", "Legendary Crown", true, vec![group("crown-inner", "Legendary Crown", false, vec![])]),
            group("twins", "Name & Title Boxes", true, vec![group("twins-shape", "Shape", true, vec![
                group("twins-name", "Card Name", true, vec![rect("twins-name-normal", "Normal", false, [200.0, 200.0, 1800.0, 350.0])]),
                group("twins-type", "Typeline", true, [by_size("twins-type", 1400.0), vec![rect("twins-type-Textless", "Textless", false, [200.0, 2700.0, 1800.0, 2800.0])]].concat()),
            ])]),
            group("ptbox", "PT Box", false, vec![group("ptbox-shape", "Shape", true, vec![rect("ptbox-box", "PT Box", true, [1500.0, 2400.0, 1800.0, 2600.0])])]),
            group("ptbox-low", "PT Box Textless", false, vec![group("ptbox-low-shape", "Shape", true, vec![rect("ptbox-low-box", "PT Box", true, [1500.0, 2700.0, 1800.0, 2900.0])])]),
            group("border", "Border", true, vec![]),
            group("dot", "Color Indicator", false, vec![
                group("dot-shape", "Shape", true, vec![rect("dot-circle", "Circle", true, [210.0, 1420.0, 250.0, 1460.0])]),
                with(rect("dot-2", "2", false, [229.0, 1420.0, 231.0, 1460.0]), json!({ "clip": true })),
            ]),
            group("g-text", "Text and Icons", true, vec![
                rules("Tall"), rules("Normal"), rules("Medium"), rules("Short"),
                group("refs", "Textbox Reference", true, sizes.iter().enumerate().map(|(i, s)| with(base("pixel", &format!("ref-{s}"), s, false), json!({ "asset": "ref", "transform": [1, 0, 200, 0, 1, 1700 + 100 * i] }))).collect()),
                rect("ref-pt", "PT Reference", false, [1500.0, 2400.0, 1800.0, 2600.0]),
                text("type", "Typeline", true, "Type", json!({ "origin": { "x": 250, "y": 1470 }, "effectsEnabled": false })),
                text("type-shift", "Typeline Shift", false, "Type", json!({ "origin": { "x": 300, "y": 1470 }, "effectsEnabled": false })),
                rect("ref-expansion", "Expansion Reference", false, [1600.0, 1400.0, 1750.0, 1500.0]),
                text("pt-text", "Power / Toughness", false, "0/0", json!({ "origin": { "x": 1650, "y": 2550 }, "effectsEnabled": false })),
                text("mana", "Mana Cost", true, "Mana", json!({ "origin": { "x": 1800, "y": 300 } })),
                text("name", "Card Name", true, "Name", json!({ "origin": { "x": 250, "y": 300 }, "effectsEnabled": false })),
            ]),
            group("masks", "Masks", true, vec![
                group("mask-pins", "Pinlines", true, sizes.iter().map(|s| masked(s)).chain([masked("Textless")]).collect()),
                group("mask-border", "Border", true, vec![{ let mut cut = masked("Textless"); cut["id"] = json!("cut-border-Textless"); cut }]),
                group("mask-dot", "Color Indicator", true, vec![masked("1/2")]),
            ]),
        ] }))
    });
    let text_ids: serde_json::Map<String, Value> = sizes.iter().flat_map(|s| [(format!("rulesCreature.{}", s.to_lowercase()), json!(format!("rules-c-{s}"))), (format!("rulesNoncreature.{}", s.to_lowercase()), json!(format!("rules-n-{s}")))]).chain([("name", "name"), ("mana", "mana"), ("type", "type"), ("typeShift", "type-shift"), ("pt", "pt-text")].map(|(k, v)| (k.to_string(), json!(v)))).collect();
    let manifest = json!({ "id": "mini-vector", "bindings": {
        "kind": "vector", "layouts": ["normal"],
        "artRefs": { "short": "art-short", "tall": "art-tall", "textless": "art-short" },
        "groups": { "text": "g-text", "masks": "masks", "border": "border", "pinlinesShapes": "pinlines", "textboxShapes": "textbox", "twinsShapes": "twins", "ptBoxShapes": "ptbox", "crownShapes": "crown" },
        "text": text_ids, "textRefs": { "textbox": "refs", "pt": "ref-pt", "expansion": "ref-expansion" }, "toggles": {}, "layers": {}, "art": "placeholder"
    } });
    Template::from_json(&doc.to_string(), &manifest.to_string()).unwrap()
}

fn build_vector(card: &CardData) -> (Doc, Vec<String>) {
    build_vector_with(card, Options::default())
}

fn build_vector_with(card: &CardData, more: Options) -> (Doc, Vec<String>) {
    let template = vector_template();
    let options = Options { font_italic: "Blocks-Regular".into(), art: Some(Art { asset: "pic".into(), path: "assets/pic.png".into(), width: 400, height: 300 }), ..more };
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

#[test]
fn the_set_symbol_fills_its_areas_height_at_the_right_and_the_type_line_stops_short_of_it() {
    // A 20 x 10 drawing in an area 150 x 100 ending at x 1750: ten times the size, so 200 wide from x 1550.
    let symbol = cardsmith_engine::svg::parse_svg(r##"<svg width="20" height="10"><path d="M0 0h20v10H0z" fill="#ff0000"/></svg>"##).unwrap();
    let options = Options { symbol: Some(symbol), set_size: Some(280), ..Options::default() };
    let (doc, warnings) = build(&card(json!({ "type_line": "bbbbbbbbbbbbbbbbbbbb" })), &options);
    assert!(warnings.iter().all(|w| w.contains("Proxyglyph")), "{warnings:?}");
    let Layer::Group(group) = layer(&doc, "card-symbol") else { panic!() };
    assert_eq!((group.base.role.as_deref(), group.children.len()), (Some("card.symbol"), 1));
    let Layer::Shape(shape) = &group.children[0] else { panic!() };
    assert_eq!(shape.path.d, "M1550 1450L1750 1450L1750 1550L1550 1550Z");
    // It sits just above its reference area in the text group.
    let Layer::Group(text) = layer(&doc, "g-text") else { panic!() };
    let at = text.children.iter().position(|l| l.id() == "ref-expansion").unwrap();
    assert_eq!(text.children[at + 1].id(), "card-symbol");
    // The type line clears the symbol by the usual gap (18 px on this template).
    assert!(ink(&doc, "type")[2] <= 1532.0, "{:?}", ink(&doc, "type"));
    // With the set's size known, the collector line is number over size.
    assert_eq!(runs(&doc, "col-top"), "007/280 R");
}

fn tiny_svg() -> cardsmith_engine::svg::SvgArt {
    cardsmith_engine::svg::parse_svg(r##"<svg width="20" height="10"><path d="M0 0h20v10H0z" fill="#ff0000"/></svg>"##).unwrap()
}

#[test]
fn a_vector_card_with_no_rules_text_has_no_textbox_and_its_type_line_at_the_foot() {
    let blank = card(json!({ "oracle_text": "", "mana_cost": "{U}" }));
    // No set symbol: the creature keeps a P/T box, the lower one, and its numbers move down with it.
    let (doc, warnings) = build_vector(&blank);
    assert!(warnings.iter().all(|w| w.contains("Proxyglyph")), "{warnings:?}");
    for shown in ["pin-type-Textless", "twins-type-Textless", "ptbox-low"] {
        assert!(layer(&doc, shown).base().visible, "{shown}");
    }
    assert!(!layer(&doc, "ptbox").base().visible && !layer(&doc, "tb-Short").base().visible && tree::find(&doc, "card-color-textbox").is_none());
    assert!(["rules-c-Short", "rules-c-Tall"].iter().all(|id| !layer(&doc, id).base().visible));
    assert_eq!(layer(&doc, "pinlines").base().mask.as_ref().unwrap().asset, "cut-Textless");
    // The border is cut for it, without hiding its effects.
    let border = layer(&doc, "border").base().mask.clone().unwrap();
    assert_eq!((border.asset.as_str(), border.hides_effects), ("cut-Textless", None));
    let Layer::Text(pt) = layer(&doc, "pt-text") else { panic!() };
    assert_eq!((pt.origin.unwrap().y, runs(&doc, "pt-text")), (2850.0, "2/3".into()));
    // The type line is 1300 px lower than in the tall frame and a touch to the left (10 px at 3264 wide: 6 here).
    let Layer::Text(type_line) = layer(&doc, "type") else { panic!() };
    assert_eq!((type_line.origin.unwrap().x, type_line.origin.unwrap().y, runs(&doc, "type")), (244.0, 2770.0, "Creature — Cat".into()));

    // With a set symbol the corner is the symbol's: power and toughness end the type line, and there is no box.
    let (doc, _) = build_vector_with(&blank, Options { symbol: Some(tiny_svg()), ..Options::default() });
    assert_eq!(runs(&doc, "type"), "Creature — Cat — 2/3");
    assert!(!layer(&doc, "ptbox-low").base().visible && !layer(&doc, "pt-text").base().visible);
    let Layer::Group(symbol) = layer(&doc, "card-symbol") else { panic!() };
    let Layer::Shape(shape) = &symbol.children[0] else { panic!() };
    // The symbol follows the type line down (1300) and sits 6 px further right.
    assert!(shape.path.d.starts_with("M1556 2700"), "{}", shape.path.d);
}

#[test]
fn a_vector_card_shows_its_color_indicator_split_for_two_colors() {
    let (doc, warnings) = build_vector(&card(json!({ "mana_cost": "", "color_indicator": ["W", "U"], "colors": ["W", "U"], "oracle_text": "a\na\na\na\na\na\na" })));
    assert!(warnings.iter().all(|w| w.contains("Proxyglyph")), "{warnings:?}");
    assert!(layer(&doc, "dot").base().visible && layer(&doc, "dot-2").base().visible && layer(&doc, "type-shift").base().visible);
    // Two color layers clipped to the dot's shape: the first color underneath, the second on top, cut in half.
    let Layer::Group(dot) = layer(&doc, "dot") else { panic!() };
    let order: Vec<&str> = dot.children.iter().map(|l| l.id()).collect();
    assert_eq!(order, ["dot-shape", "card-color-color-indicator-w", "card-color-color-indicator-u", "dot-2"]);
    assert_eq!(color_of(&doc, "card-color-color-indicator-w"), shade(248, 244, 240));
    assert_eq!(layer(&doc, "card-color-color-indicator-u").base().mask.as_ref().unwrap().asset, "cut-1/2");
    assert!(layer(&doc, "card-color-color-indicator-w").base().mask.is_none());

    // In a shorter textbox the dot moves down with the type line, shapes, colors and mask alike.
    let (doc, _) = build_vector(&card(json!({ "mana_cost": "", "color_indicator": ["W", "U"], "colors": ["W", "U"] })));
    let Layer::Shape(circle) = layer(&doc, "dot-circle") else { panic!() };
    assert!(circle.path.d.starts_with("M210 1720"), "{}", circle.path.d);
    assert_eq!(layer(&doc, "card-color-color-indicator-u").base().mask.as_ref().unwrap().offset.y, 300.0);
}

#[test]
fn a_basic_land_shows_its_land_symbol_in_place_of_rules_text() {
    let land = card(json!({ "name": "Island", "mana_cost": "", "type_line": "Basic Land — Island", "oracle_text": "({T}: Add {U}.)", "power": null, "toughness": null }));
    let (doc, _) = build(&land, &Options { basic_watermark: Some(tiny_svg()), ..Options::default() });
    // Under the text group, three quarters of the textbox's height (1000), centered in it.
    let at = doc.root.children.iter().position(|l| l.id() == "g-text").unwrap();
    assert_eq!(doc.root.children[at - 1].id(), "card-watermark");
    let Layer::Group(mark) = layer(&doc, "card-watermark") else { panic!() };
    let shape = serde_json::to_value(&mark.children[0]).unwrap();
    assert_eq!(shape["path"]["d"], "M250 1725L1750 1725L1750 2475L250 2475Z");
    assert_eq!(shape["fill"]["color"], shade(0, 115, 178));
    assert!(matches!(mark.base.effects.as_deref(), Some([cardsmith_core::doc::Effect::Bevel { .. }])));
    assert!(!layer(&doc, "rules-n").base().visible && !layer(&doc, "rules-c").base().visible);
    // Without the symbol the rules text is written as usual; other cards never get one.
    let (doc, _) = build(&land, &Options::default());
    assert!(layer(&doc, "rules-n").base().visible && tree::find(&doc, "card-watermark").is_none());
    let (doc, _) = build(&card(json!({})), &Options { basic_watermark: Some(tiny_svg()), ..Options::default() });
    assert!(tree::find(&doc, "card-watermark").is_none());
}

// ---- filling around the art ------------------------------------------------------

fn art(width: u32, height: u32) -> Options {
    Options { art: Some(Art { asset: "pic".into(), path: "assets/pic.png".into(), width, height }), ..Options::default() }
}

#[test]
fn an_ordinary_frame_needs_no_fill_but_a_full_art_one_does_unless_the_art_is_tall() {
    let template = template();
    // An ordinary card: the frame hides everything past the art area.
    let built = build_card(&template, &card(json!({})), &fonts(), &art(400, 300)).unwrap();
    assert!(built.fill.is_none());
    // A colorless card is full-art: its frame shows past the ordinary art area (200..1800 x 400..1400).
    let eldrazi = card(json!({ "mana_cost": "{7}", "type_line": "Creature — Eldrazi", "colors": [] }));
    let built = build_card(&template, &eldrazi, &fonts(), &art(400, 300)).unwrap();
    let plan = built.fill.clone().expect("a full-art frame is filled");
    // The art is 1600 x 1200 from (200, 300). The template is 2000 px wide, so Proxyshop's 10 px margin is 6.
    assert_eq!((plan.art_position, plan.art_scale, plan.canvas), ((200.0, 300.0), 4.0, (2000, 3000)));
    assert_eq!(plan.keep, [206.0, 306.0, 1794.0, 1494.0]);
    let mut doc = template.doc.clone();
    apply(&mut doc, &built.edit).unwrap();
    // The art shows only inside the trusted area, with its corners rounded.
    let mask = layer(&doc, "card-art").base().vector_mask.clone().unwrap();
    assert!(mask.path.d.starts_with("M") && mask.enabled.is_none() && mask.inverted.is_none(), "{}", mask.path.d);
    // The finished fill goes on its own layer directly beneath the art, and remembers how it was made.
    let edit = plan.edit("card-fill", "assets/card-fill.png", 500, 750, 4.0, "patchmatch", json!({ "seed": 1 })).unwrap();
    let undo = apply(&mut doc, &edit).unwrap();
    let order: Vec<&str> = doc.root.children.iter().map(|l| l.id()).take(3).collect();
    assert_eq!(order, ["placeholder", "card-fill", "card-art"]);
    let Layer::Fill(fill) = layer(&doc, "card-fill") else { panic!() };
    assert_eq!((fill.source.as_str(), fill.params["seed"].as_i64(), fill.transform, fill.base.role.as_deref()), ("patchmatch", Some(1), [4.0, 0.0, 0.0, 0.0, 4.0, 0.0], Some("card.fill")));
    assert_eq!((doc.assets["card-fill"].width, doc.assets["card-fill"].height), (Some(500), Some(750)));
    // Adding the fill is its own step: undoing it leaves the card as built.
    apply(&mut doc, &undo).unwrap();
    assert!(tree::find(&doc, "card-fill").is_none() && tree::find(&doc, "card-art").is_some());
    // Tall art goes in the full-art area, which the frame was made for: nothing to fill.
    assert!(build_card(&template, &eldrazi, &fonts(), &art(300, 600)).unwrap().fill.is_none());
}

#[test]
fn extended_and_borderless_templates_always_fill_and_trust_the_art_up_to_the_cards_edge() {
    // The made-up vector template: its short art area is the top 2000 px of the card, edge to edge.
    let template = vector_template();
    let built = build_card(&template, &card(json!({})), &fonts(), &Options { font_italic: "Blocks-Regular".into(), ..art(400, 300) }).unwrap();
    let plan = built.fill.expect("a borderless frame is always filled");
    // 400 x 300 covers 2000 x 2000 at 6.67: it overhangs left and right, so only the bottom edge is pulled in.
    assert_eq!((plan.keep[0], plan.keep[1], plan.keep[2], plan.keep[3]), (0.0, 0.0, 2000.0, 1994.0));
    // A raster template says so in its bindings.
    let mut extended = self::template();
    extended.bindings.art_fill = Some("always".into());
    assert!(build_card(&extended, &card(json!({})), &fonts(), &art(400, 300)).unwrap().fill.is_some());
}

// ---- the workbench: building and rebuilding in an open document ---------------------

use cardsmith_core::session::Session;
use cardsmith_engine::workbench::{BuildRequest, Workbench};

fn request(card: &CardData) -> BuildRequest {
    serde_json::from_value(json!({ "card": card })).unwrap()
}

fn picture(width: u32, height: u32) -> Vec<u8> {
    cardsmith_core::render::encode_png(width, height, &[90, 120, 60, 255].repeat((width * height) as usize)).unwrap()
}

fn bench() -> (Workbench, Session) {
    let bench = Workbench::new(template());
    let session = Session::with_fonts(bench.doc(), Default::default(), fonts());
    (bench, session)
}

#[track_caller]
fn assert_frame_is_exact(session: &mut Session, images: &[(&str, Vec<u8>)]) {
    let mut renderer = cardsmith_core::render::Renderer::new();
    *renderer.fonts_mut() = fonts();
    let assets = images.iter().map(|(id, bytes)| (id.to_string(), bytes.clone())).collect();
    let full = renderer.render(session.doc(), &assets).pixels;
    let differing = full.chunks_exact(4).zip(session.frame().chunks_exact(4)).filter(|(a, b)| a != b).count();
    assert_eq!(differing, 0, "the session's picture differs from a full render at {differing} pixels");
}

#[test]
fn rebuilding_in_an_open_document_gives_what_a_fresh_build_would_as_one_step() {
    let (mut bench, mut session) = bench();
    assert_eq!(bench.set_art(&mut session, picture(40, 30)).unwrap(), (40, 30));
    let first = card(json!({}));
    let second = card(json!({ "name": "bbbb", "mana_cost": "{7}", "type_line": "Creature — Eldrazi", "colors": [], "oracle_text": "a\nb" }));

    let report = bench.build(&mut session, &request(&first)).unwrap();
    assert!(report.fill.is_none());
    assert_eq!(session.labels(), ["Build ab"]);
    let again = bench.build(&mut session, &request(&second)).unwrap();
    assert_eq!(session.labels(), ["Build ab", "Update bbbb"]);
    assert!(again.build != report.build);

    // The same document a fresh build of the second card gives, but for the note of which build it is.
    let art = Options { art: Some(Art { asset: "card-art-1".into(), path: "assets/card-art-1.png".into(), width: 40, height: 30 }), ..Options::default() };
    let (mut fresh, _) = build(&second, &art);
    let mut rebuilt = session.doc().clone();
    assert_eq!(rebuilt.card.as_ref().unwrap()["_build"], json!(again.build));
    (rebuilt.card, fresh.card) = (None, None);
    assert!(rebuilt == fresh, "a rebuilt card differs from a freshly built one");

    // Only what differs between the two cards was drawn again, and the picture is still
    // exactly what drawing the whole document afresh gives.
    assert_frame_is_exact(&mut session, &[("card-art-1", picture(40, 30))]);
    let third = card(json!({ "name": "bbbbb", "mana_cost": "{7}", "type_line": "Creature — Eldrazi", "colors": [], "oracle_text": "a\nb" }));
    bench.build(&mut session, &request(&third)).unwrap();
    assert_frame_is_exact(&mut session, &[("card-art-1", picture(40, 30))]);
    assert!(session.undo());

    // Each build is one step back, and the first step back is the bare template.
    assert!(session.undo());
    assert_eq!(runs(session.doc(), "name"), "ab");
    assert!(session.undo());
    assert!(*session.doc() == template().doc);
}

#[test]
fn a_fill_goes_under_the_art_of_the_card_it_was_made_for_and_leaves_with_it() {
    let (mut bench, mut session) = bench();
    bench.set_art(&mut session, picture(400, 300)).unwrap();
    let eldrazi = card(json!({ "mana_cost": "{7}", "type_line": "Creature — Eldrazi", "colors": [] }));
    let report = bench.build(&mut session, &request(&eldrazi)).unwrap();
    let fill = report.fill.clone().expect("a full-art frame is filled");
    assert_eq!((fill.art_position, fill.art_scale, fill.canvas), ((200.0, 300.0), 4.0, (2000, 3000)));

    assert!(bench.add_fill(&mut session, report.build, picture(500, 750), "patchmatch", json!({ "seed": 1 })).unwrap());
    let order: Vec<&str> = session.doc().root.children.iter().map(|l| l.id()).take(3).collect();
    assert_eq!(order, ["placeholder", "card-fill", "card-art"]);
    let Layer::Fill(layer) = tree::find(session.doc(), "card-fill").unwrap() else { panic!() };
    assert_eq!((layer.source.as_str(), layer.transform), ("patchmatch", [4.0, 0.0, 0.0, 0.0, 4.0, 0.0]));
    // Another fill for the same card replaces the first, as its own step.
    assert!(bench.add_fill(&mut session, report.build, picture(250, 375), "lama", json!({})).unwrap());
    let fills = tree::all_ids(session.doc()).iter().filter(|id| **id == "card-fill").count();
    let Layer::Fill(layer) = tree::find(session.doc(), "card-fill").unwrap() else { panic!() };
    assert_eq!((fills, layer.source.as_str(), layer.transform[0]), (1, "lama", 8.0));

    // Changing the card's text leaves the art where it was, so the fill stays: nothing is made again.
    let renamed = card(json!({ "name": "bbbb", "mana_cost": "{7}", "type_line": "Creature — Eldrazi", "colors": [] }));
    let next = bench.build(&mut session, &request(&renamed)).unwrap();
    assert!(next.fill_kept && next.fill.is_some());
    let order: Vec<&str> = session.doc().root.children.iter().map(|l| l.id()).take(3).collect();
    assert_eq!(order, ["placeholder", "card-fill", "card-art"]);
    let Layer::Fill(layer) = tree::find(session.doc(), "card-fill").unwrap() else { panic!() };
    let kept = layer.asset.clone();
    assert_eq!(layer.source, "lama");
    // A fill that was still being made for the card as it was is as good for the card as it is.
    assert!(bench.add_fill(&mut session, report.build, picture(500, 750), "patchmatch", json!({})).unwrap());
    assert!(session.undo());
    assert_frame_is_exact(&mut session, &[("card-art-1", picture(400, 300)), (kept.as_str(), picture(250, 375))]);

    // Different art sits differently: the fill goes with the build it belonged to...
    bench.set_art(&mut session, picture(400, 200)).unwrap();
    let moved = bench.build(&mut session, &request(&renamed)).unwrap();
    assert!(!moved.fill_kept && moved.fill.is_some() && tree::find(session.doc(), "card-fill").is_none());
    // ...and one still being made for the old art is turned away.
    assert!(!bench.add_fill(&mut session, report.build, picture(500, 750), "patchmatch", json!({})).unwrap());
    assert!(tree::find(session.doc(), "card-fill").is_none());
    // One step back, the card has its fill again.
    assert!(session.undo());
    assert!(tree::find(session.doc(), "card-fill").is_some());
    assert_eq!(runs(session.doc(), "name"), "bbbb");
    assert!(session.undo());
    assert_eq!(runs(session.doc(), "name"), "ab");
    let fill = tree::find(session.doc(), "card-fill").and_then(|l| if let Layer::Fill(f) = l { Some(f.asset.clone()) } else { None }).unwrap();
    assert_frame_is_exact(&mut session, &[("card-art-1", picture(400, 300)), (fill.as_str(), picture(250, 375))]);
}

#[test]
fn layers_added_by_hand_survive_a_rebuild_and_a_bad_picture_is_refused() {
    let (mut bench, mut session) = bench();
    assert!(bench.set_art(&mut session, b"not a picture".to_vec()).unwrap_err().contains("not a picture"));
    bench.build(&mut session, &request(&card(json!({})))).unwrap();
    let paint = session.add_paint_layer("Touch-up").unwrap();
    bench.build(&mut session, &request(&card(json!({ "name": "bbbb" })))).unwrap();
    assert!(tree::find(session.doc(), &paint).is_some());
    assert_eq!(runs(session.doc(), "name"), "bbbb");
    assert_eq!(session.doc().root.children.last().unwrap().id(), paint);
}

#[test]
fn a_card_can_be_built_straight_from_scryfalls_json_so_edits_to_it_change_the_frame() {
    let (mut bench, mut session) = bench();
    let mut raw = json!({ "object": "card", "layout": "normal", "name": "ab", "mana_cost": "{W}", "colors": ["W"], "type_line": "Creature — Cat", "oracle_text": "a", "power": "2", "toughness": "3", "set": "tst", "collector_number": "7", "rarity": "rare", "artist": "bb", "lang": "en" });
    bench.build(&mut session, &serde_json::from_value(json!({ "scryfall": raw })).unwrap()).unwrap();
    assert_eq!(session.doc().card.as_ref().unwrap()["frame"]["background"], "W");
    // The form changes the cost and colors: the same card is now blue.
    raw["mana_cost"] = json!("{U}");
    raw["colors"] = json!(["U"]);
    bench.build(&mut session, &serde_json::from_value(json!({ "scryfall": raw })).unwrap()).unwrap();
    assert_eq!(session.doc().card.as_ref().unwrap()["frame"]["background"], "U");
    assert!(bench.build(&mut session, &serde_json::from_value(json!({})).unwrap()).unwrap_err().contains("no card"));
}

#[test]
fn what_was_changed_by_hand_on_the_cards_layers_survives_a_rebuild_unless_the_engine_changes_it_too() {
    use cardsmith_core::arrange::{place_edit, Placement};
    use cardsmith_core::commands::Edit;
    let (mut bench, mut session) = bench();
    bench.set_art(&mut session, picture(400, 300)).unwrap();
    let eldrazi = json!({ "mana_cost": "{7}", "type_line": "Creature — Eldrazi", "colors": [] });
    let report = bench.build(&mut session, &request(&card(eldrazi.clone()))).unwrap();
    assert!(bench.add_fill(&mut session, report.build, picture(500, 750), "patchmatch", json!({})).unwrap());
    let Layer::Art(art) = layer(session.doc(), "card-art") else { panic!() };
    let placed = art.transform;

    // The art is dragged, the type line recolored, the collector line hidden, and the name moved.
    let drag = place_edit(session.doc(), "card-art", &Placement::shift(30.0, -20.0)).unwrap();
    session.execute(drag, "Move art").unwrap();
    let Layer::Text(kind) = layer(session.doc(), "type") else { panic!() };
    let mut style = serde_json::to_value(&kind.style).unwrap();
    style["color"] = json!([1.0, 0.0, 0.0, 1.0]);
    let props = |v: Value| v.as_object().unwrap().clone();
    session.execute(Edit::SetLayerProps { id: "type".into(), props: props(json!({ "style": style })) }, "Recolor").unwrap();
    session.execute(Edit::SetLayerProps { id: "col-top".into(), props: props(json!({ "visible": false })) }, "Hide").unwrap();

    // The rules text is edited: a rebuild.
    let mut edited = eldrazi.clone();
    edited["oracle_text"] = json!("a\nb");
    let next = bench.build(&mut session, &request(&card(edited.clone()))).unwrap();
    let Layer::Art(art) = layer(session.doc(), "card-art") else { panic!() };
    assert_eq!(art.transform, [placed[0], placed[1], placed[2] + 30.0, placed[3], placed[4], placed[5] - 20.0], "the art stays where it was dragged");
    let Layer::Text(kind) = layer(session.doc(), "type") else { panic!() };
    assert_eq!(kind.style.color, [1.0, 0.0, 0.0, 1.0]);
    assert!(!layer(session.doc(), "col-top").base().visible);
    // The fill was made for where the engine put the art, and the engine's plan has not changed, so it stays too.
    assert!(next.fill_kept);
    assert!(runs(session.doc(), "rules-c").contains('b'));

    // Where the engine changes the same thing, the engine wins: a new type line replaces the old words, but keeps the hand-picked color only if the style is untouched by it.
    edited["type_line"] = json!("Creature — Cat");
    bench.build(&mut session, &request(&card(edited))).unwrap();
    assert_eq!(runs(session.doc(), "type"), "Creature — Cat");
    let Layer::Art(art) = layer(session.doc(), "card-art") else { panic!() };
    assert_eq!(art.transform[2], placed[2] + 30.0);
    // Undo steps back through the rebuilds and the hand changes in order.
    assert!(session.undo() && session.undo());
    assert!(!layer(session.doc(), "col-top").base().visible);
    assert!(session.undo());
    assert!(layer(session.doc(), "col-top").base().visible);
}

#[test]
fn text_shadows_follow_the_template_unless_asked_for_and_are_added_where_the_template_has_none() {
    use cardsmith_core::doc::Effect;
    let shadowed = |doc: &Doc, id: &str| {
        let base = layer(doc, id).base();
        base.effects_enabled == Some(true) && base.effects.iter().flatten().any(|e| matches!(e, Effect::DropShadow { enabled: true, .. }))
    };
    // A raster template leaves them off, as Proxyshop's normal template does.
    let (doc, _) = build(&card(json!({})), &Options::default());
    assert!(!shadowed(&doc, "name") && !shadowed(&doc, "type") && !shadowed(&doc, "rules-c"));
    // Asked for, every line of the card's text casts one; the template's text has none of its own, so one is added.
    let (doc, _) = build(&card(json!({})), &Options { text_shadows: Some(true), ..Options::default() });
    for id in ["name", "type", "rules-c", "pt-text"] {
        assert!(shadowed(&doc, id), "{id}");
    }
    let Some(Effect::DropShadow { distance, size, .. }) = layer(&doc, "name").base().effects.as_ref().and_then(|e| e.last()) else { panic!() };
    // The normal template's 6 px at 3264 wide, for a document 2000 wide.
    assert_eq!((*distance, *size), (3.68, 3.68));
}

#[test]
fn a_saved_card_is_taken_up_again_with_its_fields_and_hand_changes() {
    use cardsmith_core::arrange::{place_edit, Placement};
    use cardsmith_core::serialize::load_cardsmith_available;
    let (mut bench, mut session) = bench();
    bench.set_art(&mut session, picture(400, 300)).unwrap();
    let eldrazi = json!({ "object": "card", "layout": "normal", "name": "ab", "mana_cost": "{7}", "colors": [], "type_line": "Creature — Eldrazi", "oracle_text": "a", "power": "2", "toughness": "3", "set": "tst", "collector_number": "7", "rarity": "rare", "artist": "bb", "lang": "en" });
    let request: BuildRequest = serde_json::from_value(json!({ "scryfall": eldrazi, "artPath": "/pictures/art.png", "textShadows": true })).unwrap();
    let report = bench.build(&mut session, &request).unwrap();
    assert!(bench.add_fill(&mut session, report.build, picture(500, 750), "patchmatch", json!({})).unwrap());
    let drag = place_edit(session.doc(), "card-art", &Placement::shift(30.0, -20.0)).unwrap();
    session.execute(drag, "Move art").unwrap();
    let moved = match layer(session.doc(), "card-art") { Layer::Art(a) => a.transform, _ => panic!() };
    let file = session.save_available().unwrap();

    // Another day: the file is opened and its template found again.
    let (doc, assets) = load_cardsmith_available(&file).unwrap();
    let mut reopened = Session::with_fonts(doc, assets, fonts());
    let (mut bench, request) = Workbench::resume(template(), &reopened).unwrap();
    assert_eq!((request.art_path.as_deref(), request.text_shadows, request.scryfall.as_ref().unwrap()["name"].as_str()), (Some("/pictures/art.png"), Some(true), Some("ab")));
    // The card's name is changed in the form: one step, the art stays where it was dragged, the fill stays.
    let mut edited = request.clone();
    edited.scryfall.as_mut().unwrap()["name"] = json!("bbbb");
    let next = bench.build(&mut reopened, &edited).unwrap();
    assert_eq!(runs(reopened.doc(), "name"), "bbbb");
    let Layer::Art(art) = layer(reopened.doc(), "card-art") else { panic!() };
    assert_eq!(art.transform, moved);
    assert!(next.fill_kept && tree::find(reopened.doc(), "card-fill").is_some());
    assert_eq!(reopened.labels(), ["Update bbbb"]);
    assert!(reopened.undo());
    assert_eq!(runs(reopened.doc(), "name"), "ab");
    // New art gets a name of its own, past the ones the file already uses.
    bench.set_art(&mut reopened, picture(40, 30)).unwrap();
    bench.build(&mut reopened, &edited).unwrap();
    let Layer::Art(art) = layer(reopened.doc(), "card-art") else { panic!() };
    assert!(art.asset.as_deref().is_some_and(|id| id != "card-art-1"), "{:?}", art.asset);

    // A document that is not a built card says so.
    let plain = Session::with_fonts(template().doc, Default::default(), fonts());
    assert!(Workbench::resume(template(), &plain).is_err());
}
