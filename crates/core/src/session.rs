//! An open document: the model, its undo history, its pixels and what is on screen.
//!
//! The session is the one place that mutates a document. Every change, whether
//! a structural edit or a brush stroke, goes through it, is undoable, and
//! leaves the rendered frame up to date by redrawing only what changed.

use crate::commands::{apply, Edit};
use crate::doc::build::pixel;
use crate::doc::{AssetKind, AssetRef, Doc, GroupLayer, Layer};
use crate::effects::effect_extent;
use crate::paint::{crop, paste, Brush, Stroke};
use crate::raster::{Image, Raster, Region};
use crate::render::{encode_png, hash, Renderer};
use crate::serialize::{save_cardsmith, AssetBytes};
use crate::{err, Result};
use std::collections::BTreeSet;

enum Step {
    Edit { label: String, edit: Edit, inverse: Edit },
    /// A brush stroke: the pixels of one image region before and after it.
    Paint { label: String, asset: String, offset: (i32, i32), region: Region, before: Vec<u8>, after: Vec<u8>, margin: i32 },
}

impl Step {
    fn label(&self) -> &str {
        match self {
            Step::Edit { label, .. } | Step::Paint { label, .. } => label,
        }
    }
}

struct ActiveStroke {
    stroke: Stroke,
    asset: String,
    /// Document position of the image's top-left pixel.
    offset: (i32, i32),
    /// How far beyond the painted pixels the document can change (layer effects).
    margin: i32,
}

/// The part of the document shown on screen: `scale` screen pixels per
/// document pixel, with document point (x, y) at the view's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub scale: f32,
    pub x: f32,
    pub y: f32,
    pub w: u32,
    pub h: u32,
}

pub struct Session {
    doc: Doc,
    assets: AssetBytes,
    renderer: Renderer,
    done: Vec<Step>,
    undone: Vec<Step>,
    frame: Raster,
    warnings: Vec<String>,
    view: View,
    view_pixels: Vec<u8>,
    /// View-space area changed by the last call that redrew anything.
    dirty_view: Region,
    stroke: Option<ActiveStroke>,
    /// Image assets whose pixels differ from their encoded bytes.
    painted: BTreeSet<String>,
}

const HISTORY_LIMIT: usize = 200;

impl Session {
    pub fn new(doc: Doc, assets: AssetBytes) -> Session {
        let whole = Region::new(0, 0, doc.width as i32, doc.height as i32);
        let view = View { scale: 1.0, x: 0.0, y: 0.0, w: doc.width, h: doc.height };
        let mut session = Session {
            doc,
            assets,
            renderer: Renderer::new(),
            done: Vec::new(),
            undone: Vec::new(),
            frame: Raster::new(whole),
            warnings: Vec::new(),
            view,
            view_pixels: Vec::new(),
            dirty_view: Region::new(0, 0, 0, 0),
            stroke: None,
            painted: BTreeSet::new(),
        };
        session.redraw(whole);
        session
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    fn whole(&self) -> Region {
        self.frame.region
    }

    // ---- drawing --------------------------------------------------------------

    /// Re-render one region of the document into the frame, then into the view.
    fn redraw(&mut self, region: Region) {
        let Some(region) = region.intersect(&self.whole()) else { return };
        let (part, warnings) = self.renderer.render_region(&self.doc, &self.assets, region);
        if region == self.whole() {
            self.warnings = warnings;
        }
        let width = region.w as usize * 4;
        for y in region.y..region.y + region.h {
            let to = self.frame.region.index(region.x, y) * 4;
            let from = part.region.index(region.x, y) * 4;
            self.frame.data[to..to + width].copy_from_slice(&part.data[from..from + width]);
        }
        self.update_view(region);
    }

    /// The full-resolution frame: straight RGBA, document size.
    pub fn frame(&self) -> &[u8] {
        &self.frame.data
    }

    pub fn frame_hash(&self) -> u64 {
        hash(&self.frame.data)
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    // ---- the view --------------------------------------------------------------

    pub fn set_view(&mut self, view: View) {
        self.view = View { scale: view.scale.max(1e-3), ..view };
        self.view_pixels = vec![0; view.w as usize * view.h as usize * 4];
        self.update_view(self.whole());
        self.dirty_view = Region::new(0, 0, view.w as i32, view.h as i32);
    }

    /// The view's pixels: straight RGBA, `view.w × view.h`.
    pub fn view_pixels(&self) -> &[u8] {
        &self.view_pixels
    }

    pub fn view(&self) -> View {
        self.view
    }

    /// The part of the view that the last change redrew.
    pub fn dirty_view(&self) -> Region {
        self.dirty_view
    }

    /// Refresh the view pixels that show `region` of the document.
    fn update_view(&mut self, region: Region) {
        let v = self.view;
        if self.view_pixels.is_empty() {
            self.dirty_view = Region::new(0, 0, 0, 0);
            return;
        }
        let x0 = (((region.x as f32 - v.x) * v.scale).floor() as i32 - 1).max(0);
        let y0 = (((region.y as f32 - v.y) * v.scale).floor() as i32 - 1).max(0);
        let x1 = ((((region.x + region.w) as f32 - v.x) * v.scale).ceil() as i32 + 1).min(v.w as i32);
        let y1 = ((((region.y + region.h) as f32 - v.y) * v.scale).ceil() as i32 + 1).min(v.h as i32);
        self.dirty_view = Region::new(x0, y0, x1 - x0, y1 - y0);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let doc = self.whole();
        let frame = &self.frame.data;
        let step = 1.0 / v.scale;
        // Zoomed out, one screen pixel covers several document pixels: average a small grid of them.
        let taps = (step.ceil() as i32).clamp(1, 4);
        for vy in y0..y1 {
            for vx in x0..x1 {
                let mut acc = [0f32; 4];
                for j in 0..taps {
                    for i in 0..taps {
                        let dx = (v.x + (vx as f32 + (i as f32 + 0.5) / taps as f32) * step).floor() as i32;
                        let dy = (v.y + (vy as f32 + (j as f32 + 0.5) / taps as f32) * step).floor() as i32;
                        if dx < 0 || dy < 0 || dx >= doc.w || dy >= doc.h {
                            continue;
                        }
                        let p = (dy as usize * doc.w as usize + dx as usize) * 4;
                        let a = frame[p + 3] as f32;
                        acc[0] += frame[p] as f32 * a;
                        acc[1] += frame[p + 1] as f32 * a;
                        acc[2] += frame[p + 2] as f32 * a;
                        acc[3] += a;
                    }
                }
                let o = (vy as usize * v.w as usize + vx as usize) * 4;
                let out = &mut self.view_pixels[o..o + 4];
                if acc[3] > 0.0 {
                    out[0] = (acc[0] / acc[3] + 0.5) as u8;
                    out[1] = (acc[1] / acc[3] + 0.5) as u8;
                    out[2] = (acc[2] / acc[3] + 0.5) as u8;
                    out[3] = (acc[3] / (taps * taps) as f32 + 0.5) as u8;
                } else {
                    out.fill(0);
                }
            }
        }
    }

    // ---- edits and history -------------------------------------------------------

    fn push(&mut self, step: Step) {
        self.done.push(step);
        if self.done.len() > HISTORY_LIMIT {
            self.done.remove(0);
        }
        self.undone.clear();
    }

    /// Apply a structural edit. A refused edit changes nothing.
    pub fn execute(&mut self, edit: Edit, label: &str) -> Result<()> {
        if self.stroke.is_some() {
            return Err(err!("Finish the brush stroke first"));
        }
        let inverse = apply(&mut self.doc, &edit)?;
        self.push(Step::Edit { label: label.to_string(), edit, inverse });
        self.redraw(self.whole());
        Ok(())
    }

    fn replay(&mut self, step: &Step, forward: bool) {
        match step {
            Step::Edit { edit, inverse, .. } => {
                apply(&mut self.doc, if forward { edit } else { inverse }).expect("a recorded edit always applies");
                self.redraw(self.whole());
            }
            Step::Paint { asset, offset, region, before, after, margin, .. } => {
                if let Some(image) = self.renderer.image_mut(asset, &self.assets) {
                    paste(image, region, if forward { after } else { before });
                }
                // The pixels no longer match the encoded bytes, whichever way this went.
                self.painted.insert(asset.clone());
                self.redraw(Region::new(region.x + offset.0 - margin, region.y + offset.1 - margin, region.w + margin * 2, region.h + margin * 2));
            }
        }
    }

    /// Returns false when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        if self.stroke.is_some() {
            return false;
        }
        let Some(step) = self.done.pop() else { return false };
        self.replay(&step, false);
        self.undone.push(step);
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.stroke.is_some() {
            return false;
        }
        let Some(step) = self.undone.pop() else { return false };
        self.replay(&step, true);
        self.done.push(step);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Labels of the undo stack, oldest first.
    pub fn labels(&self) -> Vec<&str> {
        self.done.iter().map(Step::label).collect()
    }

    // ---- painting ----------------------------------------------------------------

    /// Add an empty, document-sized pixel layer on top and return its id.
    pub fn add_paint_layer(&mut self, name: &str) -> Result<String> {
        let mut n = 1;
        while self.doc.assets.contains_key(&format!("paint-{n}")) || crate::tree::find(&self.doc, &format!("paint-{n}")).is_some() {
            n += 1;
        }
        let id = format!("paint-{n}");
        let (w, h) = (self.doc.width, self.doc.height);
        self.renderer.set_image(&id, Image { w: w as i32, h: h as i32, data: vec![0; w as usize * h as usize * 4] });
        self.painted.insert(id.clone());
        let mut layer = pixel(&id, &id, crate::doc::IDENTITY);
        layer.base_mut().name = name.to_string();
        let asset = AssetRef { kind: AssetKind::Image, path: format!("assets/{id}.png"), width: Some(w), height: Some(h) };
        let root = self.doc.root.base.id.clone();
        self.execute(Edit::Batch { edits: vec![Edit::SetAsset { id: id.clone(), asset: Some(asset) }, Edit::AddLayer { parent: root, layer, index: None }] }, "New paint layer")?;
        Ok(id)
    }

    /// Start a brush stroke on a pixel layer.
    pub fn stroke_begin(&mut self, layer_id: &str, brush: Brush) -> Result<()> {
        if self.stroke.is_some() {
            return Err(err!("A brush stroke is already in progress"));
        }
        let (asset, offset, margin) = paint_target(&self.doc.root, layer_id, 0).ok_or_else(|| err!("No layer with id \"{layer_id}\""))??;
        // Painting changes the image itself, which would silently change every other layer showing it.
        let mut users = 0;
        crate::tree::walk(&self.doc.root, &mut |layer| {
            let own = match layer {
                Layer::Pixel(l) => Some(&l.asset),
                Layer::Fill(l) => Some(&l.asset),
                Layer::Art(l) => l.asset.as_ref(),
                _ => None,
            };
            users += usize::from(own == Some(&asset)) + usize::from(layer.base().mask.as_ref().is_some_and(|m| m.asset == asset));
        });
        if users > 1 {
            return Err(err!("This layer shares its image with another layer. Painting on shared images is not supported yet."));
        }
        let image = self.renderer.image_mut(&asset, &self.assets).ok_or_else(|| err!("The layer's image is unavailable"))?;
        self.stroke = Some(ActiveStroke { stroke: Stroke::new(image, brush), asset, offset, margin });
        Ok(())
    }

    /// Extend the stroke to a document position. Redraws what changed.
    pub fn stroke_to(&mut self, x: f32, y: f32, pressure: f32) {
        self.dirty_view = Region::new(0, 0, 0, 0);
        let Some(active) = self.stroke.as_mut() else { return };
        let Some(image) = self.renderer.image_mut(&active.asset, &self.assets) else { return };
        let (ox, oy) = active.offset;
        let Some(changed) = active.stroke.line_to(image, x - ox as f32, y - oy as f32, pressure) else { return };
        let m = active.margin;
        self.redraw(Region::new(changed.x + ox - m, changed.y + oy - m, changed.w + m * 2, changed.h + m * 2));
    }

    /// Finish the stroke and record it for undo.
    pub fn stroke_end(&mut self, label: &str) {
        let Some(active) = self.stroke.take() else { return };
        let Some(region) = active.stroke.dirty() else { return };
        let Some(image) = self.renderer.image(&active.asset) else { return };
        let step = Step::Paint {
            label: label.to_string(),
            before: crop(active.stroke.before(), image.w, &region),
            after: crop(&image.data, image.w, &region),
            asset: active.asset.clone(),
            offset: active.offset,
            region,
            margin: active.margin,
        };
        self.painted.insert(active.asset);
        self.push(step);
    }

    // ---- saving -------------------------------------------------------------------

    /// The document as `.cardsmith` bytes, with painted layers encoded.
    pub fn save(&mut self) -> Result<Vec<u8>> {
        for id in std::mem::take(&mut self.painted) {
            if let Some(image) = self.renderer.image(&id) {
                self.assets.insert(id, encode_png(image.w as u32, image.h as u32, &image.data)?);
            }
        }
        save_cardsmith(&self.doc, &self.assets)
    }
}

/// Find a layer for painting. Returns its image asset, the image's document
/// offset, and the largest effect extent on the layer or any group around it.
/// The outer `None` means the layer is not under `group`.
fn paint_target(group: &GroupLayer, id: &str, margin: i32) -> Option<Result<(String, (i32, i32), i32)>> {
    let extent = |layer: &Layer| layer.base().active_effects().iter().map(|e| effect_extent(e)).max().unwrap_or(0);
    for layer in &group.children {
        let margin = margin.max(extent(layer));
        if layer.id() == id {
            let Layer::Pixel(p) = layer else {
                return Some(Err(err!("Only pixel layers can be painted on. \"{}\" is a {} layer.", layer.base().name, layer.kind())));
            };
            let [a, c, tx, b, d, ty] = p.transform;
            if a != 1.0 || d != 1.0 || b != 0.0 || c != 0.0 || tx.fract() != 0.0 || ty.fract() != 0.0 {
                return Some(Err(err!("\"{}\" is scaled or rotated. Painting on transformed layers is not supported yet.", p.base.name)));
            }
            return Some(Ok((p.asset.clone(), (tx as i32, ty as i32), margin)));
        }
        if let Layer::Group(g) = layer {
            if let Some(found) = paint_target(g, id, margin) {
                return Some(found);
            }
        }
    }
    None
}
