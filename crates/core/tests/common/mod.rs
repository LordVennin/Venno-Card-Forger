//! Helpers for building tiny documents and reading pixels in tests.
#![allow(dead_code)]

use cardsmith_core::doc::build::{rect_path, shape};
use cardsmith_core::doc::*;
use cardsmith_core::render::{encode_png, render, RenderResult};
use cardsmith_core::serialize::AssetBytes;
use serde_json::Value;

pub const RED: Color = [1.0, 0.0, 0.0, 1.0];
pub const GREEN: Color = [0.0, 1.0, 0.0, 1.0];
pub const BLUE: Color = [0.0, 0.0, 1.0, 1.0];
pub const WHITE: Color = [1.0, 1.0, 1.0, 1.0];
pub const BLACK: Color = [0.0, 0.0, 0.0, 1.0];
pub const GRAY: Color = [0.5, 0.5, 0.5, 1.0];
pub const CLEAR: [u8; 4] = [0, 0, 0, 0];

/// PNG bytes from a function giving straight-alpha RGBA per pixel.
pub fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let data: Vec<u8> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).flat_map(|(x, y)| f(x, y)).collect();
    encode_png(w, h, &data).unwrap()
}

/// Render a list of layers (bottom first) with optional image assets.
pub fn scene(width: u32, height: u32, layers: Vec<Layer>, images: &[(&str, Vec<u8>)]) -> RenderResult {
    let mut doc = Doc::empty(width, height, 72.0);
    doc.root.children = layers;
    let mut assets = AssetBytes::new();
    for (id, bytes) in images {
        doc.assets.insert(id.to_string(), AssetRef { kind: AssetKind::Image, path: format!("assets/{id}.png"), width: None, height: None });
        assets.insert(id.to_string(), bytes.clone());
    }
    render(&doc, &assets)
}

pub fn px(r: &RenderResult, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * r.width + x) * 4) as usize;
    [r.pixels[i], r.pixels[i + 1], r.pixels[i + 2], r.pixels[i + 3]]
}

/// Compare with a tolerance per channel.
#[track_caller]
pub fn near_by(actual: [u8; 4], expected: [u8; 4], tolerance: u8) {
    assert!(actual.iter().zip(&expected).all(|(a, e)| a.abs_diff(*e) <= tolerance), "got {actual:?}, want {expected:?} (±{tolerance})");
}

/// Compare with a tolerance of one 8-bit step per channel.
#[track_caller]
pub fn near(actual: [u8; 4], expected: [u8; 4]) {
    near_by(actual, expected, 1);
}

/// A rectangle shape layer.
pub fn rect(id: &str, x: f64, y: f64, w: f64, h: f64, color: Color) -> Layer {
    shape(id, &rect_path(x, y, w, h), color)
}

/// Patch a layer through its JSON form: `patch(layer, json!({ "opacity": 0.5 }))`.
pub fn patch(layer: Layer, props: Value) -> Layer {
    let mut value = serde_json::to_value(&layer).unwrap();
    value.as_object_mut().unwrap().extend(props.as_object().unwrap().clone());
    serde_json::from_value(value).expect("the patched layer is valid")
}
