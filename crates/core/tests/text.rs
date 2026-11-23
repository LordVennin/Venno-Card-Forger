//! Text layout and drawing, checked with exact numbers against the test font
//! `fixtures/blocks.ttf` (see tools/make-test-font.py): rectangles on the
//! baseline, 1000 units per em. At size 100: "a" is 40 x 50 and advances 50,
//! "b" is 80 x 50 and advances 100, "d" is 40 x 70 and advances 50, a space
//! advances 25, and every rectangle starts 5 px right of the pen.

mod common;

use cardsmith_core::doc::*;
use cardsmith_core::raster::Region;
use cardsmith_core::render::Renderer;
use cardsmith_core::serialize::AssetBytes;
use cardsmith_core::session::Session;
use cardsmith_core::text::{layout_text, Fonts, TextLayout};
use common::*;
use serde_json::{json, Value};

const FONT: &[u8] = include_bytes!("fixtures/blocks.ttf");

fn fonts() -> Fonts {
    let mut fonts = Fonts::new();
    assert_eq!(fonts.add(FONT.to_vec()).unwrap(), ["Blocks-Regular"]);
    fonts
}

/// A text layer at size 100 in the test font, with `extra` merged over it.
fn text(runs: Value, extra: Value) -> TextLayer {
    let mut layer = json!({
        "kind": "text", "id": "t", "name": "Text", "visible": true, "opacity": 1, "blend": "normal", "clip": false,
        "box": { "x": 0, "y": 0, "w": 0, "h": 0 }, "content": { "runs": runs },
        "style": { "font": "Blocks-Regular", "size": 100, "color": [1, 0, 0, 1], "align": "left" }, "fit": "none"
    });
    for (key, value) in extra.as_object().unwrap() {
        if key == "style" {
            for (k, v) in value.as_object().unwrap() {
                layer["style"][k] = v.clone();
            }
        } else {
            layer[key] = value.clone();
        }
    }
    match serde_json::from_value(layer).unwrap() {
        Layer::Text(t) => t,
        _ => unreachable!(),
    }
}

fn plain(s: &str, extra: Value) -> TextLayout {
    layout_text(&text(json!([{ "text": s }]), extra), &fonts())
}

fn ink(layout: &TextLayout) -> [f64; 4] {
    let ink = layout.ink.expect("the text has ink");
    ink.map(|v| (v * 1000.0).round() / 1000.0)
}

#[test]
fn point_text_hangs_from_its_origin_on_a_whole_pixel_baseline() {
    let l = plain("ab", json!({ "origin": { "x": 10, "y": 100.4 } }));
    assert_eq!(ink(&l), [15.0, 50.0, 145.0, 100.0]);
    assert_eq!((l.lines, l.scale), (1, 1.0));
    // Point text never wraps, whatever its box says.
    let long = plain("ab ab ab ab", json!({ "origin": { "x": 0, "y": 100 }, "box": { "x": 0, "y": 0, "w": 60, "h": 60 } }));
    assert_eq!(long.lines, 1);
}

#[test]
fn point_text_is_centered_on_or_ends_at_its_origin() {
    // "ab" advances 150.
    let center = plain("ab", json!({ "origin": { "x": 200, "y": 100 }, "style": { "align": "center" } }));
    assert_eq!(ink(&center), [130.0, 50.0, 260.0, 100.0]);
    let right = plain("ab", json!({ "origin": { "x": 200, "y": 100 }, "style": { "align": "right" } }));
    assert_eq!(ink(&right), [55.0, 50.0, 185.0, 100.0]);
}

#[test]
fn line_breaks_start_new_lines_at_the_leading() {
    // Default leading is 1.2 x size.
    let auto = plain("a\nb", json!({ "origin": { "x": 0, "y": 100 } }));
    assert_eq!(ink(&auto), [5.0, 50.0, 85.0, 220.0]);
    let set = plain("a\nb", json!({ "origin": { "x": 0, "y": 100 }, "style": { "leading": 1.5, "paragraphSpacing": 30 } }));
    assert_eq!(ink(&set), [5.0, 50.0, 85.0, 280.0]);
    assert_eq!(set.lines, 2);
}

#[test]
fn box_text_wraps_at_spaces_and_starts_one_tall_letter_below_the_top() {
    // "ab" is 150 wide and a space 25, so a 200 px box holds one word a line.
    let l = plain("ab ab ab", json!({ "box": { "x": 20, "y": 30, "w": 200, "h": 500 } }));
    assert_eq!(l.lines, 3);
    // First baseline at 30 + 70 (the height of "d"), then 120 apart.
    assert_eq!(ink(&l), [25.0, 50.0, 155.0, 340.0]);
    // Wide enough for two words; the space at the break takes no room.
    let two = plain("ab ab ab", json!({ "box": { "x": 0, "y": 0, "w": 325, "h": 500 } }));
    assert_eq!(two.lines, 2);
    // A word wider than the box is broken between letters, as Photoshop does.
    let wide = plain("a bbb a", json!({ "box": { "x": 0, "y": 0, "w": 100, "h": 500 } }));
    assert_eq!(wide.lines, 5);
}

#[test]
fn box_text_aligns_within_its_box() {
    let center = plain("ab", json!({ "box": { "x": 0, "y": 0, "w": 350, "h": 200 }, "style": { "align": "center" } }));
    assert_eq!(ink(&center), [105.0, 20.0, 235.0, 70.0]);
    let right = plain("ab", json!({ "box": { "x": 0, "y": 0, "w": 350, "h": 200 }, "style": { "align": "right" } }));
    assert_eq!(ink(&right), [205.0, 20.0, 335.0, 70.0]);
}

#[test]
fn runs_carry_their_own_size_color_and_tracking() {
    let layer = text(
        json!([{ "text": "a" }, { "text": "a", "size": 200, "color": [0, 0, 1, 1] }, { "text": "aa", "tracking": 100 }]),
        json!({ "origin": { "x": 0, "y": 300 }, "style": { "scaleX": 2 } }),
    );
    let l = layout_text(&layer, &fonts());
    // Pens at 0, 100, 300, 410 (tracking adds 10 after each tracked glyph); glyphs are twice as wide.
    assert_eq!(ink(&l), [10.0, 200.0, 500.0, 300.0]);
    let colors: Vec<Color> = l.parts.iter().map(|(c, _)| *c).collect();
    assert_eq!(colors, [RED, BLUE, RED]);
}

#[test]
fn paragraphs_can_set_their_own_spacing_indent_and_alignment() {
    // Three paragraphs in a 300 px box: plain, a bullet-style hanging indent, right-aligned.
    let layer = text(
        json!([{ "text": "a\na a a a a a\na" }]),
        json!({ "box": { "x": 0, "y": 0, "w": 300, "h": 900 }, "style": { "leading": 1, "paragraphSpacing": 40 } }),
    );
    let mut styled = layer.clone();
    styled.content.paragraphs = serde_json::from_value(json!([{}, { "spaceBefore": 10, "indent": 100, "firstLineIndent": -60 }, { "align": "right" }])).unwrap();
    let plain = layout_text(&layer, &fonts());
    let l = layout_text(&styled, &fonts());
    // Unstyled: "a a a a" (275 wide) then "a a"; 4 lines in all.
    assert_eq!(plain.lines, 4);
    // Styled: the first bullet line starts at 40 and has 260 of room ("a a a" = 200 advance + trailing), later lines start at 100 with 200 of room.
    let p = &l.paragraph_ink;
    assert_eq!(p[0].unwrap().map(f64::round), [5.0, 20.0, 45.0, 70.0]);
    assert_eq!(p[1].unwrap()[0], 45.0);
    // 10 px above the second paragraph instead of 40: first baseline 70, second 70 + 100 + 10.
    assert_eq!(p[1].unwrap()[1], 130.0);
    // "a a a" fits the first line's 260 px and the other three the 200 px left beside the indent.
    assert_eq!(l.lines, 4);
    // Second line of the bullet hangs at the indent.
    let hanging = layout_text(&{ let mut one = styled.clone(); one.content.runs[0].text = "a\na a a a".into(); one.content.paragraphs.truncate(2); one }, &fonts());
    assert_eq!(hanging.lines, 3);
    // The last paragraph ends at the right edge of the box: its "a" is 40 wide and the advance is 50.
    assert_eq!(p[2].unwrap()[2], 295.0);
    // The style's 40 px applies before the third paragraph, which has no spacing of its own.
    assert_eq!(p[2].unwrap()[1] - p[1].unwrap()[3], 100.0 + 40.0 - 50.0);
}

#[test]
fn fitting_shrinks_the_type_but_not_paragraph_spacing_or_indents() {
    let mut layer = text(json!([{ "text": "a\na" }]), json!({ "box": { "x": 0, "y": 0, "w": 300, "h": 120 }, "fit": "shrinkHeight", "style": { "leading": 1 } }));
    layer.content.paragraphs = serde_json::from_value(json!([{}, { "spaceBefore": 40, "indent": 30 }])).unwrap();
    let l = layout_text(&layer, &fonts());
    assert!(l.scale < 1.0);
    let (a, b) = (l.paragraph_ink[0].unwrap(), l.paragraph_ink[1].unwrap());
    // The second "a" still starts 30 px in (plus the glyph's own 5 px, scaled).
    assert!((b[0] - (30.0 + 5.0 * l.scale)).abs() < 1e-6, "{b:?}");
    // Baselines are one (scaled) line plus the full 40 px apart, to the nearest pixel.
    assert!(((b[3] - a[3]) - (100.0 * l.scale + 40.0)).abs() <= 1.0, "{a:?} {b:?} {}", l.scale);
}

#[test]
fn shrink_width_scales_point_text_down_to_its_box_and_never_up() {
    // "bbb" has 280 px of ink.
    let fits = plain("bbb", json!({ "origin": { "x": 0, "y": 100 }, "box": { "x": 0, "y": 0, "w": 300, "h": 100 }, "fit": "shrinkWidth" }));
    assert_eq!(fits.scale, 1.0);
    // The ink must end by the right edge of the box (x 142.5): at half size it runs from 2.5 to 142.5.
    let l = plain("bbb", json!({ "origin": { "x": 0, "y": 100 }, "box": { "x": 0, "y": 0, "w": 142.5, "h": 100 }, "fit": "shrinkWidth" }));
    assert!((l.scale - 0.5).abs() < 1e-5, "{}", l.scale);
    let i = ink(&l);
    // It also moves up by 0.3 of the size it lost, to stay centered in its bar.
    assert_eq!((i[2].round(), (i[3] - i[1]).round(), i[3]), (143.0, 25.0, 85.0));
    // Right-aligned text is held to the left edge instead.
    let r = plain("bbb", json!({ "origin": { "x": 300, "y": 100 }, "box": { "x": 160, "y": 0, "w": 140, "h": 100 }, "fit": "shrinkWidth", "style": { "align": "right" } }));
    // Its ink starts 295 x scale left of the origin.
    assert!((r.scale - 140.0 / 295.0).abs() < 1e-5 && (ink(&r)[0] - 160.0).abs() < 0.01, "{} {:?}", r.scale, ink(&r));
}

#[test]
fn shrinking_in_steps_stops_at_the_first_size_that_fits_then_tries_half_a_step_back() {
    // One line of "a" is 50 tall at size 100, so it fits a 33 px box at size 66.
    let at = |step: f64| plain("a", json!({ "box": { "x": 0, "y": 0, "w": 300, "h": 33 }, "fit": "shrinkHeight", "fitStep": step })).scale;
    // Steps of 10: 90, 80, 70, 60 fits; 65 fits too.
    assert!((at(10.0) - 0.65).abs() < 1e-9, "{}", at(10.0));
    // Steps of 8: 92, 84, 76, 68, 60 fits; 64 fits too.
    assert!((at(8.0) - 0.64).abs() < 1e-9, "{}", at(8.0));
    // Steps of 17: 83, 66 fits; 74.5 does not, so it stays.
    assert!((at(17.0) - 0.66).abs() < 1e-9, "{}", at(17.0));
    // Without a step the exact largest size is found.
    let exact = plain("a", json!({ "box": { "x": 0, "y": 0, "w": 300, "h": 33 }, "fit": "shrinkHeight" })).scale;
    assert!((exact - 0.66).abs() < 1e-5, "{exact}");
}

#[test]
fn shrink_height_finds_the_largest_size_that_fits_the_box() {
    let at = |h: f64| plain("ab ab ab ab", json!({ "box": { "x": 0, "y": 0, "w": 200, "h": h }, "fit": "shrinkHeight" }));
    // Four lines: 3 x 120 of leading plus 50 of ink.
    assert_eq!((at(410.0).scale, at(410.0).lines), (1.0, 4));
    let l = at(200.0);
    let i = ink(&l);
    assert!(l.scale < 1.0 && i[3] - i[1] <= 200.0, "scale {} ink {:?}", l.scale, i);
    // Nothing larger fits: a slightly bigger scale would overflow, or wrap differently and still not fit better.
    assert!(i[3] - i[1] > 190.0 || l.lines < 4, "scale {} lines {} ink {:?}", l.scale, l.lines, i);
}

#[test]
fn box_text_can_be_centered_vertically() {
    let l = plain("a", json!({ "box": { "x": 0, "y": 100, "w": 300, "h": 250 }, "verticalAlign": "center" }));
    assert_eq!(ink(&l), [5.0, 200.0, 45.0, 250.0]);
}

#[test]
fn missing_fonts_are_named_and_the_rest_still_draws() {
    let layer = text(json!([{ "text": "a" }, { "text": "a", "font": "Nope-Bold" }]), json!({ "origin": { "x": 0, "y": 100 } }));
    let l = layout_text(&layer, &fonts());
    assert_eq!(l.missing, ["Nope-Bold"]);
    assert_eq!(ink(&l), [5.0, 50.0, 45.0, 100.0]);
    // Font names match loosely: spacing, case and hyphens do not matter.
    let loose = plain("a", json!({ "origin": { "x": 0, "y": 100 }, "style": { "font": "blocks regular" } }));
    assert!(loose.missing.is_empty() && loose.ink.is_some());
}

fn doc_with_text() -> Doc {
    let mut doc = Doc::empty(300, 200, 72.0);
    let words = Layer::Text(text(json!([{ "text": "ab " }, { "text": "ab", "color": [0, 0, 1, 1] }]), json!({ "box": { "x": 10, "y": 10, "w": 200, "h": 180 } })));
    doc.root.children = vec![cardsmith_core::doc::build::shape("bg", "M0 0H300V200H0Z", WHITE), words];
    doc
}

#[test]
fn draws_glyphs_in_their_colors_and_a_region_equals_the_same_crop() {
    let doc = doc_with_text();
    let assets = AssetBytes::new();
    let mut renderer = Renderer::new();
    renderer.fonts_mut().add(FONT.to_vec()).unwrap();
    let full = renderer.render(&doc, &assets);
    assert!(full.warnings.is_empty(), "{:?}", full.warnings);
    // First line: baseline at 80, "a" from x 15 to 55. Second line is blue, 120 lower.
    assert_eq!(px(&full, 30, 60), [255, 0, 0, 255]);
    assert_eq!(px(&full, 30, 180), [0, 0, 255, 255]);
    assert_eq!(px(&full, 30, 100), [255, 255, 255, 255]);

    for region in [Region::new(20, 40, 61, 150), Region::new(0, 0, 37, 70), Region::new(140, 150, 160, 50)] {
        // A fresh renderer, so nothing is reused from the full render.
        let mut fresh = Renderer::new();
        fresh.fonts_mut().add(FONT.to_vec()).unwrap();
        let (part, _) = fresh.render_region(&doc, &assets, region);
        for y in 0..region.h {
            for x in 0..region.w {
                let i = ((y * region.w + x) * 4) as usize;
                assert_eq!(part.data[i..i + 4], px(&full, (region.x + x) as u32, (region.y + y) as u32), "at {x},{y} of {region:?}");
            }
        }
    }
}

#[test]
fn loading_a_font_redraws_an_open_document() {
    let doc = doc_with_text();
    let mut session = Session::new(doc.clone(), AssetBytes::new());
    assert!(session.warnings().iter().any(|w| w.contains("needs the font \"Blocks-Regular\"")), "{:?}", session.warnings());
    session.add_font(FONT.to_vec()).unwrap();
    assert!(session.warnings().is_empty(), "{:?}", session.warnings());

    let mut renderer = Renderer::new();
    renderer.fonts_mut().add(FONT.to_vec()).unwrap();
    let fresh = renderer.render(&doc, &AssetBytes::new());
    assert_eq!(session.frame(), &fresh.pixels[..]);

    // Editing the text redraws both where it was and where it now is.
    let props = json!({ "content": { "runs": [{ "text": "b" }] }, "origin": { "x": 150, "y": 150 } });
    session.execute(cardsmith_core::commands::Edit::SetLayerProps { id: "t".into(), props: props.as_object().unwrap().clone() }, "Edit text").unwrap();
    let mut renderer = Renderer::new();
    renderer.fonts_mut().add(FONT.to_vec()).unwrap();
    let fresh = renderer.render(session.doc(), &AssetBytes::new());
    assert_eq!(session.frame(), &fresh.pixels[..]);
    session.undo();
    let fresh = renderer.render(session.doc(), &AssetBytes::new());
    assert_eq!(session.frame(), &fresh.pixels[..]);
}
