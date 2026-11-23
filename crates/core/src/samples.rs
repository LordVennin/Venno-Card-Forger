//! Hand-written sample documents. They exercise every feature the renderer
//! supports and are what the tests, the CLI and the app's start screen render.

use crate::doc::build::{group, pixel, rect_path, shape, with};
use crate::doc::*;
use crate::render::encode_png;
use crate::serialize::AssetBytes;
use serde_json::json;

pub struct Sample {
    pub name: &'static str,
    pub doc: Doc,
    pub assets: AssetBytes,
}

fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            data.extend_from_slice(&f(x, y));
        }
    }
    encode_png(w, h, &data).expect("encoding a generated image cannot fail")
}

fn round_rect(x: f64, y: f64, w: f64, h: f64, r: f64) -> String {
    format!(
        "M{} {y}H{}A{r} {r} 0 0 1 {} {}V{}A{r} {r} 0 0 1 {} {}H{}A{r} {r} 0 0 1 {x} {}V{}A{r} {r} 0 0 1 {} {y}Z",
        x + r, x + w - r, x + w, y + r, y + h - r, x + w - r, y + h, x + r, y + h - r, y + r, x + r
    )
}

fn circle(cx: f64, cy: f64, r: f64) -> String {
    format!("M{} {cy}A{r} {r} 0 1 0 {} {cy}A{r} {r} 0 1 0 {} {cy}Z", cx - r, cx + r, cx - r)
}

/// Effects are easiest to read as the JSON they are stored as.
fn effects(value: serde_json::Value) -> Option<Vec<Effect>> {
    Some(serde_json::from_value(value).expect("sample effects are valid"))
}

fn compositing() -> Sample {
    let (w, h) = (420u32, 300u32);
    let mut doc = Doc::empty(w, h, 72.0);
    let mut assets = AssetBytes::new();
    // A colorful backdrop so blend modes have something to work on.
    assets.insert("backdrop".into(), image(w, h, |x, y| [(x as f32 / w as f32 * 255.0).round() as u8, (y as f32 / h as f32 * 255.0).round() as u8, 160, 255]));
    // Checkerboard with transparent squares.
    assets.insert("checker".into(), image(64, 64, |x, y| if ((x >> 3) + (y >> 3)) % 2 == 1 { [250, 210, 60, 255] } else { [0, 0, 0, 0] }));
    // Horizontal ramp, used as a raster mask.
    assets.insert("ramp".into(), image(100, 60, |x, _| { let v = (x as f32 / 99.0 * 255.0).round() as u8; [v, v, v, 255] }));
    for id in assets.keys() {
        doc.assets.insert(id.clone(), AssetRef { kind: AssetKind::Image, path: format!("assets/{id}.png"), width: None, height: None });
    }

    let blends = [
        ("normal", BlendMode::Normal), ("multiply", BlendMode::Multiply), ("screen", BlendMode::Screen), ("overlay", BlendMode::Overlay),
        ("softLight", BlendMode::SoftLight), ("linearLight", BlendMode::LinearLight), ("luminosity", BlendMode::Luminosity),
    ];
    let swatches = blends
        .iter()
        .enumerate()
        .map(|(i, (name, blend))| with(shape(&format!("blend-{name}"), &round_rect(12.0 + i as f64 * 58.0, 14.0, 50.0, 70.0, 8.0), [0.85, 0.45, 0.2, 1.0]), |b| b.blend = *blend))
        .collect();
    let orange: Color = [0.95, 0.5, 0.1, 1.0];
    let mask = |x: f64, inverted: bool| Some(RasterMask { asset: "ramp".into(), offset: Vec2 { x, y: 110.0 }, outside: None, enabled: None, inverted: inverted.then_some(true), density: None, feather: None });
    let vector = |d: String, inverted: bool| Some(VectorMask { path: Path { d, fill_rule: None }, enabled: None, inverted: inverted.then_some(true) });

    let mut layers = vec![
        pixel("backdrop", "backdrop", IDENTITY),
        with(group("swatches", true, swatches), |b| b.name = "Blend modes".into()),
        // Clipping: two layers clipped to a circle, one with a blend mode.
        shape("clip-base", &circle(60.0, 150.0, 42.0), [0.2, 0.2, 0.25, 0.85]),
        with(pixel("clip-checker", "checker", [1.5, 0.0, 12.0, 0.0, 1.5, 102.0]), |b| b.clip = true),
        with(shape("clip-tint", &rect_path(18.0, 150.0, 84.0, 42.0), [0.3, 0.7, 1.0, 1.0]), |b| {
            b.clip = true;
            b.blend = BlendMode::Overlay;
        }),
        // Masks: raster ramp, vector circle, and both with the vector one inverted.
        with(shape("mask-raster", &rect_path(120.0, 110.0, 100.0, 60.0), orange), |b| b.mask = mask(120.0, false)),
        with(shape("mask-vector", &rect_path(230.0, 110.0, 80.0, 60.0), orange), |b| b.vector_mask = vector(circle(270.0, 140.0, 36.0), false)),
        with(shape("mask-both", &rect_path(320.0, 110.0, 90.0, 60.0), orange), |b| {
            b.mask = mask(315.0, true);
            b.vector_mask = vector(circle(365.0, 140.0, 16.0), true);
        }),
    ];
    // Groups: the same multiply child, pass-through, isolated, and pass-through at 50%.
    for (i, (name, pass_through, opacity)) in [("pass", true, 1.0), ("isolated", false, 1.0), ("faded", true, 0.5)].into_iter().enumerate() {
        let dx = i as f64 * 100.0;
        layers.push(with(
            group(
                &format!("group-{name}"),
                pass_through,
                vec![
                    shape(&format!("group-{name}-a"), &round_rect(120.0 + dx, 190.0, 60.0, 60.0, 10.0), [0.9, 0.9, 0.3, 1.0]),
                    with(shape(&format!("group-{name}-b"), &circle(170.0 + dx, 240.0, 30.0), [0.4, 0.8, 0.9, 1.0]), |b| b.blend = BlendMode::Multiply),
                ],
            ),
            |b| b.opacity = opacity,
        ));
    }
    let mut stroked = shape("stroked-shape", &round_rect(20.0, 215.0, 80.0, 60.0, 14.0), [1.0, 1.0, 1.0, 0.6]);
    if let Layer::Shape(s) = &mut stroked {
        s.stroke = Some(ShapeStroke { color: [0.1, 0.1, 0.3, 1.0], width: 5.0, align: StrokeAlign::Inside });
        s.base.fill_opacity = Some(0.7);
    }
    layers.push(stroked);
    doc.root.children = layers;
    Sample { name: "compositing", doc, assets }
}

fn effects_sample() -> Sample {
    let (w, h) = (420u32, 300u32);
    let mut doc = Doc::empty(w, h, 72.0);
    let gold: Color = [0.84, 0.67, 0.32, 1.0];
    let ink = json!([0.05, 0.05, 0.1, 1]);
    let black = json!([0, 0, 0, 1]);
    let rainbow = json!({ "stops": [{ "offset": 0, "color": [0.9, 0.2, 0.2, 1] }, { "offset": 0.5, "color": [0.95, 0.9, 0.3, 1] }, { "offset": 1, "color": [0.2, 0.4, 0.9, 1] }] });
    let bevel = json!({
        "type": "bevel", "enabled": true, "style": "inner", "technique": "chiselHard", "direction": "up", "depth": 1, "size": 8, "soften": 0, "angle": 120, "altitude": 30,
        "highlight": { "color": [1, 1, 1, 1], "blend": "screen", "opacity": 0.75 },
        "shadow": { "color": [0, 0, 0, 1], "blend": "multiply", "opacity": 0.75 }
    });
    let mut outer_bevel = bevel.clone();
    outer_bevel["style"] = json!("outer");
    outer_bevel["size"] = json!(6);
    outer_bevel["soften"] = json!(2);

    let tile = |i: usize, id: &str, fx: serde_json::Value| {
        let (x, y) = (20.0 + (i % 4) as f64 * 100.0, 20.0 + (i / 4) as f64 * 95.0);
        with(shape(id, &round_rect(x, y, 78.0, 68.0, 16.0), gold), |b| b.effects = effects(fx))
    };
    let on = json!({ "enabled": true, "blend": "normal", "opacity": 1 });
    // Merge the shared "enabled / blend / opacity" fields under each effect's own.
    let fx = |own: serde_json::Value| {
        let mut merged = on.as_object().unwrap().clone();
        merged.extend(own.as_object().unwrap().clone());
        serde_json::Value::Object(merged)
    };

    doc.root.children = vec![
        shape("paper", &rect_path(0.0, 0.0, w as f64, h as f64), [0.93, 0.92, 0.88, 1.0]),
        tile(0, "stroke-outside", json!([fx(json!({ "type": "stroke", "color": ink, "size": 5, "position": "outside" }))])),
        tile(1, "stroke-inside-center", json!([
            fx(json!({ "type": "stroke", "color": ink, "size": 4, "position": "inside" })),
            fx(json!({ "type": "stroke", "color": [1, 1, 1, 1], "size": 10, "position": "center" })),
        ])),
        tile(2, "drop-shadow", json!([fx(json!({ "type": "dropShadow", "color": black, "blend": "multiply", "opacity": 0.7, "angle": 120, "distance": 7, "spread": 0.2, "size": 10 }))])),
        tile(3, "outer-glow", json!([fx(json!({ "type": "outerGlow", "color": [0.1, 0.3, 0.9, 1], "opacity": 0.9, "spread": 0.1, "size": 12 }))])),
        tile(4, "inner-shadow", json!([fx(json!({ "type": "innerShadow", "color": black, "blend": "multiply", "opacity": 0.7, "angle": 120, "distance": 5, "choke": 0, "size": 8 }))])),
        tile(5, "bevel-inner", json!([bevel])),
        tile(6, "bevel-outer", json!([outer_bevel])),
        tile(7, "color-overlay", json!([fx(json!({ "type": "colorOverlay", "color": [0.3, 0.6, 0.4, 1], "blend": "multiply" }))])),
        tile(8, "gradient-linear", json!([fx(json!({ "type": "gradientOverlay", "gradient": rainbow, "style": "linear", "angle": 45, "scale": 1 }))])),
        tile(9, "gradient-radial", json!([fx(json!({ "type": "gradientOverlay", "gradient": rainbow, "style": "radial", "angle": 0, "scale": 1, "blend": "luminosity" }))])),
        with(
            tile(10, "fill-zero", json!([
                fx(json!({ "type": "stroke", "color": ink, "size": 3, "position": "inside" })),
                fx(json!({ "type": "dropShadow", "color": black, "opacity": 0.6, "angle": 120, "distance": 5, "spread": 0, "size": 6 })),
            ])),
            |b| b.fill_opacity = Some(0.0),
        ),
        with(
            tile(11, "everything", json!([
                fx(json!({ "type": "dropShadow", "color": black, "blend": "multiply", "opacity": 0.6, "angle": 120, "distance": 6, "spread": 0, "size": 8 })),
                fx(json!({ "type": "gradientOverlay", "gradient": rainbow, "style": "linear", "angle": 90, "scale": 1, "opacity": 0.6 })),
                bevel,
                fx(json!({ "type": "stroke", "color": ink, "size": 3, "position": "outside" })),
            ])),
            |b| b.opacity = 0.9,
        ),
    ];
    Sample { name: "effects", doc, assets: AssetBytes::new() }
}

pub fn samples() -> Vec<Sample> {
    vec![compositing(), effects_sample()]
}
