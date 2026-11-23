//! The session: edits, brush strokes, undo, incremental redraw, the view, saving.

mod common;
use cardsmith_core::commands::Edit;
use cardsmith_core::doc::build::{group, pixel};
use cardsmith_core::doc::*;
use cardsmith_core::paint::Brush;
use cardsmith_core::raster::Region;
use cardsmith_core::render::render;
use cardsmith_core::serialize::{load_cardsmith, AssetBytes};
use cardsmith_core::session::{Session, View};
use common::*;
use serde_json::json;

fn red(size: f32) -> Brush {
    Brush { size, hardness: 1.0, color: [255, 0, 0], ..Brush::default() }
}

/// White background, a gray square with a drop shadow, then whatever the test adds.
fn scene_doc() -> Doc {
    let mut doc = Doc::empty(120, 80, 72.0);
    let shadow = json!([{ "type": "dropShadow", "enabled": true, "blend": "multiply", "opacity": 0.6, "color": [0, 0, 0, 1], "angle": 120, "distance": 4, "spread": 0, "size": 6 }]);
    doc.root.children = vec![rect("bg", 0.0, 0.0, 120.0, 80.0, WHITE), patch(rect("card", 20.0, 15.0, 60.0, 45.0, GRAY), json!({ "effects": shadow }))];
    doc
}

fn frame_px(s: &Session, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * s.doc().width + x) * 4) as usize;
    s.frame()[i..i + 4].try_into().unwrap()
}

/// The frame a session keeps up to date incrementally must equal a fresh full render.
#[track_caller]
fn assert_frame_is_exact(s: &mut Session) {
    let bytes = s.save().unwrap();
    let (doc, assets) = load_cardsmith(&bytes).unwrap();
    let full = render(&doc, &assets).pixels;
    let w = doc.width as usize;
    let differing: Vec<(usize, usize)> = full.chunks_exact(4).zip(s.frame().chunks_exact(4)).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| (i % w, i / w)).collect();
    assert!(differing.is_empty(), "the incrementally updated frame differs from a full render at {} pixels, e.g. {:?}", differing.len(), &differing[..differing.len().min(8)]);
}

#[test]
fn a_new_session_holds_a_full_render() {
    let doc = scene_doc();
    let expected = render(&doc, &AssetBytes::new());
    let s = Session::new(doc, AssetBytes::new());
    assert!(s.frame() == expected.pixels);
    assert_eq!(s.frame_hash(), cardsmith_core::render::hash(&expected.pixels));
    assert!(!s.can_undo() && !s.can_redo());
}

#[test]
fn edits_redraw_and_undo_restores_the_frame() {
    let mut s = Session::new(scene_doc(), AssetBytes::new());
    let before = s.frame().to_vec();
    s.execute(Edit::SetLayerProps { id: "card".into(), props: json!({ "visible": false }).as_object().unwrap().clone() }, "Hide card").unwrap();
    assert_eq!(frame_px(&s, 50, 40), [255, 255, 255, 255]);
    assert_eq!(s.labels(), ["Hide card"]);
    assert!(s.undo());
    assert!(s.frame() == before);
    assert!(s.redo());
    assert_eq!(frame_px(&s, 50, 40), [255, 255, 255, 255]);
    // A refused edit changes nothing and is not recorded.
    assert!(s.execute(Edit::RemoveLayer { id: "ghost".into() }, "x").is_err());
    assert_eq!(s.labels(), ["Hide card"]);
}

#[test]
fn a_stroke_paints_on_the_layer_and_redraws_only_what_it_touched() {
    let mut s = Session::new(scene_doc(), AssetBytes::new());
    s.set_view(View { scale: 1.0, x: 0.0, y: 0.0, w: 120, h: 80 });
    let layer = s.add_paint_layer("Paint").unwrap();
    assert_eq!(layer, "paint-1");
    s.stroke_begin(&layer, red(10.0)).unwrap();
    s.stroke_to(30.5, 30.5, 1.0);
    let first = s.dirty_view();
    assert!(first.w > 0 && first.w <= 16 && first.x >= 22, "{first:?}");
    s.stroke_to(60.5, 30.5, 1.0);
    let second = s.dirty_view();
    assert!(second.x + second.w >= 65 && second.h <= 16, "{second:?}");
    s.stroke_end("Brush");

    assert_eq!(frame_px(&s, 45, 30), [255, 0, 0, 255]);
    assert_eq!(frame_px(&s, 45, 50), [128, 128, 128, 255]); // untouched part of the card
    assert_eq!(s.labels(), ["New paint layer", "Brush"]);
    assert_frame_is_exact(&mut s);
}

#[test]
fn strokes_undo_and_redo_exactly() {
    let mut s = Session::new(scene_doc(), AssetBytes::new());
    let layer = s.add_paint_layer("Paint").unwrap();
    let clean = s.frame().to_vec();
    s.stroke_begin(&layer, red(12.0)).unwrap();
    for (x, y) in [(10.0, 10.0), (100.0, 60.0), (20.0, 70.0)] {
        s.stroke_to(x, y, 1.0);
    }
    s.stroke_end("Brush");
    let painted = s.frame().to_vec();
    assert!(painted != clean);

    s.stroke_begin(&layer, Brush { erase: true, ..red(30.0) }).unwrap();
    s.stroke_to(60.0, 35.0, 1.0);
    s.stroke_end("Eraser");
    assert_eq!(frame_px(&s, 60, 35), [128, 128, 128, 255]); // erased back to the card beneath
    assert_frame_is_exact(&mut s);

    assert!(s.undo());
    assert!(s.frame() == painted);
    assert!(s.undo());
    assert!(s.frame() == clean);
    assert!(s.redo() && s.redo());
    assert_eq!(frame_px(&s, 60, 35), [128, 128, 128, 255]);
    assert!(s.undo() && s.undo() && s.undo()); // also removes the paint layer
    assert!(s.doc().root.children.len() == 2 && !s.can_undo());
}

#[test]
fn painting_under_and_inside_effects_redraws_their_reach() {
    // The paint layer sits in a group with a wide outer stroke, below a multiply layer.
    let mut doc = scene_doc();
    let stroke = json!([{ "type": "stroke", "enabled": true, "blend": "normal", "opacity": 1, "color": [0, 0, 1, 1], "size": 5, "position": "outside" }]);
    doc.assets.insert("ink".into(), AssetRef { kind: AssetKind::Image, path: "assets/ink.png".into(), width: None, height: None });
    doc.root.children.push(patch(group("g", false, vec![pixel("ink", "ink", IDENTITY)]), json!({ "effects": stroke })));
    doc.root.children.push(patch(rect("tint", 0.0, 0.0, 120.0, 80.0, [1.0, 1.0, 0.5, 1.0]), json!({ "blend": "multiply" })));
    let mut assets = AssetBytes::new();
    assets.insert("ink".into(), png(120, 80, |_, _| [0, 0, 0, 0]));

    let mut s = Session::new(doc, assets);
    s.stroke_begin("ink", red(8.0)).unwrap();
    s.stroke_to(40.0, 40.0, 1.0);
    s.stroke_to(70.0, 42.0, 1.0);
    s.stroke_end("Brush");
    assert_eq!(frame_px(&s, 55, 41), [255, 0, 0, 255]); // red × the yellow tint stays red
    assert_eq!(frame_px(&s, 55, 34), [0, 0, 128, 255]); // the group's blue stroke around the paint, tinted
    assert_frame_is_exact(&mut s);
    s.undo();
    assert_frame_is_exact(&mut s);
}

#[test]
fn painting_respects_layer_offsets_and_refuses_unsuitable_layers() {
    let mut doc = scene_doc();
    doc.assets.insert("small".into(), AssetRef { kind: AssetKind::Image, path: "assets/small.png".into(), width: None, height: None });
    doc.root.children.push(pixel("small", "small", [1.0, 0.0, 50.0, 0.0, 1.0, 20.0]));
    for id in ["tile", "twin"] {
        doc.assets.insert(id.into(), AssetRef { kind: AssetKind::Image, path: format!("assets/{id}.png"), width: None, height: None });
    }
    doc.root.children.push(pixel("scaled", "tile", [2.0, 0.0, 90.0, 0.0, 2.0, 60.0]));
    doc.root.children.push(pixel("twin-a", "twin", [1.0, 0.0, 0.0, 0.0, 1.0, 70.0]));
    doc.root.children.push(pixel("twin-b", "twin", [1.0, 0.0, 10.0, 0.0, 1.0, 70.0]));
    let mut assets = AssetBytes::new();
    assets.insert("small".into(), png(30, 30, |_, _| [0, 255, 0, 255]));
    assets.insert("tile".into(), png(4, 4, |_, _| [0, 0, 255, 255]));
    assets.insert("twin".into(), png(4, 4, |_, _| [0, 0, 255, 255]));
    let mut s = Session::new(doc, assets);

    s.stroke_begin("small", red(6.0)).unwrap();
    assert!(s.stroke_begin("small", red(6.0)).unwrap_err().0.contains("already in progress"));
    s.stroke_to(60.0, 30.0, 1.0); // document coordinates: image pixel (10,10)
    s.stroke_to(200.0, 30.0, 1.0); // runs off the image and the document
    s.stroke_end("Brush");
    assert_eq!(frame_px(&s, 60, 30), [255, 0, 0, 255]);
    assert_eq!(frame_px(&s, 79, 30), [255, 0, 0, 255]); // the last column of the 30 px image
    assert_ne!(frame_px(&s, 80, 30), [255, 0, 0, 255]); // past it: nothing painted
    assert_eq!(frame_px(&s, 75, 45), [0, 255, 0, 255]); // the rest of the image is untouched
    assert_frame_is_exact(&mut s);

    assert!(s.stroke_begin("scaled", red(6.0)).unwrap_err().0.contains("scaled or rotated"));
    assert!(s.stroke_begin("twin-a", red(6.0)).unwrap_err().0.contains("shares its image"));
    assert!(s.stroke_begin("card", red(6.0)).unwrap_err().0.contains("Only pixel layers"));
    assert!(s.stroke_begin("nope", red(6.0)).unwrap_err().0.contains("No layer"));
}

#[test]
fn the_view_scales_pans_and_reports_dirty_areas_in_view_space() {
    let mut s = Session::new(scene_doc(), AssetBytes::new());
    let view_px = |s: &Session, x: u32, y: u32| -> [u8; 4] {
        let i = ((y * s.view().w + x) * 4) as usize;
        s.view_pixels()[i..i + 4].try_into().unwrap()
    };

    // Half size: document (50,40) is the card; it lands at view (25,20).
    s.set_view(View { scale: 0.5, x: 0.0, y: 0.0, w: 60, h: 40 });
    assert_eq!(s.dirty_view(), Region::new(0, 0, 60, 40));
    assert_eq!(view_px(&s, 25, 20), [128, 128, 128, 255]);
    assert_eq!(view_px(&s, 2, 2), [255, 255, 255, 255]);

    // Zoomed in ×2 and panned so document (40,30) is at the top-left; beyond the document is transparent.
    s.set_view(View { scale: 2.0, x: 40.0, y: 30.0, w: 200, h: 120 });
    assert_eq!(view_px(&s, 0, 0), [128, 128, 128, 255]);
    assert_eq!(view_px(&s, 199, 0)[3], 0); // document x = 139.5, outside the 120 px width
    assert_eq!(view_px(&s, 150, 119)[3], 0); // document y = 89.5, outside the 80 px height

    // A dab at document (60,40) with a 10 px brush covers view x 30..50, y 10..30 at ×2.
    let layer = s.add_paint_layer("Paint").unwrap();
    s.stroke_begin(&layer, red(10.0)).unwrap();
    s.stroke_to(60.0, 40.0, 1.0);
    let d = s.dirty_view();
    assert!(d.x >= 24 && d.x + d.w <= 56 && d.y >= 4 && d.y + d.h <= 36, "{d:?}");
    assert_eq!(view_px(&s, 40, 20), [255, 0, 0, 255]);
    s.stroke_end("Brush");
}

#[test]
fn saving_keeps_painted_pixels() {
    let mut s = Session::new(scene_doc(), AssetBytes::new());
    let layer = s.add_paint_layer("Paint").unwrap();
    s.stroke_begin(&layer, red(20.0)).unwrap();
    s.stroke_to(60.0, 40.0, 1.0);
    s.stroke_end("Brush");
    let (doc, assets) = load_cardsmith(&s.save().unwrap()).unwrap();
    assert!(doc.assets.contains_key("paint-1"));
    let reopened = Session::new(doc, assets);
    assert!(reopened.frame() == s.frame());
    assert_eq!(frame_px(&reopened, 60, 40), [255, 0, 0, 255]);
}
