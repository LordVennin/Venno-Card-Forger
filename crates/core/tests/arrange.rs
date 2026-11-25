//! Moving and scaling layers, dragging as a preview, and picking a layer by clicking.

mod common;
use cardsmith_core::arrange::{place_edit, Placement};
use cardsmith_core::commands::apply;
use cardsmith_core::doc::build::{group, pixel};
use cardsmith_core::doc::*;
use cardsmith_core::raster::Region;
use cardsmith_core::render::encode_png;
use cardsmith_core::serialize::AssetBytes;
use cardsmith_core::session::{Session, View};
use cardsmith_core::tree;
use common::*;
use serde_json::json;

const FONT: &[u8] = include_bytes!("fixtures/blocks.ttf");

/// White background; a group holding a gray square and a small image; a line of text; a masked shape.
fn scene() -> (Doc, AssetBytes) {
    let mut doc = Doc::empty(200, 120, 72.0);
    doc.assets.insert("dot".into(), serde_json::from_value(json!({ "kind": "image", "path": "assets/dot.png", "width": 10, "height": 10 })).unwrap());
    let text: Layer = serde_json::from_value(json!({
        "kind": "text", "id": "words", "name": "words", "visible": true, "opacity": 1, "blend": "normal", "clip": false,
        "box": { "x": 20, "y": 80, "w": 100, "h": 30 }, "content": { "runs": [{ "text": "ab" }] },
        "style": { "font": "Blocks-Regular", "size": 20, "color": [0, 0, 0, 1], "align": "left" }, "fit": "none"
    }))
    .unwrap();
    let masked = patch(rect("masked", 150.0, 10.0, 40.0, 40.0, GRAY), json!({ "vectorMask": { "path": { "d": "M150 10L170 10L170 50L150 50Z" } } }));
    doc.root.children = vec![rect("bg", 0.0, 0.0, 200.0, 120.0, WHITE), group("pair", true, vec![rect("square", 20.0, 20.0, 40.0, 40.0, GRAY), pixel("dot", "dot", [1.0, 0.0, 70.0, 0.0, 1.0, 30.0])]), text, masked];
    let mut assets = AssetBytes::new();
    assets.insert("dot".into(), encode_png(10, 10, &[255, 0, 0, 255].repeat(100)).unwrap());
    (doc, assets)
}

fn place(doc: &mut Doc, id: &str, to: &Placement) {
    let edit = place_edit(doc, id, to).unwrap();
    apply(doc, &edit).unwrap();
}

fn session() -> Session {
    let (doc, assets) = scene();
    let mut s = Session::new(doc, assets);
    s.add_font(FONT.to_vec()).unwrap();
    s
}

fn px(s: &mut Session, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * s.doc().width + x) * 4) as usize;
    s.frame()[i..i + 4].try_into().unwrap()
}

#[test]
fn every_kind_of_layer_moves_with_its_masks_and_a_group_takes_its_children() {
    let (mut doc, _) = scene();
    place(&mut doc, "pair", &Placement::shift(5.0, -3.0));
    let Layer::Shape(square) = tree::find(&doc, "square").unwrap() else { panic!() };
    assert_eq!(square.path.d, "M25,17 L65,17 L65,57 L25,57 Z");
    let Layer::Pixel(dot) = tree::find(&doc, "dot").unwrap() else { panic!() };
    assert_eq!(dot.transform, [1.0, 0.0, 75.0, 0.0, 1.0, 27.0]);

    place(&mut doc, "words", &Placement::shift(10.0, 10.0));
    let Layer::Text(words) = tree::find(&doc, "words").unwrap() else { panic!() };
    assert_eq!((words.bounds.x, words.bounds.y, words.bounds.w, words.style.size), (30.0, 90.0, 100.0, 20.0));

    place(&mut doc, "masked", &Placement::shift(-100.0, 0.0));
    let Layer::Shape(masked) = tree::find(&doc, "masked").unwrap() else { panic!() };
    assert_eq!((masked.path.d.as_str(), masked.base.vector_mask.as_ref().unwrap().path.d.as_str()), ("M50,10 L90,10 L90,50 L50,50 Z", "M50,10 L70,10 L70,50 L50,50 Z"));
}

#[test]
fn scaling_about_a_point_keeps_that_point_and_scales_text_with_its_box() {
    let (mut doc, _) = scene();
    // Twice the size about the square's top-left corner.
    place(&mut doc, "pair", &Placement::scale_about(2.0, 20.0, 20.0));
    let Layer::Shape(square) = tree::find(&doc, "square").unwrap() else { panic!() };
    assert_eq!(square.path.d, "M20,20 L100,20 L100,100 L20,100 Z");
    let Layer::Pixel(dot) = tree::find(&doc, "dot").unwrap() else { panic!() };
    assert_eq!(dot.transform, [2.0, 0.0, 120.0, 0.0, 2.0, 40.0]);
    place(&mut doc, "words", &Placement::scale_about(0.5, 20.0, 80.0));
    let Layer::Text(words) = tree::find(&doc, "words").unwrap() else { panic!() };
    assert_eq!((words.bounds.x, words.bounds.y, words.bounds.w, words.bounds.h, words.style.size), (20.0, 80.0, 50.0, 15.0, 10.0));
    assert!(place_edit(&doc, "nothing", &Placement::shift(1.0, 1.0)).is_err());
    assert!(place_edit(&doc, "pair", &Placement::scale_about(0.0, 0.0, 0.0)).is_err());
}

#[test]
fn a_drag_shows_each_position_and_becomes_one_step_or_none() {
    let mut s = session();
    s.set_view(View { scale: 1.0, x: 0.0, y: 0.0, w: 200, h: 120 });
    let before = s.doc().clone();
    assert_eq!(px(&mut s, 30, 30), [128, 128, 128, 255]);
    // Three positions of one drag, each given from where the drag began.
    for dx in [10.0, 60.0, 100.0] {
        let edit = place_edit(&before, "pair", &Placement::shift(dx, 0.0)).unwrap();
        s.preview(edit).unwrap();
    }
    assert_eq!((px(&mut s, 30, 30), px(&mut s, 130, 30), px(&mut s, 175, 35)), ([255, 255, 255, 255], [128, 128, 128, 255], [255, 0, 0, 255]));
    assert!(!s.can_undo());
    assert!(s.commit_preview("Move pair"));
    assert_eq!(s.labels(), ["Move pair"]);
    // One step back is where the drag began, exactly.
    assert!(s.undo());
    assert!(*s.doc() == before);
    assert_eq!((px(&mut s, 30, 30), px(&mut s, 130, 30)), ([128, 128, 128, 255], [255, 255, 255, 255]));

    // A drag that is given up leaves nothing behind, and so does one interrupted by another edit.
    s.preview(place_edit(&before, "pair", &Placement::shift(50.0, 20.0)).unwrap()).unwrap();
    s.cancel_preview();
    assert!(*s.doc() == before && !s.commit_preview("Nothing"));
    s.preview(place_edit(&before, "words", &Placement::shift(50.0, 0.0)).unwrap()).unwrap();
    s.execute(place_edit(&before, "masked", &Placement::shift(0.0, 5.0)).unwrap(), "Move masked").unwrap();
    let Layer::Text(words) = tree::find(s.doc(), "words").unwrap() else { panic!() };
    assert_eq!(words.bounds.x, 20.0);
    assert_eq!(s.labels(), ["Move pair", "Move masked"].iter().skip(1).copied().collect::<Vec<_>>());
}

#[test]
fn clicking_picks_the_topmost_layer_with_ink_there_and_its_box_is_known() {
    let mut s = session();
    assert_eq!(s.hit(30.0, 30.0).as_deref(), Some("square"));
    assert_eq!(s.hit(75.0, 35.0).as_deref(), Some("dot"));
    assert_eq!(s.hit(5.0, 5.0).as_deref(), Some("bg"));
    // The masked half of a shape is not there to be clicked: the background is.
    assert_eq!((s.hit(160.0, 20.0).as_deref(), s.hit(180.0, 20.0).as_deref()), (Some("masked"), Some("bg")));
    assert_eq!(s.hit(500.0, 5.0), None);
    // Text is picked anywhere in the box around its ink, between the letters too.
    let words = s.layer_box("words").unwrap();
    for x in words.x + 1..words.x + words.w - 1 {
        assert_eq!(s.hit(x as f64, (words.y + words.h / 2) as f64).as_deref(), Some("words"), "at x {x}");
    }
    // Hidden layers are passed over.
    s.execute(cardsmith_core::commands::Edit::SetLayerProps { id: "square".into(), props: json!({ "visible": false }).as_object().unwrap().clone() }, "Hide").unwrap();
    assert_eq!(s.hit(30.0, 30.0).as_deref(), Some("bg"));

    assert_eq!(s.layer_box("dot"), Some(Region::new(70, 30, 10, 10)));
    let pair = s.layer_box("pair").unwrap();
    assert_eq!((pair.x, pair.y, pair.x + pair.w, pair.y + pair.h), (70, 30, 80, 40), "only the showing child counts");
    let words = s.layer_box("words").unwrap();
    assert!(words.x >= 18 && words.y >= 78 && words.w > 10 && words.h > 5, "{words:?}");
}

#[test]
fn a_drag_while_zoomed_out_shows_on_screen_and_leaves_the_real_picture_exact() {
    let (doc, assets) = scene();
    let mut s = Session::new(doc, assets.clone());
    s.add_font(FONT.to_vec()).unwrap();
    // A quarter size on screen: 50 x 30.
    s.set_view(View { scale: 0.25, x: 0.0, y: 0.0, w: 50, h: 30 });
    let view = |s: &Session, x: usize, y: usize| -> [u8; 4] { s.view_pixels()[(y * 50 + x) * 4..(y * 50 + x) * 4 + 4].try_into().unwrap() };
    assert_eq!(view(&s, 10, 10), [128, 128, 128, 255]);
    let before = s.doc().clone();
    for dx in [20.0, 60.0, 100.0] {
        s.preview(place_edit(&before, "pair", &Placement::shift(dx, 0.0)).unwrap()).unwrap();
        // The first step of a drag redraws the whole screen (a smaller copy takes over); later ones only what moved.
        assert!(s.dirty_view().area() > 0);
    }
    assert_eq!((view(&s, 10, 10), view(&s, 35, 10)), ([255, 255, 255, 255], [128, 128, 128, 255]));
    assert!(s.commit_preview("Move"));
    assert_eq!((view(&s, 10, 10), view(&s, 35, 10)), ([255, 255, 255, 255], [128, 128, 128, 255]));
    // What is saved or exported is the real document drawn at full size.
    let mut fresh = cardsmith_core::render::Renderer::new();
    fresh.fonts_mut().add(FONT.to_vec()).unwrap();
    let full = fresh.render(s.doc(), &assets).pixels;
    assert!(s.frame() == full);
    assert!(s.undo());
    assert_eq!(view(&s, 10, 10), [128, 128, 128, 255]);
    assert!(*s.doc() == before);
}
