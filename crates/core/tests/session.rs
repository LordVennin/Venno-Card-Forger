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

fn frame_px(s: &mut Session, x: u32, y: u32) -> [u8; 4] {
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
    let mut s = Session::new(doc, AssetBytes::new());
    assert!(s.frame() == expected.pixels);
    assert_eq!(s.frame_hash(), cardsmith_core::render::hash(&expected.pixels));
    assert!(!s.can_undo() && !s.can_redo());
}

#[test]
fn edits_redraw_and_undo_restores_the_frame() {
    let mut s = Session::new(scene_doc(), AssetBytes::new());
    let before = s.frame().to_vec();
    s.execute(Edit::SetLayerProps { id: "card".into(), props: json!({ "visible": false }).as_object().unwrap().clone() }, "Hide card").unwrap();
    assert_eq!(frame_px(&mut s, 50, 40), [255, 255, 255, 255]);
    assert_eq!(s.labels(), ["Hide card"]);
    assert!(s.undo());
    assert!(s.frame() == before);
    assert!(s.redo());
    assert_eq!(frame_px(&mut s, 50, 40), [255, 255, 255, 255]);
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

    assert_eq!(frame_px(&mut s, 45, 30), [255, 0, 0, 255]);
    assert_eq!(frame_px(&mut s, 45, 50), [128, 128, 128, 255]); // untouched part of the card
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
    assert_eq!(frame_px(&mut s, 60, 35), [128, 128, 128, 255]); // erased back to the card beneath
    assert_frame_is_exact(&mut s);

    assert!(s.undo());
    assert!(s.frame() == painted);
    assert!(s.undo());
    assert!(s.frame() == clean);
    assert!(s.redo() && s.redo());
    assert_eq!(frame_px(&mut s, 60, 35), [128, 128, 128, 255]);
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
    assert_eq!(frame_px(&mut s, 55, 41), [255, 0, 0, 255]); // red × the yellow tint stays red
    assert_eq!(frame_px(&mut s, 55, 34), [0, 0, 128, 255]); // the group's blue stroke around the paint, tinted
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
    assert_eq!(frame_px(&mut s, 60, 30), [255, 0, 0, 255]);
    assert_eq!(frame_px(&mut s, 79, 30), [255, 0, 0, 255]); // the last column of the 30 px image
    assert_ne!(frame_px(&mut s, 80, 30), [255, 0, 0, 255]); // past it: nothing painted
    assert_eq!(frame_px(&mut s, 75, 45), [0, 255, 0, 255]); // the rest of the image is untouched
    assert_frame_is_exact(&mut s);

    assert!(s.stroke_begin("scaled", red(6.0)).unwrap_err().0.contains("scaled or rotated"));
    assert!(s.stroke_begin("twin-a", red(6.0)).unwrap_err().0.contains("shares its image"));
    assert!(s.stroke_begin("card", red(6.0)).unwrap_err().0.contains("color goes on a paint layer"));
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
    let mut reopened = Session::new(doc, assets);
    assert!(reopened.frame() == s.frame());
    assert_eq!(frame_px(&mut reopened, 60, 40), [255, 0, 0, 255]);
}

#[test]
fn painting_between_layers_is_exact_whether_or_not_the_stack_can_be_split() {
    // The painted layer sits in the middle of the stack, inside nested groups, under a multiply layer.
    let build = |outer_opacity: f64| {
        let mut doc = scene_doc();
        doc.assets.insert("ink".into(), AssetRef { kind: AssetKind::Image, path: "assets/ink.png".into(), width: None, height: None });
        let inner = group("inner", true, vec![rect("under", 30.0, 20.0, 40.0, 30.0, [0.0, 0.6, 0.0, 1.0]), pixel("ink", "ink", IDENTITY), rect("over", 50.0, 25.0, 30.0, 30.0, [0.0, 0.0, 1.0, 0.5])]);
        doc.root.children.push(patch(group("outer", true, vec![inner]), json!({ "opacity": outer_opacity })));
        doc.root.children.push(patch(rect("tint", 0.0, 0.0, 120.0, 80.0, [1.0, 0.8, 0.8, 1.0]), json!({ "blend": "multiply" })));
        let mut assets = AssetBytes::new();
        assets.insert("ink".into(), png(120, 80, |_, _| [0, 0, 0, 0]));
        Session::new(doc, assets)
    };
    // At full opacity the groups are plain containers and the stack is split at the layer.
    // At 50% the outer group is composited as a unit, so the whole stack is redrawn.
    for opacity in [1.0, 0.5] {
        let mut s = build(opacity);
        for round in 0..2 {
            s.stroke_begin("ink", Brush { erase: round == 1, ..red(14.0) }).unwrap();
            for (x, y) in [(20.0, 20.0), (70.0, 45.0), (110.0, 30.0 + round as f32 * 20.0)] {
                s.stroke_to(x, y, 1.0);
            }
            s.stroke_end("Stroke");
            assert_frame_is_exact(&mut s);
        }
        // An edit beneath the painted layer, then more painting: the cached layers beneath must not be reused.
        s.execute(Edit::SetLayerProps { id: "under".into(), props: json!({ "visible": false }).as_object().unwrap().clone() }, "Hide").unwrap();
        s.stroke_begin("ink", red(10.0)).unwrap();
        s.stroke_to(45.0, 35.0, 1.0);
        s.stroke_end("Stroke");
        assert_frame_is_exact(&mut s);
        s.undo();
        s.undo();
        assert_frame_is_exact(&mut s);
    }
}

#[test]
fn an_asset_that_arrives_late_is_drawn_and_hidden_layers_ask_for_nothing() {
    let png = |rgba: [u8; 4]| cardsmith_core::render::encode_png(4, 4, &rgba.repeat(16)).unwrap();
    let mut doc = scene_doc();
    for (id, path) in [("red", "assets/red.png"), ("blue", "assets/blue.png")] {
        doc.assets.insert(id.into(), serde_json::from_value(json!({ "kind": "image", "path": path, "width": 4, "height": 4 })).unwrap());
    }
    doc.root.children.push(pixel("shown", "red", [1.0, 0.0, 2.0, 0.0, 1.0, 2.0]));
    doc.root.children.push(patch(pixel("hidden", "blue", [1.0, 0.0, 10.0, 0.0, 1.0, 2.0]), json!({ "visible": false })));
    let mut s = Session::new(doc, AssetBytes::new());

    // Only what shows is asked for, and without it the layer is simply not drawn.
    assert_eq!(s.missing_assets(), vec![("red".to_string(), "assets/red.png".to_string())]);
    assert_eq!(frame_px(&mut s, 3, 3), [255, 255, 255, 255]);

    s.add_asset("red", png([255, 0, 0, 255])).unwrap();
    assert_eq!(frame_px(&mut s, 3, 3), [255, 0, 0, 255]);
    assert!(s.missing_assets().is_empty());

    // Showing the other layer makes its image wanted.
    s.execute(Edit::SetLayerProps { id: "hidden".into(), props: json!({ "visible": true }).as_object().unwrap().clone() }, "Show").unwrap();
    assert_eq!(s.missing_assets(), vec![("blue".to_string(), "assets/blue.png".to_string())]);
    s.add_asset("blue", png([0, 0, 255, 255])).unwrap();
    assert_eq!(frame_px(&mut s, 11, 3), [0, 0, 255, 255]);
    assert_frame_is_exact(&mut s);

    // Several images supplied together are all drawn when the last one arrives.
    s.stash_asset("red", png([0, 255, 0, 255]));
    assert_eq!(frame_px(&mut s, 3, 3), [255, 0, 0, 255]);
    s.add_asset("blue", png([255, 255, 0, 255])).unwrap();
    assert_eq!((frame_px(&mut s, 3, 3), frame_px(&mut s, 11, 3)), ([0, 255, 0, 255], [255, 255, 0, 255]));
    assert_frame_is_exact(&mut s);
}

#[test]
fn a_batch_redraws_only_what_it_changed_in_the_end() {
    let mut doc = scene_doc();
    doc.root.children.push(rect("far", 100.0, 60.0, 10.0, 10.0, GRAY));
    let mut s = Session::new(doc, AssetBytes::new());
    s.set_view(View { scale: 1.0, x: 0.0, y: 0.0, w: 120, h: 80 });
    let props = |v: serde_json::Value| v.as_object().unwrap().clone();
    // The card is hidden and shown again, the far square really is hidden, and a note is made
    // on the document: only the far square's corner is drawn again.
    let batch = Edit::Batch { edits: vec![
        Edit::SetLayerProps { id: "card".into(), props: props(json!({ "visible": false })) },
        Edit::SetLayerProps { id: "card".into(), props: props(json!({ "visible": true })) },
        Edit::SetLayerProps { id: "far".into(), props: props(json!({ "visible": false })) },
        Edit::SetDocProps { props: props(json!({ "card": { "name": "x" } })) },
    ] };
    s.execute(batch, "Batch").unwrap();
    let dirty = s.dirty_view();
    assert!(dirty.x >= 95 && dirty.y >= 55 && dirty.area() > 0 && dirty.area() < 400, "{dirty:?}");
    assert_frame_is_exact(&mut s);
    // Swapping two layers that overlap redraws them; the result is still exact.
    s.execute(Edit::MoveLayer { id: "bg".into(), parent: s.doc().root.base.id.clone(), index: 1 }, "Move").unwrap();
    assert_frame_is_exact(&mut s);
    s.execute(Edit::Batch { edits: vec![Edit::MoveLayer { id: "bg".into(), parent: s.doc().root.base.id.clone(), index: 0 }] }, "Move back").unwrap();
    assert_eq!(frame_px(&mut s, 30, 30), [128, 128, 128, 255]);
    assert_frame_is_exact(&mut s);
}

fn view_px(s: &Session, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * s.view().w + x) * 4) as usize;
    s.view_pixels()[i..i + 4].try_into().unwrap()
}

#[test]
fn zoomed_out_the_screen_is_drawn_from_a_smaller_copy_and_the_full_picture_catches_up() {
    // A 480 x 320 scene seen at a quarter of its size: the copy is 120 x 80.
    let mut doc = Doc::empty(480, 320, 72.0);
    doc.root.children = vec![rect("bg", 0.0, 0.0, 480.0, 320.0, WHITE), rect("card", 80.0, 60.0, 240.0, 180.0, GRAY), rect("far", 400.0, 240.0, 40.0, 40.0, GRAY)];
    let mut s = Session::new(doc, AssetBytes::new());
    s.set_view(View { scale: 0.25, x: 0.0, y: 0.0, w: 120, h: 80 });
    assert_eq!((view_px(&s, 50, 40), view_px(&s, 5, 5)), ([128, 128, 128, 255], [255, 255, 255, 255]));

    // An edit shows on screen at once...
    let props = |v: serde_json::Value| v.as_object().unwrap().clone();
    s.execute(Edit::SetLayerProps { id: "card".into(), props: props(json!({ "visible": false })) }, "Hide").unwrap();
    assert_eq!(view_px(&s, 50, 40), [255, 255, 255, 255]);
    let dirty = s.dirty_view();
    assert!(dirty.area() > 0 && dirty.x + dirty.w <= 90, "only the card's part of the screen changed: {dirty:?}");
    // ...and the full-size picture is exactly right when it is asked for.
    assert_frame_is_exact(&mut s);
    assert_eq!(frame_px(&mut s, 200, 150), [255, 255, 255, 255]);

    // Undo, a brush stroke and its undo all show, and all leave the full picture exact.
    assert!(s.undo());
    assert_eq!(view_px(&s, 50, 40), [128, 128, 128, 255]);
    let paint = s.add_paint_layer("Paint").unwrap();
    s.stroke_begin(&paint, red(40.0)).unwrap();
    s.stroke_to(100.0, 100.0, 1.0);
    s.stroke_to(300.0, 100.0, 1.0);
    s.stroke_end("Brush");
    assert_eq!(view_px(&s, 50, 25), [255, 0, 0, 255]);
    assert_frame_is_exact(&mut s);
    s.execute(Edit::SetLayerProps { id: "far".into(), props: props(json!({ "visible": false })) }, "Hide").unwrap();
    assert_eq!((view_px(&s, 50, 25), view_px(&s, 105, 65)), ([255, 0, 0, 255], [255, 255, 255, 255]));
    assert!(s.undo() && s.undo());
    assert_eq!((view_px(&s, 50, 25), view_px(&s, 105, 65)), ([128, 128, 128, 255], [128, 128, 128, 255]));
    assert_frame_is_exact(&mut s);

    // Zooming in past half size goes back to the real pixels.
    s.set_view(View { scale: 1.0, x: 80.0, y: 60.0, w: 100, h: 100 });
    assert_eq!(view_px(&s, 50, 50), [128, 128, 128, 255]);
}

#[test]
fn zoomed_in_only_what_is_on_screen_is_drawn_until_it_is_scrolled_to_or_wanted() {
    let mut doc = Doc::empty(480, 320, 72.0);
    doc.root.children = vec![rect("bg", 0.0, 0.0, 480.0, 320.0, WHITE), rect("near", 10.0, 10.0, 40.0, 40.0, GRAY), rect("far", 400.0, 240.0, 40.0, 40.0, GRAY)];
    let mut s = Session::new(doc, AssetBytes::new());
    s.set_view(View { scale: 1.0, x: 0.0, y: 0.0, w: 100, h: 100 });
    let props = |v: serde_json::Value| v.as_object().unwrap().clone();
    // Both squares are hidden; only the near one is on screen.
    s.execute(Edit::Batch { edits: vec![
        Edit::SetLayerProps { id: "near".into(), props: props(json!({ "visible": false })) },
        Edit::SetLayerProps { id: "far".into(), props: props(json!({ "visible": false })) },
    ] }, "Hide both").unwrap();
    assert_eq!(view_px(&s, 30, 30), [255, 255, 255, 255]);
    // Scrolling to the far one shows it gone too.
    s.set_view(View { scale: 1.0, x: 380.0, y: 220.0, w: 100, h: 100 });
    assert_eq!(view_px(&s, 40, 40), [255, 255, 255, 255]);
    assert_frame_is_exact(&mut s);
    // And without scrolling, the full picture is still right when asked for.
    s.set_view(View { scale: 1.0, x: 0.0, y: 0.0, w: 100, h: 100 });
    assert!(s.undo());
    assert_eq!((view_px(&s, 30, 30), frame_px(&mut s, 420, 260)), ([128, 128, 128, 255], [128, 128, 128, 255]));
    assert_frame_is_exact(&mut s);
}

#[test]
fn a_document_saves_with_only_the_images_it_has_and_opens_again_without_the_rest() {
    use cardsmith_core::serialize::load_cardsmith_available;
    let png = |rgba: [u8; 4]| cardsmith_core::render::encode_png(4, 4, &rgba.repeat(16)).unwrap();
    let mut doc = scene_doc();
    for (id, path) in [("red", "assets/red.png"), ("blue", "assets/blue.png")] {
        doc.assets.insert(id.into(), serde_json::from_value(json!({ "kind": "image", "path": path, "width": 4, "height": 4 })).unwrap());
    }
    doc.root.children.push(pixel("shown", "red", [1.0, 0.0, 2.0, 0.0, 1.0, 2.0]));
    doc.root.children.push(patch(pixel("hidden", "blue", [1.0, 0.0, 10.0, 0.0, 1.0, 2.0]), json!({ "visible": false })));
    let mut s = Session::new(doc, AssetBytes::new());
    s.add_asset("red", png([255, 0, 0, 255])).unwrap();
    // The whole document cannot be saved, the hidden layer's image never having been loaded; what is there can.
    assert!(s.save().is_err());
    let file = s.save_available().unwrap();
    assert!(load_cardsmith(&file).is_err());
    let (doc, assets) = load_cardsmith_available(&file).unwrap();
    assert_eq!(assets.keys().collect::<Vec<_>>(), ["red"]);
    let mut reopened = Session::new(doc, assets);
    assert!(reopened.frame() == s.frame());
    // The hidden layer is still there and asks for its image when shown.
    reopened.execute(Edit::SetLayerProps { id: "hidden".into(), props: json!({ "visible": true }).as_object().unwrap().clone() }, "Show").unwrap();
    assert_eq!(reopened.missing_assets(), vec![("blue".to_string(), "assets/blue.png".to_string())]);
}

#[test]
fn a_host_that_scales_the_frame_itself_is_told_only_what_changed_and_moving_the_view_costs_nothing() {
    let mut doc = Doc::empty(480, 320, 72.0);
    doc.root.children = vec![rect("bg", 0.0, 0.0, 480.0, 320.0, WHITE), rect("card", 80.0, 60.0, 240.0, 180.0, GRAY), rect("far", 400.0, 240.0, 40.0, 40.0, GRAY)];
    let mut s = Session::new(doc, AssetBytes::new());
    s.set_direct(true);
    s.set_view(View { scale: 0.25, x: 0.0, y: 0.0, w: 120, h: 80 });
    // A quarter-size frame, all of it new.
    let (pixels, w, h, per) = s.display();
    assert_eq!((w, h, per, pixels.len()), (120, 80, 4, 120 * 80 * 4));
    let at = |s: &Session, x: usize, y: usize| -> [u8; 4] { let (p, w, _, _) = s.display(); p[(y * w as usize + x) * 4..(y * w as usize + x) * 4 + 4].try_into().unwrap() };
    assert_eq!((at(&s, 50, 40), at(&s, 5, 5)), ([128, 128, 128, 255], [255, 255, 255, 255]));
    assert_eq!(s.take_display_dirty(), Some(Region::new(0, 0, 120, 80)));
    assert_eq!(s.take_display_dirty(), None);

    // Panning and zooming within the same copy: nothing to draw, nothing to send.
    s.set_view(View { scale: 0.25, x: 100.0, y: 50.0, w: 120, h: 80 });
    s.set_view(View { scale: 0.2, x: -40.0, y: 0.0, w: 120, h: 80 });
    assert_eq!(s.take_display_dirty(), None);

    // An edit: only its part of the frame, in the frame's own pixels.
    let props = |v: serde_json::Value| v.as_object().unwrap().clone();
    s.execute(Edit::SetLayerProps { id: "far".into(), props: props(json!({ "visible": false })) }, "Hide").unwrap();
    let dirty = s.take_display_dirty().unwrap();
    assert!(dirty.x >= 98 && dirty.y >= 58 && dirty.x + dirty.w <= 112 && dirty.y + dirty.h <= 72, "{dirty:?}");
    assert_eq!(at(&s, 105, 65), [255, 255, 255, 255]);

    // Zoomed in to full size: the real frame is on display, new to the host, and right where it is on screen.
    s.set_view(View { scale: 1.0, x: 0.0, y: 0.0, w: 120, h: 80 });
    let (_, w, h, per) = s.display();
    assert_eq!((w, h, per), (480, 320, 1));
    assert_eq!(s.take_display_dirty(), Some(Region::new(0, 0, 480, 320)));
    assert_eq!(at(&s, 100, 70), [128, 128, 128, 255]);
    // An edit off screen draws nothing now; scrolling there draws it and says so.
    assert!(s.undo());
    assert_eq!(s.take_display_dirty(), None);
    s.set_view(View { scale: 1.0, x: 360.0, y: 240.0, w: 120, h: 80 });
    assert!(s.take_display_dirty().is_some());
    assert_eq!(at(&s, 420, 260), [128, 128, 128, 255]);
    // Back out: the smaller copy was kept and only caught up, and it shows the square again.
    s.set_view(View { scale: 0.25, x: 0.0, y: 0.0, w: 120, h: 80 });
    assert_eq!(at(&s, 105, 65), [128, 128, 128, 255]);
    assert_frame_is_exact(&mut s);
}
