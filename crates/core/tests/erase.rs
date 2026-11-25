//! Erasing any layer through a mask, bringing it back, and the magic eraser.

mod common;
use cardsmith_core::arrange::{place_edit, Placement};
use cardsmith_core::doc::build::pixel;
use cardsmith_core::doc::*;
use cardsmith_core::paint::Brush;
use cardsmith_core::render::{encode_png, render};
use cardsmith_core::serialize::{load_cardsmith, AssetBytes};
use cardsmith_core::session::Session;
use cardsmith_core::tree;
use common::*;
use serde_json::json;

/// White background; a gray shape; a picture that is a red field with a blue square in its middle.
fn scene() -> (Doc, AssetBytes) {
    let mut doc = Doc::empty(200, 120, 72.0);
    doc.assets.insert("art".into(), serde_json::from_value(json!({ "kind": "image", "path": "assets/art.png", "width": 60, "height": 60 })).unwrap());
    doc.root.children = vec![rect("bg", 0.0, 0.0, 200.0, 120.0, WHITE), rect("shape", 10.0, 10.0, 60.0, 60.0, GRAY), pixel("art", "art", [1.0, 0.0, 100.0, 0.0, 1.0, 30.0])];
    let mut picture = Vec::new();
    for y in 0..60 {
        for x in 0..60 {
            picture.extend(if (20..40).contains(&x) && (20..40).contains(&y) { [0u8, 0, 255, 255] } else { [255u8, 0, 0, 255] });
        }
    }
    let mut assets = AssetBytes::new();
    assets.insert("art".into(), encode_png(60, 60, &picture).unwrap());
    (doc, assets)
}

fn px(s: &mut Session, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * s.doc().width + x) * 4) as usize;
    s.frame()[i..i + 4].try_into().unwrap()
}

#[track_caller]
fn assert_exact(s: &mut Session) {
    let (doc, assets) = load_cardsmith(&s.save().unwrap()).unwrap();
    assert!(render(&doc, &assets).pixels == s.frame(), "the session's picture differs from a fresh render of what it saves");
}

fn eraser(size: f32, restore: bool) -> Brush {
    Brush { size, hardness: 1.0, erase: true, restore, ..Brush::default() }
}

#[test]
fn the_eraser_works_on_any_layer_through_a_mask_and_what_it_took_can_be_brought_back() {
    let (doc, assets) = scene();
    let mut s = Session::new(doc, assets);
    // A shape has no pixels of its own to rub out; it gets a mask.
    s.stroke_begin("shape", eraser(20.0, false)).unwrap();
    s.stroke_to(40.0, 40.0, 1.0);
    s.stroke_end("Eraser");
    assert_eq!((px(&mut s, 40, 40), px(&mut s, 15, 15)), ([255, 255, 255, 255], [128, 128, 128, 255]));
    let mask = tree::find(s.doc(), "shape").unwrap().base().mask.clone().expect("the shape was given a mask");
    assert!(mask.asset.starts_with("mask-") && mask.outside == Some(Outside::Reveal));
    assert_eq!(s.labels(), ["Mask shape", "Eraser"]);
    assert_exact(&mut s);

    // The same on a picture: its own pixels are untouched, so restoring brings them back.
    s.stroke_begin("art", eraser(30.0, false)).unwrap();
    s.stroke_to(130.0, 60.0, 1.0);
    s.stroke_end("Eraser");
    assert_eq!(px(&mut s, 130, 60), [255, 255, 255, 255]);
    s.stroke_begin("art", eraser(30.0, true)).unwrap();
    s.stroke_to(130.0, 60.0, 1.0);
    s.stroke_end("Restore");
    assert_eq!(px(&mut s, 130, 60), [0, 0, 255, 255]);
    // A second stroke uses the mask that is there: no new mask step.
    assert_eq!(s.labels(), ["Mask shape", "Eraser", "Mask art", "Eraser", "Restore"]);
    assert_exact(&mut s);

    // Undo goes back through it all; color on such a layer is refused with a reason.
    for _ in 0..5 {
        assert!(s.undo());
    }
    assert_eq!((px(&mut s, 40, 40), tree::find(s.doc(), "shape").unwrap().base().mask.is_none()), ([128, 128, 128, 255], true));
    let refused = s.stroke_begin("shape", Brush::default()).unwrap_err().to_string();
    assert!(refused.contains("paint layer"), "{refused}");
}

#[test]
fn the_magic_eraser_cuts_out_the_color_clicked_and_stops_where_the_color_changes() {
    let (doc, assets) = scene();
    let mut s = Session::new(doc, assets);
    // Click the red field: all of it goes, the blue square stays.
    let gone = s.magic_erase("art", 105.0, 35.0, 30.0, true, "Magic eraser").unwrap();
    assert_eq!(gone, 60 * 60 - 20 * 20);
    assert_eq!((px(&mut s, 105, 35), px(&mut s, 155, 85), px(&mut s, 130, 60)), ([255, 255, 255, 255], [255, 255, 255, 255], [0, 0, 255, 255]));
    assert_eq!(s.labels(), ["Mask art", "Magic eraser"]);
    assert_exact(&mut s);
    // What is left can be moved and scaled, and its mask goes with it.
    let twice = place_edit(s.doc(), "art", &Placement::scale_about(2.0, 100.0, 30.0)).unwrap();
    s.execute(twice, "Scale").unwrap();
    // The blue square was at 120..140; it is now at 140..180, with nothing around it.
    assert_eq!((px(&mut s, 160, 90), px(&mut s, 110, 40), px(&mut s, 190, 100)), ([0, 0, 255, 255], [255, 255, 255, 255], [255, 255, 255, 255]));
    assert_exact(&mut s);
    // Erasing again after a scale works: the mask is made plain again first.
    s.magic_erase("art", 160.0, 90.0, 30.0, true, "Magic eraser").unwrap();
    assert_eq!(px(&mut s, 160, 90), [255, 255, 255, 255]);
    assert_exact(&mut s);
    assert!(s.undo() && s.undo() && s.undo());
    assert_eq!(px(&mut s, 130, 60), [0, 0, 255, 255]);
    assert!(s.undo() && s.undo());
    assert_eq!((px(&mut s, 105, 35), tree::find(s.doc(), "art").unwrap().base().mask.is_none()), ([255, 0, 0, 255], true));

    // Not contiguous: every part of that color, wherever it is. And a click on nothing says so.
    let (mut doc, assets) = scene();
    doc.root.children.push(pixel("copy", "art", [1.0, 0.0, 20.0, 0.0, 1.0, 50.0]));
    let mut s = Session::new(doc, assets);
    assert!(s.magic_erase("art", 5.0, 5.0, 30.0, true, "Magic eraser").unwrap_err().to_string().contains("nothing of this layer"));
    assert_eq!(s.magic_erase("art", 130.0, 60.0, 30.0, false, "Magic eraser").unwrap(), 400);
    assert_eq!((px(&mut s, 130, 60), px(&mut s, 105, 35)), ([255, 255, 255, 255], [255, 0, 0, 255]));
}

#[test]
fn a_drawn_area_is_erased_or_everything_but_it_which_cuts_a_figure_out() {
    let (doc, assets) = scene();
    let mut s = Session::new(doc, assets);
    // A triangle drawn around the blue square on the picture (which sits at 120..140, 50..70): erase outside it.
    s.erase_area("art", "M130 35L155 80L105 80Z", true, "Erase outside").unwrap();
    assert_eq!((px(&mut s, 130, 60), px(&mut s, 105, 35), px(&mut s, 155, 85)), ([0, 0, 255, 255], [255, 255, 255, 255], [255, 255, 255, 255]));
    // Inside the triangle but outside the square, the red is still there.
    assert_eq!(px(&mut s, 130, 45), [255, 0, 0, 255]);
    assert_eq!(s.labels(), ["Mask art", "Erase outside"]);
    assert_exact(&mut s);
    // Then a rectangle inside is erased too.
    s.erase_area("art", "M125 55L135 55L135 65L125 65Z", false, "Erase").unwrap();
    assert_eq!((px(&mut s, 130, 60), px(&mut s, 122, 52)), ([255, 255, 255, 255], [0, 0, 255, 255]));
    assert_exact(&mut s);
    assert!(s.undo() && s.undo());
    assert_eq!(px(&mut s, 105, 35), [255, 0, 0, 255]);
    assert!(s.erase_area("art", "not a path", false, "Erase").is_err());
}
