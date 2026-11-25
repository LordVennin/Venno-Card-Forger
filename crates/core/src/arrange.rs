//! Moving and scaling layers.
//!
//! Every kind of layer keeps its place differently: an image in a transform, a
//! shape in its path, text in its box, a group in its children. These
//! functions work out the edit that moves or scales any of them, masks
//! included, so the editor can drag whatever is selected.

use crate::commands::Edit;
use crate::doc::*;
use crate::{err, tree, Result};
use serde_json::{Map, Value};

/// Move every point `p` of a layer to `scale * R(turn) * p + (tx, ty)`, where
/// `R` turns clockwise on screen by `turn` radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub scale: f64,
    pub turn: f64,
    pub tx: f64,
    pub ty: f64,
}

impl Placement {
    pub fn shift(dx: f64, dy: f64) -> Placement {
        Placement { scale: 1.0, turn: 0.0, tx: dx, ty: dy }
    }

    /// Turn about a point that stays where it is.
    pub fn turn_about(turn: f64, x: f64, y: f64) -> Placement {
        let mut to = Placement { scale: 1.0, turn, tx: 0.0, ty: 0.0 };
        let (px, py) = to.point(x, y);
        (to.tx, to.ty) = (x - px, y - py);
        to
    }

    /// The linear part: [a, b, c, d] with x' = a x + b y, y' = c x + d y.
    fn linear(&self) -> [f64; 4] {
        let (sin, cos) = self.turn.sin_cos();
        [self.scale * cos, -self.scale * sin, self.scale * sin, self.scale * cos]
    }

    /// Scale about a point that stays where it is.
    pub fn scale_about(scale: f64, x: f64, y: f64) -> Placement {
        Placement { scale, turn: 0.0, tx: x * (1.0 - scale), ty: y * (1.0 - scale) }
    }

    fn point(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d] = self.linear();
        (a * x + b * y + self.tx, c * x + d * y + self.ty)
    }

    fn path(&self, d: &str) -> Result<String> {
        let mut path = kurbo::BezPath::from_svg(d).map_err(|_| err!("A vector path could not be read, so its layer cannot be moved"))?;
        let [a, b, c, d] = self.linear();
        path.apply_affine(kurbo::Affine::new([a, c, b, d, self.tx, self.ty]));
        Ok(path.to_svg())
    }

    fn matrix(&self, m: &Mat3) -> Mat3 {
        // A layer's matrix is [a, b, tx, c, d, ty]: this placement applied after it.
        let [a, b, c, d] = self.linear();
        [a * m[0] + b * m[3], a * m[1] + b * m[4], a * m[2] + b * m[5] + self.tx, c * m[0] + d * m[3], c * m[1] + d * m[4], c * m[2] + d * m[5] + self.ty]
    }
}

fn place(layer: &mut Layer, to: &Placement) -> Result<()> {
    let s = to.scale;
    let name = layer.base().name.clone();
    {
        let base = match layer {
            Layer::Group(l) => &mut l.base,
            Layer::Pixel(l) => &mut l.base,
            Layer::Fill(l) => &mut l.base,
            Layer::Art(l) => &mut l.base,
            Layer::Shape(l) => &mut l.base,
            Layer::Text(l) => &mut l.base,
        };
        if let Some(mask) = &mut base.mask {
            if to.turn != 0.0 {
                return Err(err!("\"{name}\" has a pixel mask, which cannot be turned yet. It can be moved and scaled."));
            }
            let (x, y) = to.point(mask.offset.x, mask.offset.y);
            mask.offset = Vec2 { x, y };
            if s != 1.0 {
                mask.scale = Some(mask.scale.unwrap_or(1.0) * s);
                mask.feather = mask.feather.map(|f| f * s);
            }
        }
        if let Some(mask) = &mut base.vector_mask {
            mask.path.d = to.path(&mask.path.d)?;
        }
    }
    match layer {
        Layer::Group(_) => {}
        Layer::Pixel(l) => l.transform = to.matrix(&l.transform),
        Layer::Fill(l) => l.transform = to.matrix(&l.transform),
        Layer::Art(l) => l.transform = to.matrix(&l.transform),
        Layer::Shape(l) => {
            l.path.d = to.path(&l.path.d)?;
            if let Some(stroke) = &mut l.stroke {
                stroke.width *= s;
            }
        }
        Layer::Text(l) => {
            if to.turn != 0.0 {
                return Err(err!("\"{name}\" is text, which cannot be turned yet. It can be moved and scaled."));
            }
            let (x, y) = to.point(l.bounds.x, l.bounds.y);
            l.bounds = Rect { x, y, w: l.bounds.w * s, h: l.bounds.h * s };
            if let Some(origin) = l.origin {
                let (x, y) = to.point(origin.x, origin.y);
                l.origin = Some(Vec2 { x, y });
            }
            l.style.size *= s;
            l.style.leading = l.style.leading.map(|v| v * s);
            l.style.paragraph_spacing = l.style.paragraph_spacing.map(|v| v * s);
            for run in &mut l.content.runs {
                run.size = run.size.map(|v| v * s);
            }
            for paragraph in &mut l.content.paragraphs {
                paragraph.space_before = paragraph.space_before.map(|v| v * s);
                paragraph.indent = paragraph.indent.map(|v| v * s);
                paragraph.first_line_indent = paragraph.first_line_indent.map(|v| v * s);
            }
        }
    }
    Ok(())
}

/// The properties of `after` that differ from `before`, as a property patch.
fn changed_props(before: &Layer, after: &Layer) -> Map<String, Value> {
    // A group's children are placed in their own right; only its own fields are compared here.
    let fields = |layer: &Layer| {
        let own = match layer {
            Layer::Group(g) => serde_json::to_value(&g.base),
            other => serde_json::to_value(other),
        };
        own.ok().and_then(|v| v.as_object().cloned()).unwrap_or_default()
    };
    let (old, new) = (fields(before), fields(after));
    new.into_iter().filter(|(key, value)| key != "children" && old.get(key) != Some(value)).collect()
}

fn collect(layer: &Layer, to: &Placement, edits: &mut Vec<Edit>) -> Result<()> {
    // The layer alone: a group without what is in it, which is placed below.
    let alone = match layer {
        Layer::Group(g) => Layer::Group(GroupLayer { base: g.base.clone(), children: Vec::new(), pass_through: g.pass_through }),
        other => other.clone(),
    };
    let layer_alone = &alone;
    let mut placed = alone.clone();
    place(&mut placed, to)?;
    let props = changed_props(layer_alone, &placed);
    if !props.is_empty() {
        edits.push(Edit::SetLayerProps { id: layer.id().to_string(), props });
    }
    for child in layer.children() {
        collect(child, to, edits)?;
    }
    Ok(())
}

/// The edit that moves and scales a layer, and everything in it if it is a group.
pub fn place_edit(doc: &Doc, id: &str, to: &Placement) -> Result<Edit> {
    if !(to.scale.is_finite() && to.scale > 0.0 && to.turn.is_finite() && to.tx.is_finite() && to.ty.is_finite()) {
        return Err(err!("A layer can only be scaled by a size above nothing"));
    }
    let layer = tree::find(doc, id).ok_or_else(|| err!("There is no layer \"{id}\""))?;
    let mut edits = Vec::new();
    collect(layer, to, &mut edits)?;
    Ok(Edit::Batch { edits })
}
