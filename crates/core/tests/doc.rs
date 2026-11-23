//! Ported from packages/doc/src/doc.test.ts.
#![recursion_limit = "256"]

use cardsmith_core::commands::{apply, Edit, History};
use cardsmith_core::doc::build::{group, pixel, rect_path, shape, with};
use cardsmith_core::doc::*;
use cardsmith_core::serialize::*;
use cardsmith_core::tree::find;
use serde_json::{json, Map, Value};

const BLACK: Color = [0.0, 0.0, 0.0, 1.0];
const WHITE: Color = [1.0, 1.0, 1.0, 1.0];
const RED: Color = [1.0, 0.0, 0.0, 1.0];

fn sample() -> Doc {
    let mut doc = Doc::empty(100, 140, 800.0);
    doc.assets.insert("art".into(), AssetRef { kind: AssetKind::Image, path: "assets/art.png".into(), width: Some(2), height: Some(2) });
    doc.root.children = vec![
        shape("bg", &rect_path(0.0, 0.0, 100.0, 140.0), BLACK),
        group("frame", true, vec![pixel("art", "art", IDENTITY), with(shape("pin", &rect_path(5.0, 5.0, 90.0, 2.0), WHITE), |b| b.clip = true)]),
        group("text", false, vec![]),
    ];
    doc
}

fn props(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}
fn set(id: &str, value: Value) -> Edit {
    Edit::SetLayerProps { id: id.into(), props: props(value) }
}
fn ids(doc: &Doc, group_id: &str) -> Vec<String> {
    let children = if group_id == "root" { &doc.root.children[..] } else { find(doc, group_id).unwrap().children() };
    children.iter().map(|l| l.id().to_string()).collect()
}

/// Every edit must restore the exact document on undo and repeat exactly on redo.
fn round_trip(edit: Edit) -> Doc {
    let before = sample();
    let mut history = History::new(before.clone());
    history.execute(edit, "edit").unwrap();
    let after = history.doc().clone();
    assert_ne!(after, before);
    assert!(history.undo());
    assert_eq!(history.doc(), &before);
    assert!(history.redo());
    assert_eq!(history.doc(), &after);
    history.undo();
    assert_eq!(history.doc(), &before);
    after
}

fn fails(edit: Edit, expected: &str) {
    let mut doc = sample();
    let error = apply(&mut doc, &edit).expect_err("the edit should be refused");
    assert!(error.0.contains(expected), "error was: {error}");
    assert_eq!(doc, sample(), "a failed edit must not change the document");
}

#[test]
fn set_layer_props_patches_removes_and_restores() {
    let after = round_trip(set("art", json!({ "opacity": 0.5, "blend": "overlay", "role": "art" })));
    let base = find(&after, "art").unwrap().base();
    assert_eq!((base.opacity, base.blend, base.role.as_deref()), (0.5, BlendMode::Overlay, Some("art")));

    let mut history = History::new(sample());
    history.execute(set("art", json!({ "role": "art" })), "tag").unwrap();
    history.execute(set("art", json!({ "role": null })), "untag").unwrap();
    assert_eq!(find(history.doc(), "art").unwrap().base().role, None);
    history.undo();
    assert_eq!(find(history.doc(), "art").unwrap().base().role.as_deref(), Some("art"));
}

#[test]
fn set_layer_props_reaches_kind_specific_fields_and_the_root() {
    let after = round_trip(set("art", json!({ "transform": [2, 0, 5, 0, 2, 7] })));
    let Layer::Pixel(p) = find(&after, "art").unwrap() else { panic!() };
    assert_eq!(p.transform, [2.0, 0.0, 5.0, 0.0, 2.0, 7.0]);

    let after = round_trip(set("frame", json!({ "passThrough": false, "opacity": 0.25 })));
    let Layer::Group(g) = find(&after, "frame").unwrap() else { panic!() };
    assert!(!g.pass_through);
    assert_eq!(ids(&after, "frame"), ["art", "pin"], "children survive a patch to their group");

    let after = round_trip(set("root", json!({ "name": "Card" })));
    assert_eq!(after.root.base.name, "Card");
    assert_eq!(ids(&after, "root"), ["bg", "frame", "text"]);
}

#[test]
fn set_layer_props_refuses_bad_input() {
    fails(set("art", json!({ "id": "x" })), "cannot be changed");
    fails(set("frame", json!({ "children": [] })), "cannot be changed");
    fails(set("nope", json!({ "opacity": 0 })), "No layer");
    fails(set("art", json!({ "blend": "dissolve" })), "Invalid layer properties");
    fails(set("art", json!({ "opacity": "half" })), "Invalid layer properties");
    fails(set("art", json!({ "opacty": 0.5 })), "has no property \"opacty\"");
    fails(set("bg", json!({ "transform": [1, 0, 0, 0, 1, 0] })), "has no property \"transform\"");
}

#[test]
fn add_layer_inserts_at_the_top_by_default_or_at_an_index() {
    let top = round_trip(Edit::AddLayer { parent: "frame".into(), layer: shape("new", "M0 0", RED), index: None });
    assert_eq!(ids(&top, "frame"), ["art", "pin", "new"]);
    let nested = group("g", true, vec![shape("inner", "M0 0", RED)]);
    let bottom = round_trip(Edit::AddLayer { parent: "root".into(), layer: nested, index: Some(0) });
    assert_eq!(ids(&bottom, "root")[0], "g");
    assert!(find(&bottom, "inner").is_some());
}

#[test]
fn add_layer_rejects_duplicates_and_bad_targets() {
    let add = |parent: &str, layer: Layer, index| Edit::AddLayer { parent: parent.into(), layer, index };
    fails(add("root", shape("art", "", BLACK), None), "already in the document");
    fails(add("root", group("g", true, vec![shape("pin", "", BLACK)]), None), "already");
    fails(add("root", group("g", true, vec![shape("z", "", BLACK), shape("z", "", BLACK)]), None), "already");
    fails(add("art", shape("z", "", BLACK), None), "not a group");
    fails(add("root", shape("z", "", BLACK), Some(9)), "out of range");
    fails(add("missing", shape("z", "", BLACK), None), "No layer");
}

#[test]
fn remove_layer_takes_the_subtree_out_and_puts_it_back() {
    let after = round_trip(Edit::RemoveLayer { id: "frame".into() });
    assert!(find(&after, "art").is_none());
    assert_eq!(ids(&after, "root"), ["bg", "text"]);
    fails(Edit::RemoveLayer { id: "root".into() }, "root group");
    fails(Edit::RemoveLayer { id: "ghost".into() }, "No layer");
}

#[test]
fn move_layer_reorders_and_moves_between_groups() {
    let mv = |id: &str, parent: &str, index| Edit::MoveLayer { id: id.into(), parent: parent.into(), index };
    let reordered = round_trip(mv("bg", "root", 2));
    assert_eq!(ids(&reordered, "root"), ["frame", "text", "bg"]);
    let moved = round_trip(mv("art", "text", 0));
    assert_eq!(ids(&moved, "text"), ["art"]);
    assert_eq!(ids(&moved, "frame"), ["pin"]);
    // Moving out of a group to a later sibling position of that group's parent.
    let out = round_trip(mv("pin", "root", 3));
    assert_eq!(ids(&out, "root"), ["bg", "frame", "text", "pin"]);
    // Moving into an earlier sibling group shifts nothing it depends on.
    let into = round_trip(mv("text", "frame", 1));
    assert_eq!(ids(&into, "frame"), ["art", "text", "pin"]);
}

#[test]
fn move_layer_refuses_cycles_and_bad_indexes() {
    let mv = |id: &str, parent: &str, index| Edit::MoveLayer { id: id.into(), parent: parent.into(), index };
    fails(mv("frame", "frame", 0), "into itself");
    fails(mv("bg", "root", 3), "out of range");
    fails(mv("bg", "art", 0), "not a group");
    let mut doc = sample();
    apply(&mut doc, &Edit::AddLayer { parent: "frame".into(), layer: group("inner", true, vec![]), index: None }).unwrap();
    let before = doc.clone();
    assert!(apply(&mut doc, &mv("frame", "inner", 0)).unwrap_err().0.contains("into itself"));
    assert_eq!(doc, before);
}

#[test]
fn set_asset_and_set_doc_props_round_trip() {
    let asset = |path: &str| Some(AssetRef { kind: AssetKind::Image, path: path.into(), width: None, height: None });
    round_trip(Edit::SetAsset { id: "mask".into(), asset: asset("assets/mask.png") });
    round_trip(Edit::SetAsset { id: "art".into(), asset: asset("assets/other.png") });
    round_trip(Edit::SetAsset { id: "art".into(), asset: None });
    let after = round_trip(Edit::SetDocProps { props: props(json!({ "templateId": "normal", "card": { "name": "Test" }, "dpi": 1200 })) });
    assert_eq!(after.card, Some(json!({ "name": "Test" })));
    assert_eq!((after.template_id.as_deref(), after.dpi), (Some("normal"), 1200.0));
    fails(Edit::SetDocProps { props: props(json!({ "width": 0 })) }, "positive");
    fails(Edit::SetDocProps { props: props(json!({ "dpi": 300, "root": {} })) }, "cannot be changed");
}

#[test]
fn batch_is_one_undo_step_and_rolls_back_on_failure() {
    let after = round_trip(Edit::Batch {
        edits: vec![set("art", json!({ "visible": false })), Edit::RemoveLayer { id: "pin".into() }, set("bg", json!({ "opacity": 0.2 }))],
    });
    assert!(find(&after, "pin").is_none());

    let mut history = History::new(sample());
    let bad = Edit::Batch { edits: vec![set("art", json!({ "visible": false })), Edit::RemoveLayer { id: "missing".into() }] };
    assert!(history.execute(bad, "bad").is_err());
    assert_eq!(history.doc(), &sample());
    assert!(!history.can_undo());
}

#[test]
fn history_tracks_labels_clears_redo_and_caps_its_length() {
    let mut history = History::new(sample());
    assert!(!history.undo());
    history.execute(set("art", json!({ "opacity": 0.1 })), "Fade art").unwrap();
    history.execute(Edit::RemoveLayer { id: "pin".into() }, "Delete layer").unwrap();
    assert_eq!(history.labels(), ["Fade art", "Delete layer"]);
    history.undo();
    assert!(history.can_redo());
    history.execute(set("bg", json!({ "visible": false })), "Hide").unwrap();
    assert!(!history.can_redo());
    assert!(!history.redo());

    let mut capped = History::with_limit(sample(), 2);
    for opacity in [0.1, 0.2, 0.3] {
        capped.execute(set("art", json!({ "opacity": opacity })), "fade").unwrap();
    }
    assert_eq!(capped.labels().len(), 2);
}

#[test]
fn edits_are_plain_json() {
    let edit: Edit = serde_json::from_value(json!({ "type": "moveLayer", "id": "art", "parent": "text", "index": 0 })).unwrap();
    assert_eq!(edit, Edit::MoveLayer { id: "art".into(), parent: "text".into(), index: 0 });
    let text = serde_json::to_string(&set("art", json!({ "opacity": 0.5 }))).unwrap();
    assert_eq!(text, r#"{"type":"setLayerProps","id":"art","props":{"opacity":0.5}}"#);
}

#[test]
fn documents_round_trip_through_json() {
    let doc = sample();
    let json = stringify_doc(&doc).unwrap();
    assert_eq!(parse_doc(&json).unwrap(), doc);
    // The JSON shape the TypeScript model wrote: tagged layers, camelCase keys, optional fields left out.
    let value: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["root"]["kind"], "group");
    assert_eq!(value["root"]["passThrough"], true);
    assert_eq!(value["root"]["children"][0]["fill"], json!({ "type": "solid", "color": [0.0, 0.0, 0.0, 1.0] }));
    assert!(value["root"]["children"][0].get("fillOpacity").is_none());
}

#[test]
fn every_layer_and_effect_type_survives_a_round_trip() {
    let json = json!({
        "version": 1, "width": 10, "height": 10, "dpi": 72,
        "assets": { "a": { "kind": "image", "path": "assets/a.png" } },
        "root": { "kind": "group", "id": "root", "name": "Root", "visible": true, "opacity": 1, "blend": "normal", "clip": false, "passThrough": true, "children": [
            { "kind": "text", "id": "t", "name": "Rules", "visible": true, "opacity": 1, "blend": "softLight", "clip": false,
              "box": { "x": 0, "y": 0, "w": 5, "h": 5 }, "content": { "runs": [{ "text": "Flying", "italic": true }, { "text": "", "symbol": "W" }] },
              "style": { "font": "Plantin", "size": 9, "color": [0, 0, 0, 1], "align": "left", "leading": 1.1 }, "fit": "shrinkHeight" },
            { "kind": "art", "id": "art", "name": "Art", "visible": true, "opacity": 1, "blend": "normal", "clip": false,
              "frameRef": { "x": 1, "y": 1, "w": 8, "h": 6 }, "transform": [1, 0, 0, 0, 1, 0], "fit": "cover" },
            { "kind": "fill", "id": "f", "name": "Fill", "visible": true, "opacity": 1, "blend": "linearLight", "clip": true,
              "source": "patchmatch", "params": { "seed": 3 }, "asset": "a", "transform": [1, 0, 0, 0, 1, 0] },
            { "kind": "shape", "id": "s", "name": "S", "visible": false, "opacity": 0.5, "fillOpacity": 0.25, "blend": "luminosity", "clip": false, "effectsEnabled": false,
              "path": { "d": "M0 0H5V5Z", "fillRule": "evenodd" }, "fill": { "type": "solid", "color": [1, 0, 0, 1] }, "stroke": { "color": [0, 0, 0, 1], "width": 2, "align": "inside" },
              "mask": { "asset": "a", "offset": { "x": 1, "y": 2 }, "outside": "reveal", "inverted": true, "density": 0.5, "feather": 2, "enabled": false },
              "vectorMask": { "path": { "d": "M0 0H5V5Z" }, "inverted": true },
              "locked": { "position": true }, "role": "pinlines.W",
              "effects": [
                { "type": "colorOverlay", "enabled": true, "blend": "multiply", "opacity": 1, "color": [1, 0, 0, 1] },
                { "type": "gradientOverlay", "enabled": true, "blend": "normal", "opacity": 1, "gradient": { "stops": [{ "offset": 0, "color": [0, 0, 0, 1] }] }, "style": "radial", "angle": 90, "scale": 1, "reverse": true },
                { "type": "stroke", "enabled": true, "blend": "normal", "opacity": 1, "color": [0, 0, 0, 1], "size": 3, "position": "center" },
                { "type": "dropShadow", "enabled": false, "blend": "multiply", "opacity": 0.5, "color": [0, 0, 0, 1], "angle": 120, "distance": 4, "spread": 0.1, "size": 5, "contour": "gaussian" },
                { "type": "innerShadow", "enabled": true, "blend": "multiply", "opacity": 0.5, "color": [0, 0, 0, 1], "angle": 120, "distance": 4, "choke": 0, "size": 5 },
                { "type": "outerGlow", "enabled": true, "blend": "screen", "opacity": 0.5, "color": [1, 1, 1, 1], "spread": 0, "size": 5 },
                { "type": "bevel", "enabled": true, "style": "outer", "technique": "chiselHard", "direction": "down", "depth": 1, "size": 4, "soften": 0, "angle": 120, "altitude": 30,
                  "highlight": { "color": [1, 1, 1, 1], "blend": "screen", "opacity": 0.75 }, "shadow": { "color": [0, 0, 0, 1], "blend": "multiply", "opacity": 0.75 } }
              ] }
        ] }
    });
    let doc = parse_doc(&json.to_string()).unwrap();
    let again: Value = serde_json::from_str(&stringify_doc(&doc).unwrap()).unwrap();
    // Numbers come back as floats; compare through a second parse so 1 and 1.0 are equal.
    assert_eq!(parse_doc(&again.to_string()).unwrap(), doc);
    let shape = &again["root"]["children"][3];
    assert_eq!(shape["effects"].as_array().unwrap().len(), 7);
    assert_eq!(shape["effects"][3]["contour"], "gaussian");
    assert_eq!(shape["mask"]["outside"], "reveal");
    assert_eq!(shape["path"]["fillRule"], "evenodd");
    assert_eq!(again["root"]["children"][0]["box"]["w"], 5.0);
    assert_eq!(again["root"]["children"][1]["frameRef"]["h"], 6.0);
}

#[test]
fn malformed_documents_are_rejected_with_a_reason() {
    let bad = |edit: &dyn Fn(&mut Value), expected: &str| {
        let mut value: Value = serde_json::from_str(&stringify_doc(&sample()).unwrap()).unwrap();
        edit(&mut value);
        let error = parse_doc(&value.to_string()).expect_err("should be rejected");
        assert!(error.0.contains(expected), "error was: {error}");
    };
    bad(&|d| d["version"] = json!(2), "unsupported version 2");
    bad(&|d| d["width"] = json!(0), "\"width\"");
    bad(&|d| d["root"]["children"][0]["blend"] = json!("dissolve"), "dissolve");
    bad(&|d| d["root"]["children"][0]["id"] = json!("frame"), "duplicate layer id \"frame\"");
    bad(&|d| d["root"]["children"][0]["opacity"] = json!(1.5), "opacity must be 0..1");
    bad(&|d| d["assets"] = json!({}), "missing asset \"art\"");
    bad(&|d| d["assets"]["art"]["path"] = json!("../secret.png"), "unsafe path");
    bad(&|d| d["root"]["children"][0]["kind"] = json!("smart"), "smart");
    bad(&|d| d["root"]["kind"] = json!("shape"), "Invalid document");
    assert!(parse_doc("{nope").unwrap_err().0.contains("not JSON"));
    assert!(parse_doc("{}").unwrap_err().0.contains("no \"version\""));
}

#[test]
fn cardsmith_files_save_and_load_with_their_assets() {
    let doc = sample();
    let mut assets = AssetBytes::new();
    assets.insert("art".into(), vec![137, 80, 78, 71, 1, 2, 3]);
    let bytes = save_cardsmith(&doc, &assets).unwrap();
    assert!(is_zip(&bytes));
    let (loaded, loaded_assets) = load_cardsmith(&bytes).unwrap();
    assert_eq!(loaded, doc);
    assert_eq!(loaded_assets, assets);
}

#[test]
fn saving_and_loading_refuse_missing_assets_and_non_zips() {
    let doc = sample();
    assert!(save_cardsmith(&doc, &AssetBytes::new()).unwrap_err().0.contains("no data for asset \"art\""));
    assert!(load_cardsmith(&[1, 2, 3]).unwrap_err().0.contains("not a zip"));

    use std::io::Write;
    let zip_of = |name: &str, content: &str| {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
        zip.finish().unwrap().into_inner()
    };
    assert!(load_cardsmith(&zip_of("other.txt", "x")).unwrap_err().0.contains("no doc.json"));
    let only_doc = zip_of("doc.json", &stringify_doc(&doc).unwrap());
    assert!(load_cardsmith(&only_doc).unwrap_err().0.contains("missing asset \"art\""));
}
