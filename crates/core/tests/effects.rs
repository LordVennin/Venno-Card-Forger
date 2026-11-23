//! Ported from packages/render/src/effects.test.ts.

mod common;
use cardsmith_core::doc::build::{group, rect_path};
use cardsmith_core::doc::*;
use cardsmith_core::render::RenderResult;
use common::*;
use serde_json::{json, Value};

const G: [u8; 4] = [128, 128, 128, 255];

/// Shared effect fields, overridden by `own`.
fn fx(own: Value) -> Value {
    let mut merged = json!({ "enabled": true, "blend": "normal", "opacity": 1 }).as_object().unwrap().clone();
    merged.extend(own.as_object().unwrap().clone());
    Value::Object(merged)
}

/// A 40×40 document with a gray 20×20 square at (10,10): edges at 10 and 30.
fn square_with(effects: Value, extra: Value, under: Vec<Layer>) -> RenderResult {
    let mut layer = patch(rect("sq", 10.0, 10.0, 20.0, 20.0, GRAY), json!({ "effects": effects }));
    layer = patch(layer, extra);
    let mut layers = under;
    layers.push(layer);
    scene(40, 40, layers, &[])
}
fn square(effects: Value) -> RenderResult {
    square_with(effects, json!({}), vec![])
}
fn backdrop(color: Color) -> Layer {
    rect("bg", 0.0, 0.0, 40.0, 40.0, color)
}

// ---- color overlay ------------------------------------------------------------

#[test]
fn color_overlay_recolors_the_layer_and_keeps_its_outline() {
    let r = square(json!([fx(json!({ "type": "colorOverlay", "color": [1, 0, 0, 1] }))]));
    assert_eq!(px(&r, 20, 20), [255, 0, 0, 255]);
    assert_eq!(px(&r, 5, 5), CLEAR);
}

#[test]
fn color_overlay_shows_at_fill_zero_and_blends_with_the_backdrop() {
    let r = square_with(json!([fx(json!({ "type": "colorOverlay", "color": [0.5, 0.5, 0.5, 1], "blend": "multiply" }))]), json!({ "fillOpacity": 0 }), vec![backdrop(RED)]);
    near(px(&r, 20, 20), [128, 0, 0, 255]);
    assert_eq!(px(&r, 5, 5), [255, 0, 0, 255]);
}

#[test]
fn color_overlay_respects_its_own_opacity() {
    let r = square(json!([fx(json!({ "type": "colorOverlay", "color": [1, 1, 1, 1], "opacity": 0.5 }))]));
    near(px(&r, 20, 20), [191, 191, 191, 255]);
}

// ---- gradient overlay ---------------------------------------------------------

fn gradient(extra: Value) -> RenderResult {
    let mut effect = fx(json!({
        "type": "gradientOverlay", "style": "linear", "angle": 0, "scale": 1,
        "gradient": { "stops": [{ "offset": 0, "color": [1, 0, 0, 1] }, { "offset": 1, "color": [0, 0, 1, 1] }] }
    }));
    effect.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    square(json!([effect]))
}

#[test]
fn gradient_runs_along_its_angle_across_the_layers_bounds() {
    let right = gradient(json!({ "angle": 0 }));
    assert!(px(&right, 10, 20)[0] > 240); // starts red on the left
    assert!(px(&right, 29, 20)[2] > 240); // ends blue on the right
    near(px(&right, 20, 12), px(&right, 20, 27)); // constant across the gradient
    let up = gradient(json!({ "angle": 90 }));
    assert!(px(&up, 20, 29)[0] > 240); // 90° points up: starts at the bottom
    assert!(px(&up, 20, 10)[2] > 240);
}

#[test]
fn gradient_reverses_scales_and_stays_inside_the_layer() {
    let reversed = gradient(json!({ "reverse": true }));
    assert!(px(&reversed, 10, 20)[2] > 240);
    assert!(px(&reversed, 29, 20)[0] > 240);
    let squeezed = gradient(json!({ "scale": 0.2 })); // spans x = 18..22
    assert_eq!(px(&squeezed, 14, 20), [255, 0, 0, 255]);
    assert_eq!(px(&squeezed, 26, 20), [0, 0, 255, 255]);
    assert_eq!(px(&gradient(json!({})), 5, 20), CLEAR);
}

#[test]
fn gradient_offset_shifts_the_center_by_a_fraction_of_the_bounds() {
    // The square is 20 px wide; a quarter of that moves the midpoint 5 px to the right.
    let plain = gradient(json!({}));
    let shifted = gradient(json!({ "offset": { "x": 0.25, "y": 0 } }));
    near(px(&shifted, 25, 20), px(&plain, 20, 20));
    near(px(&shifted, 20, 20), px(&plain, 15, 20));
}

#[test]
fn radial_gradient_starts_at_the_center() {
    let r = gradient(json!({ "style": "radial" }));
    assert!(px(&r, 20, 20)[0] > 230);
    assert!(px(&r, 10, 20)[2] > 230);
    near_by(px(&r, 20, 11), px(&r, 11, 20), 3);
}

// ---- stroke -------------------------------------------------------------------

fn stroke(position: &str, size: f64, extra: Value) -> RenderResult {
    square_with(json!([fx(json!({ "type": "stroke", "color": [1, 0, 0, 1], "size": size, "position": position }))]), extra, vec![])
}
const R: [u8; 4] = [255, 0, 0, 255];

#[test]
fn stroke_outside_is_a_band_around_the_layer_with_round_corners() {
    let r = stroke("outside", 3.0, json!({}));
    assert_eq!(px(&r, 9, 20), R);
    assert_eq!(px(&r, 7, 20), R);
    assert_eq!(px(&r, 6, 20), CLEAR);
    assert_eq!(px(&r, 10, 20), G);
    let wide = stroke("outside", 6.0, json!({}));
    assert_eq!(px(&wide, 4, 20), R); // 6 px straight out from the edge
    assert_eq!(px(&wide, 5, 5)[3], 0); // the corner of the bounding box is beyond the radius
    assert!(px(&wide, 6, 6)[3] > 200); // but just inside the arc is painted
}

#[test]
fn stroke_inside_and_center() {
    let inside = stroke("inside", 3.0, json!({}));
    assert_eq!(px(&inside, 9, 20), CLEAR);
    assert_eq!(px(&inside, 10, 20), R);
    assert_eq!(px(&inside, 12, 20), R);
    assert_eq!(px(&inside, 13, 20), G);
    let center = stroke("center", 4.0, json!({}));
    assert_eq!(px(&center, 8, 20), R);
    assert_eq!(px(&center, 11, 20), R);
    assert_eq!(px(&center, 7, 20), CLEAR);
    assert_eq!(px(&center, 12, 20), G);
}

#[test]
fn stroke_is_not_faded_by_fill_opacity() {
    let r = stroke("outside", 3.0, json!({ "fillOpacity": 0 }));
    assert_eq!(px(&r, 8, 20), R);
    assert_eq!(px(&r, 20, 20), CLEAR);
}

#[test]
fn the_first_of_two_strokes_is_on_top() {
    let r = square(json!([
        fx(json!({ "type": "stroke", "color": [1, 0, 0, 1], "size": 2, "position": "outside" })),
        fx(json!({ "type": "stroke", "color": [0, 0, 1, 1], "size": 5, "position": "outside" })),
    ]));
    assert_eq!(px(&r, 9, 20), R);
    assert_eq!(px(&r, 6, 20), [0, 0, 255, 255]);
}

#[test]
fn stroke_follows_the_masked_outline_and_works_at_the_document_edge() {
    let r = stroke("outside", 2.0, json!({ "vectorMask": { "path": { "d": rect_path(0.0, 0.0, 20.0, 40.0) } } }));
    assert_eq!(px(&r, 20, 20), R); // the mask's cut edge gets a stroke
    assert_eq!(px(&r, 25, 20), CLEAR);

    let edge = patch(rect("edge", 0.0, 0.0, 40.0, 10.0, GRAY), json!({ "effects": [fx(json!({ "type": "stroke", "color": [1, 0, 0, 1], "size": 3, "position": "outside" }))] }));
    let r = scene(40, 40, vec![edge], &[]);
    assert_eq!(px(&r, 20, 11), R);
    assert_eq!(px(&r, 0, 0), G);
}

// ---- drop shadow --------------------------------------------------------------

fn shadow(extra: Value, layer: Value) -> RenderResult {
    let mut effect = fx(json!({ "type": "dropShadow", "color": [0, 0, 0, 1], "angle": 90, "distance": 4, "spread": 0, "size": 0 }));
    effect.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    square_with(json!([effect]), layer, vec![])
}
const K: [u8; 4] = [0, 0, 0, 255];

#[test]
fn drop_shadow_falls_away_from_the_light_by_distance() {
    let r = shadow(json!({}), json!({})); // light from above, so the shadow drops down
    assert_eq!(px(&r, 20, 32), K);
    assert_eq!(px(&r, 20, 33), K);
    assert_eq!(px(&r, 20, 34), CLEAR);
    assert_eq!(px(&r, 20, 8), CLEAR);
    assert_eq!(px(&r, 8, 20), CLEAR);
    let side = shadow(json!({ "angle": 180 }), json!({})); // light from the left
    assert_eq!(px(&side, 32, 20), K);
    assert_eq!(px(&side, 8, 20), CLEAR);
}

#[test]
fn drop_shadow_is_knocked_out_by_the_layer_even_at_fill_zero() {
    let r = shadow(json!({}), json!({ "fillOpacity": 0 }));
    assert_eq!(px(&r, 20, 25), CLEAR);
    assert_eq!(px(&r, 20, 32), K);
}

#[test]
fn drop_shadow_softens_with_size_and_hardens_with_spread() {
    let soft = shadow(json!({ "distance": 0, "size": 8 }), json!({}));
    let a: Vec<u8> = [31, 33, 35, 37].iter().map(|&y| px(&soft, 20, y)[3]).collect();
    assert!(a[0] > a[1] && a[1] > a[2] && a[2] > a[3] && a[0] < 160, "{a:?}");
    let hard = shadow(json!({ "distance": 0, "size": 6, "spread": 1 }), json!({}));
    assert_eq!(px(&hard, 20, 34), K);
    assert_eq!(px(&hard, 20, 37), CLEAR);
}

#[test]
fn the_gaussian_contour_thins_the_outer_falloff() {
    let linear = shadow(json!({ "distance": 0, "size": 10 }), json!({}));
    let curved = shadow(json!({ "distance": 0, "size": 10, "contour": "gaussian" }), json!({}));
    // Outside the layer the blurred matte is below one half, where the S-curve pulls values down.
    for y in [31, 33, 35] {
        let (lin, cur) = (px(&linear, 20, y)[3], px(&curved, 20, y)[3]);
        assert!(lin > 0 && cur < lin, "y={y}: linear {lin}, gaussian {cur}");
    }
}

#[test]
fn drop_shadow_blends_through_its_own_mode_and_opacity() {
    let r = square_with(
        json!([fx(json!({ "type": "dropShadow", "color": [0.5, 0.5, 0.5, 1], "blend": "multiply", "opacity": 0.5, "angle": 90, "distance": 4, "spread": 0, "size": 0 }))]),
        json!({}),
        vec![backdrop(RED)],
    );
    near(px(&r, 20, 32), [191, 0, 0, 255]);
}

// ---- outer glow, inner shadow ---------------------------------------------------

#[test]
fn outer_glow_surrounds_the_layer_evenly() {
    let r = square(json!([fx(json!({ "type": "outerGlow", "color": [0, 1, 0, 1], "spread": 0, "size": 8 }))]));
    let (left, top, right, bottom) = (px(&r, 7, 20), px(&r, 20, 7), px(&r, 32, 20), px(&r, 20, 32));
    assert!(left[3] > 20);
    near(left, top);
    near(left, right);
    near(left, bottom);
    assert!(px(&r, 1, 20)[3] < left[3]);
    assert_eq!(px(&r, 20, 20), G);
}

fn inner_shadow(extra: Value) -> RenderResult {
    let mut effect = fx(json!({ "type": "innerShadow", "color": [0, 0, 0, 1], "angle": 90, "distance": 3, "choke": 0, "size": 0 }));
    effect.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    square(json!([effect]))
}

#[test]
fn inner_shadow_darkens_the_inside_edge_nearest_the_light() {
    let r = inner_shadow(json!({}));
    assert_eq!(px(&r, 20, 10), K);
    assert_eq!(px(&r, 20, 12), K);
    assert_eq!(px(&r, 20, 13), G);
    assert_eq!(px(&r, 20, 29), G);
    assert_eq!(px(&r, 20, 9), CLEAR); // never outside the layer
}

#[test]
fn inner_shadow_softens_with_size_and_rings_the_edge_with_choke() {
    let soft = inner_shadow(json!({ "distance": 0, "size": 6 }));
    assert!(px(&soft, 20, 10)[0] < px(&soft, 20, 13)[0]);
    assert_eq!(px(&soft, 20, 20), G);
    let ring = inner_shadow(json!({ "distance": 0, "size": 4, "choke": 1 }));
    for (x, y) in [(20, 11), (20, 28), (11, 20), (28, 20)] {
        assert_eq!(px(&ring, x, y), K);
    }
    assert_eq!(px(&ring, 20, 15), G);
}

// ---- bevel --------------------------------------------------------------------

fn bevel(extra: Value) -> RenderResult {
    let mut effect = json!({
        "type": "bevel", "enabled": true, "style": "inner", "technique": "chiselHard", "direction": "up", "depth": 1, "size": 4, "soften": 0, "angle": 90, "altitude": 30,
        "highlight": { "color": [1, 1, 1, 1], "blend": "normal", "opacity": 1 },
        "shadow": { "color": [0, 0, 0, 1], "blend": "normal", "opacity": 1 }
    });
    effect.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    square(json!([effect]))
}

#[test]
fn inner_bevel_lights_the_edge_facing_the_light_and_shades_the_opposite() {
    let r = bevel(json!({}));
    assert!(px(&r, 20, 11)[0] > 200); // top edge faces the light
    assert!(px(&r, 20, 28)[0] < 60); // bottom edge faces away
    assert_eq!(px(&r, 20, 20), G); // flat top is untouched
    assert_eq!(px(&r, 20, 8), CLEAR);
    // Flat-shaded: constant along a straight chiselled face.
    assert_eq!(px(&r, 16, 11), px(&r, 24, 11));
    assert_eq!(px(&r, 20, 11), px(&r, 20, 12));
    assert_eq!(px(&r, 20, 14), G); // just past the 4 px face
}

#[test]
fn bevel_direction_and_light_angle() {
    let down = bevel(json!({ "direction": "down" }));
    assert!(px(&down, 20, 11)[0] < 60);
    assert!(px(&down, 20, 28)[0] > 200);
    let left = bevel(json!({ "angle": 180 })); // light from the left
    assert!(px(&left, 11, 20)[0] > 200);
    assert!(px(&left, 28, 20)[0] < 60);
}

#[test]
fn outer_bevel_shades_the_band_outside_and_leaves_the_layer_alone() {
    let r = bevel(json!({ "style": "outer" }));
    let top = px(&r, 20, 8);
    assert_eq!(&top[..3], [255, 255, 255]);
    assert!(top[3] > 200);
    let bottom = px(&r, 20, 31);
    assert!(bottom[3] > 200 && bottom[0] == 0);
    assert_eq!(px(&r, 20, 11), G);
    assert_eq!(px(&r, 20, 3), CLEAR);
}

#[test]
fn a_bevel_profile_reshapes_the_cross_section() {
    // A hump: the surface rises over the outer half of the bevel and falls over the inner half.
    // Lit from above, the top face is bright at its outer edge and dark at its inner edge,
    // and the bottom face is the other way round.
    let r = bevel(json!({ "size": 8, "depth": 3, "profile": [[0, 0], [0.5, 0.1], [1, 0]] }));
    let top: Vec<u8> = (10..18).map(|y| px(&r, 20, y)[0]).collect();
    assert!(top[..3].iter().all(|&v| v > 200) && top[5..].iter().all(|&v| v < 60), "bright outer part, dark inner part: {top:?}");
    let bottom: Vec<u8> = (22..30).map(|y| px(&r, 20, y)[0]).collect();
    assert!(bottom[..3].iter().all(|&v| v > 200) && bottom[5..].iter().all(|&v| v < 60), "bright inner part, dark outer part: {bottom:?}");
    assert_eq!(px(&r, 20, 20), G);
    // A straight-line profile is the default ramp.
    assert_eq!(bevel(json!({ "profile": [[0, 0], [1, 1]] })).pixels, bevel(json!({})).pixels);
}

#[test]
fn bevel_soften_blurs_the_facets() {
    assert!(px(&bevel(json!({ "soften": 6 })), 20, 14)[0] > 128);
}

// ---- switches, opacity, groups --------------------------------------------------

fn overlay() -> Value {
    fx(json!({ "type": "colorOverlay", "color": [1, 0, 0, 1] }))
}

#[test]
fn disabled_effects_and_a_switched_off_layer_style_are_ignored() {
    let mut off = overlay();
    off["enabled"] = json!(false);
    assert_eq!(px(&square(json!([off])), 20, 20), G);
    assert_eq!(px(&square_with(json!([overlay()]), json!({ "effectsEnabled": false }), vec![]), 20, 20), G);
}

#[test]
fn layer_opacity_fades_the_layer_and_its_effects_together() {
    let r = square_with(
        json!([overlay(), fx(json!({ "type": "stroke", "color": [0, 0, 1, 1], "size": 3, "position": "outside" }))]),
        json!({ "opacity": 0.5 }),
        vec![backdrop(WHITE)],
    );
    near(px(&r, 20, 20), [255, 128, 128, 255]);
    near(px(&r, 8, 20), [128, 128, 255, 255]);
    assert_eq!(px(&r, 2, 2), [255, 255, 255, 255]);
}

#[test]
fn effects_work_on_groups_using_the_merged_outline() {
    let g = patch(
        group("g", true, vec![rect("a", 10.0, 10.0, 10.0, 20.0, GRAY), rect("b", 20.0, 10.0, 10.0, 20.0, WHITE)]),
        json!({ "effects": [fx(json!({ "type": "stroke", "color": [1, 0, 0, 1], "size": 2, "position": "outside" }))] }),
    );
    let r = scene(40, 40, vec![g], &[]);
    assert_eq!(px(&r, 9, 20), R);
    assert_eq!(px(&r, 20, 20), [255, 255, 255, 255]); // no stroke on the seam between the children
    assert_eq!(px(&r, 19, 20), G);
}

#[test]
fn an_empty_layer_with_effects_draws_nothing() {
    let layer = patch(rect("none", 50.0, 50.0, 5.0, 5.0, GRAY), json!({ "effects": [overlay()] }));
    assert!(scene(40, 40, vec![layer], &[]).pixels.iter().all(|&v| v == 0));
}

#[test]
fn clipped_layers_sit_under_the_bases_overlay_effects() {
    let mut half = overlay();
    half["opacity"] = json!(0.5);
    let base = patch(rect("sq", 10.0, 10.0, 20.0, 20.0, GRAY), json!({ "effects": [half] }));
    let clip = patch(rect("c", 0.0, 0.0, 40.0, 40.0, BLUE), json!({ "clip": true }));
    let r = scene(40, 40, vec![base.clone(), clip], &[]);
    near(px(&r, 20, 20), [128, 0, 128, 255]); // blue from the clipped layer, then 50% red on top
    near(px(&scene(40, 40, vec![base], &[]), 20, 20), [191, 64, 64, 255]);
    assert_eq!(px(&r, 5, 5), CLEAR);
}
