//! Ported from packages/render/src/render.test.ts.

mod common;
use cardsmith_core::doc::build::{group, pixel, rect_path, shape, with};
use cardsmith_core::doc::*;
use cardsmith_core::render::RenderResult;
use common::*;
use serde_json::{json, Value};

fn full(id: &str, color: Color) -> Layer {
    rect(id, 0.0, 0.0, 8.0, 8.0, color)
}
fn render8(layers: Vec<Layer>) -> RenderResult {
    scene(8, 8, layers, &[])
}

// ---- basics -----------------------------------------------------------------

#[test]
fn draws_a_shape_where_its_path_is() {
    let r = render8(vec![rect("a", 2.0, 2.0, 4.0, 4.0, RED)]);
    assert_eq!(px(&r, 3, 3), [255, 0, 0, 255]);
    assert_eq!(px(&r, 1, 1), CLEAR);
    assert_eq!(px(&r, 6, 6), CLEAR);
    assert!(r.warnings.is_empty());
}

#[test]
fn paints_children_bottom_first() {
    let r = render8(vec![full("bottom", RED), rect("top", 0.0, 0.0, 4.0, 8.0, BLUE)]);
    assert_eq!(px(&r, 1, 1), [0, 0, 255, 255]);
    assert_eq!(px(&r, 6, 1), [255, 0, 0, 255]);
}

#[test]
fn skips_invisible_layers_and_applies_opacity() {
    let r = render8(vec![full("bg", WHITE), with(full("hidden", BLUE), |b| b.visible = false), with(full("half", RED), |b| b.opacity = 0.5)]);
    near(px(&r, 4, 4), [255, 128, 128, 255]);
}

#[test]
fn fill_opacity_acts_like_opacity_without_effects() {
    let r = render8(vec![full("bg", WHITE), with(full("a", BLACK), |b| b.fill_opacity = Some(0.25))]);
    near(px(&r, 4, 4), [191, 191, 191, 255]);
}

#[test]
fn draws_shape_strokes_centered_or_inside() {
    let stroke = |align: &str| scene(16, 16, vec![patch(rect("s", 4.0, 4.0, 8.0, 8.0, RED), json!({ "stroke": { "color": [0, 0, 1, 1], "width": 2, "align": align } }))], &[]);
    let center = stroke("center");
    assert_eq!(px(&center, 3, 8), [0, 0, 255, 255]); // 1 px outside the edge
    assert_eq!(px(&center, 4, 8), [0, 0, 255, 255]); // 1 px inside
    assert_eq!(px(&center, 5, 8), [255, 0, 0, 255]);
    let inside = stroke("inside");
    assert_eq!(px(&inside, 3, 8), CLEAR);
    assert_eq!(px(&inside, 5, 8), [0, 0, 255, 255]);
    assert_eq!(px(&inside, 6, 8), [255, 0, 0, 255]);
}

#[test]
fn draws_pixel_layers_through_their_transform() {
    let art = png(2, 2, |x, y| if x == 0 && y == 0 { [255, 0, 0, 255] } else { [0, 255, 0, 255] });
    // Scale ×2, then move to (3, 1): the red texel covers (3..4, 1..2).
    let r = scene(8, 8, vec![pixel("p", "art", [2.0, 0.0, 3.0, 0.0, 2.0, 1.0])], &[("art", art.clone())]);
    assert_eq!(px(&r, 3, 1), [255, 0, 0, 255]);
    assert_eq!(px(&r, 6, 4), [0, 255, 0, 255]);
    assert_eq!(px(&r, 2, 1), CLEAR);
    assert_eq!(px(&r, 7, 4), CLEAR);
    // Unscaled images are placed directly, including partly off the canvas.
    let placed = scene(8, 8, vec![pixel("p", "art", [1.0, 0.0, 7.0, 0.0, 1.0, -1.0])], &[("art", art)]);
    assert_eq!(px(&placed, 7, 0), [0, 255, 0, 255]);
    assert_eq!(px(&placed, 6, 0), CLEAR);
}

#[test]
fn reports_text_layers_and_missing_images_instead_of_failing() {
    let text: Layer = serde_json::from_value(json!({
        "kind": "text", "id": "t", "name": "Rules", "visible": true, "opacity": 1, "blend": "normal", "clip": false,
        "box": { "x": 0, "y": 0, "w": 8, "h": 8 }, "content": { "runs": [] },
        "style": { "font": "x", "size": 1, "color": [1, 0, 0, 1], "align": "left" }, "fit": "none"
    }))
    .unwrap();
    let r = render8(vec![full("bg", WHITE), text, with(pixel("p", "gone", IDENTITY), |b| b.name = "Art".into())]);
    assert_eq!(px(&r, 4, 4), [255, 255, 255, 255]);
    let warnings = r.warnings.join("\n");
    assert!(warnings.contains("Text layer \"Rules\" was skipped"), "{warnings}");
    assert!(warnings.contains("Asset \"gone\" has no data"), "{warnings}");

    let bad = scene(8, 8, vec![pixel("p", "junk", IDENTITY)], &[("junk", vec![1, 2, 3])]);
    assert!(bad.warnings.join("\n").contains("is not a readable image"));
    let path = render8(vec![shape("s", "M0 0 L nonsense", RED)]);
    assert!(path.warnings.join("\n").contains("could not be parsed"));
}

// ---- blend modes --------------------------------------------------------------

const S: [f64; 3] = [0.8, 0.4, 0.1];
const D: [f64; 3] = [0.3, 0.6, 0.5];
const MODES: [&str; 7] = ["normal", "multiply", "screen", "overlay", "softLight", "linearLight", "luminosity"];

fn expected(mode: &str) -> [f64; 3] {
    let lum = |c: [f64; 3]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
    let soft_d = |d: f64| if d <= 0.25 { ((16.0 * d - 12.0) * d + 4.0) * d } else { d.sqrt() };
    let f = |s: f64, d: f64| match mode {
        "normal" => s,
        "multiply" => s * d,
        "screen" => s + d - s * d,
        "overlay" => if d <= 0.5 { 2.0 * s * d } else { 1.0 - 2.0 * (1.0 - s) * (1.0 - d) },
        "softLight" => if s <= 0.5 { d - (1.0 - 2.0 * s) * d * (1.0 - d) } else { d + (2.0 * s - 1.0) * (soft_d(d) - d) },
        "linearLight" => (d + 2.0 * s - 1.0).clamp(0.0, 1.0),
        _ => unreachable!(),
    };
    if mode == "luminosity" {
        // Stays in range for these colors, so no clipping.
        return D.map(|d| d + (lum(S) - lum(D)));
    }
    [f(S[0], D[0]), f(S[1], D[1]), f(S[2], D[2])]
}
fn to8(c: [f64; 3], a: u8) -> [u8; 4] {
    [(c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8, a]
}
fn blended(mode: &str, backdrop_alpha: f64, source_alpha: f64) -> RenderResult {
    render8(vec![
        full("d", [D[0], D[1], D[2], backdrop_alpha]),
        patch(full("s", [S[0], S[1], S[2], source_alpha]), json!({ "blend": mode })),
    ])
}

#[test]
fn each_blend_mode_matches_its_formula_over_an_opaque_backdrop() {
    for mode in MODES {
        near(px(&blended(mode, 1.0, 1.0), 4, 4), to8(expected(mode), 255));
    }
}

#[test]
fn over_a_translucent_backdrop_blended_and_unblended_source_mix() {
    for mode in MODES {
        // Backdrop alpha 0.5, source alpha 0.5: a quarter is source only, a quarter blended, a quarter backdrop only.
        let b = expected(mode);
        let mixed = [0, 1, 2].map(|i| (0.25 * S[i] + 0.25 * b[i] + 0.25 * D[i]) / 0.75);
        near_by(px(&blended(mode, 0.5, 0.5), 4, 4), to8(mixed, 191), 2);
    }
}

#[test]
fn blends_normally_where_the_backdrop_is_transparent() {
    let r = render8(vec![patch(full("s", [S[0], S[1], S[2], 1.0]), json!({ "blend": "multiply" }))]);
    near(px(&r, 4, 4), to8(S, 255));
}

// ---- groups -------------------------------------------------------------------

fn multiply_child() -> Layer {
    with(full("m", GRAY), |b| b.blend = BlendMode::Multiply)
}

#[test]
fn pass_through_lets_children_blend_with_what_is_under_the_group() {
    let r = render8(vec![full("bg", RED), group("g", true, vec![multiply_child()])]);
    near(px(&r, 4, 4), [128, 0, 0, 255]);
}

#[test]
fn an_isolated_group_blends_its_children_only_with_each_other() {
    let r = render8(vec![full("bg", RED), group("g", false, vec![multiply_child()])]);
    near(px(&r, 4, 4), [128, 128, 128, 255]);
}

#[test]
fn an_isolated_group_applies_its_opacity_as_a_unit() {
    let g = with(group("g", false, vec![full("a", RED), rect("b", 0.0, 0.0, 4.0, 8.0, BLUE)]), |b| b.opacity = 0.5);
    let r = render8(vec![full("bg", WHITE), g]);
    near(px(&r, 1, 4), [128, 128, 255, 255]); // blue over red inside the group, then 50% over white
    near(px(&r, 6, 4), [255, 128, 128, 255]);
}

#[test]
fn pass_through_opacity_fades_the_groups_whole_contribution() {
    let g = with(group("g", true, vec![multiply_child()]), |b| b.opacity = 0.5);
    let r = render8(vec![full("bg", RED), g]);
    near(px(&r, 4, 4), [191, 0, 0, 255]); // halfway between 255 and 128
}

#[test]
fn hidden_groups_hide_their_children_and_groups_nest() {
    let r = render8(vec![full("bg", RED), with(group("g", true, vec![full("a", BLUE)]), |b| b.visible = false)]);
    assert_eq!(px(&r, 4, 4), [255, 0, 0, 255]);
    let nested = render8(vec![group("a", true, vec![group("b", false, vec![group("c", true, vec![full("leaf", GREEN)])])])]);
    assert_eq!(px(&nested, 4, 4), [0, 255, 0, 255]);
}

// ---- clipping -----------------------------------------------------------------

fn clip(layer: Layer) -> Layer {
    with(layer, |b| b.clip = true)
}

#[test]
fn shows_a_clipped_layer_only_inside_its_base() {
    let r = render8(vec![full("bg", WHITE), rect("base", 2.0, 2.0, 4.0, 4.0, RED), clip(full("clip", BLUE))]);
    assert_eq!(px(&r, 3, 3), [0, 0, 255, 255]);
    assert_eq!(px(&r, 0, 0), [255, 255, 255, 255]);
}

#[test]
fn keeps_the_bases_alpha_exactly_even_where_it_is_translucent() {
    let r = render8(vec![full("base", [1.0, 0.0, 0.0, 0.5]), clip(full("clip", BLUE))]);
    near(px(&r, 4, 4), [0, 0, 255, 128]);
}

#[test]
fn clips_several_layers_to_one_base_and_hiding_the_base_hides_them() {
    let layers = |visible: bool| {
        vec![
            full("bg", WHITE),
            with(rect("base", 2.0, 2.0, 4.0, 4.0, RED), |b| b.visible = visible),
            clip(rect("c1", 0.0, 0.0, 4.0, 8.0, BLUE)),
            clip(rect("c2", 0.0, 0.0, 8.0, 3.0, GREEN)),
            rect("next", 6.0, 6.0, 2.0, 2.0, BLACK),
        ]
    };
    let shown = render8(layers(true));
    assert_eq!(px(&shown, 2, 2), [0, 255, 0, 255]);
    assert_eq!(px(&shown, 3, 4), [0, 0, 255, 255]);
    assert_eq!(px(&shown, 5, 5), [255, 0, 0, 255]);
    assert_eq!(px(&shown, 7, 7), [0, 0, 0, 255]); // the next unclipped layer is independent
    let hidden = render8(layers(false));
    assert_eq!(px(&hidden, 3, 4), [255, 255, 255, 255]);
    assert_eq!(px(&hidden, 7, 7), [0, 0, 0, 255]);
}

#[test]
fn blends_a_clipped_layer_with_its_base_not_the_backdrop() {
    let r = render8(vec![full("bg", BLUE), full("base", [1.0, 1.0, 0.0, 1.0]), clip(with(full("clip", GRAY), |b| b.blend = BlendMode::Multiply))]);
    near(px(&r, 4, 4), [128, 128, 0, 255]);
}

#[test]
fn applies_the_bases_opacity_to_the_clipped_stack_as_one() {
    let r = render8(vec![full("bg", WHITE), with(full("base", RED), |b| b.opacity = 0.5), clip(full("clip", BLACK))]);
    near(px(&r, 4, 4), [128, 128, 128, 255]);
}

#[test]
fn clips_to_a_group() {
    let r = render8(vec![group("g", false, vec![rect("a", 0.0, 0.0, 4.0, 8.0, RED)]), clip(full("clip", GREEN))]);
    assert_eq!(px(&r, 1, 4), [0, 255, 0, 255]);
    assert_eq!(px(&r, 6, 4), CLEAR);
}

// ---- masks --------------------------------------------------------------------

/// 4×4 mask: left half white, right half black.
fn half() -> Vec<u8> {
    png(4, 4, |x, _| if x < 2 { [255, 255, 255, 255] } else { [0, 0, 0, 255] })
}
fn masked(extra: Value) -> RenderResult {
    let mut mask = json!({ "asset": "m", "offset": { "x": 2, "y": 2 } });
    mask.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    scene(8, 8, vec![patch(full("a", RED), json!({ "mask": mask }))], &[("m", half())])
}

#[test]
fn raster_masks_reveal_white_hide_black_and_hide_outside_by_default() {
    let r = masked(json!({}));
    assert_eq!(px(&r, 3, 3), [255, 0, 0, 255]);
    assert_eq!(px(&r, 5, 3)[3], 0);
    assert_eq!(px(&r, 0, 0)[3], 0);
}

#[test]
fn raster_masks_can_reveal_outside_and_invert() {
    let reveal = masked(json!({ "outside": "reveal" }));
    assert_eq!(px(&reveal, 0, 0), [255, 0, 0, 255]);
    assert_eq!(px(&reveal, 5, 3)[3], 0);
    let inverted = masked(json!({ "inverted": true }));
    assert_eq!(px(&inverted, 3, 3)[3], 0);
    assert_eq!(px(&inverted, 5, 3), [255, 0, 0, 255]);
    assert_eq!(px(&inverted, 0, 0), [255, 0, 0, 255]); // inversion includes the outside
}

#[test]
fn raster_masks_can_be_disabled_and_fade_with_density() {
    assert_eq!(px(&masked(json!({ "enabled": false })), 5, 3), [255, 0, 0, 255]);
    near(px(&masked(json!({ "density": 0.5 })), 5, 3), [255, 0, 0, 128]);
    assert_eq!(px(&masked(json!({ "density": 0.5 })), 3, 3), [255, 0, 0, 255]);
}

#[test]
fn gray_levels_are_partial_coverage_and_feather_softens_edges() {
    let gray = png(8, 8, |_, _| [64, 64, 64, 255]);
    let r = scene(8, 8, vec![patch(full("a", RED), json!({ "mask": { "asset": "m", "offset": { "x": 0, "y": 0 } } }))], &[("m", gray)]);
    near(px(&r, 4, 4), [255, 0, 0, 64]);

    let edge = px(&masked(json!({ "feather": 2 })), 4, 3)[3]; // just past the white/black boundary
    assert!(edge > 20 && edge < 235, "edge alpha {edge}");
    // With the outside revealed, feathering must not darken the canvas border.
    let border = masked(json!({ "feather": 2, "outside": "reveal" }));
    assert_eq!(px(&border, 0, 7), [255, 0, 0, 255]);
}

#[test]
fn vector_masks_apply_invert_and_disable() {
    let vm = |extra: Value| {
        let mut mask = json!({ "path": { "d": rect_path(2.0, 2.0, 4.0, 4.0) } });
        mask.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        render8(vec![patch(full("a", RED), json!({ "vectorMask": mask }))])
    };
    assert_eq!(px(&vm(json!({})), 3, 3), [255, 0, 0, 255]);
    assert_eq!(px(&vm(json!({})), 0, 0)[3], 0);
    assert_eq!(px(&vm(json!({ "inverted": true })), 3, 3)[3], 0);
    assert_eq!(px(&vm(json!({ "inverted": true })), 0, 0), [255, 0, 0, 255]);
    assert_eq!(px(&vm(json!({ "enabled": false })), 0, 0), [255, 0, 0, 255]);
}

#[test]
fn raster_and_vector_masks_combine() {
    let layer = patch(full("a", RED), json!({ "mask": { "asset": "m", "offset": { "x": 2, "y": 2 } }, "vectorMask": { "path": { "d": rect_path(0.0, 0.0, 8.0, 4.0) } } }));
    let r = scene(8, 8, vec![layer], &[("m", half())]);
    assert_eq!(px(&r, 3, 3), [255, 0, 0, 255]);
    assert_eq!(px(&r, 3, 5)[3], 0); // raster reveals, vector hides
    assert_eq!(px(&r, 5, 3)[3], 0); // vector reveals, raster hides
}

#[test]
fn masks_work_on_groups_isolated_or_pass_through() {
    for pass_through in [true, false] {
        let g = patch(group("g", pass_through, vec![multiply_child()]), json!({ "vectorMask": { "path": { "d": rect_path(0.0, 0.0, 4.0, 8.0) } } }));
        let r = render8(vec![full("bg", WHITE), g]);
        near(px(&r, 6, 4), [255, 255, 255, 255]);
        near(px(&r, 1, 4), [128, 128, 128, 255]);
    }
}

#[test]
fn clipped_layers_respect_the_bases_mask() {
    let base = patch(full("base", RED), json!({ "vectorMask": { "path": { "d": rect_path(0.0, 0.0, 4.0, 8.0) } } }));
    let r = render8(vec![base, clip(full("clip", BLUE))]);
    assert_eq!(px(&r, 1, 4), [0, 0, 255, 255]);
    assert_eq!(px(&r, 6, 4)[3], 0);
}

// ---- samples ------------------------------------------------------------------

#[test]
fn the_sample_documents_render_without_warnings_and_survive_a_save() {
    use cardsmith_core::render::render;
    use cardsmith_core::serialize::{load_cardsmith, save_cardsmith};
    for sample in cardsmith_core::samples::samples() {
        let direct = render(&sample.doc, &sample.assets);
        assert!(direct.warnings.is_empty(), "{}: {:?}", sample.name, direct.warnings);
        assert!(direct.pixels.iter().any(|&v| v != 0));
        let (doc, assets) = load_cardsmith(&save_cardsmith(&sample.doc, &sample.assets).unwrap()).unwrap();
        assert_eq!(render(&doc, &assets).pixels, direct.pixels, "{} changed across save and load", sample.name);
    }
}

// ---- region rendering -----------------------------------------------------------

/// Rendering part of a document must give exactly the pixels a full render has
/// there. Incremental repainting (brush strokes, edits) depends on it.
#[test]
fn a_region_render_equals_the_same_crop_of_a_full_render() {
    use cardsmith_core::raster::Region;
    use cardsmith_core::render::Renderer;
    for sample in cardsmith_core::samples::samples() {
        let mut renderer = Renderer::new();
        let full = renderer.render(&sample.doc, &sample.assets);
        let (w, h) = (full.width as i32, full.height as i32);
        // Regions chosen to cut through shapes, effects, masks, groups and the document edge.
        let regions = [
            Region::new(0, 0, 64, 64), Region::new(30, 20, 50, 40), Region::new(100, 90, 120, 110), Region::new(213, 187, 33, 71),
            Region::new(w - 70, h - 60, 70, 60), Region::new(120, 105, 3, 3), Region::new(0, 100, w, 1), Region::new(205, 0, 1, h),
            Region::new(-20, -20, 60, 60), Region::new(w - 10, 40, 50, 50),
        ];
        for region in regions {
            let (part, _) = renderer.render_region(&sample.doc, &sample.assets, region);
            assert!(part.region.w > 0 && part.region.h > 0);
            for y in part.region.y..part.region.y + part.region.h {
                for x in part.region.x..part.region.x + part.region.w {
                    let i = ((y * w + x) * 4) as usize;
                    assert_eq!(part.pixel(x, y), full.pixels[i..i + 4], "{} differs at ({x},{y}) in region {region:?}", sample.name);
                }
            }
        }
    }
}
