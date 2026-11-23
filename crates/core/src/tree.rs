//! Finding layers in the tree.

use crate::doc::{Doc, GroupLayer, Layer};
use crate::{err, Result};

/// Index path from the root to a layer: `[2, 0]` is the first child of the root's third child.
pub type IndexPath = Vec<usize>;

fn search(group: &GroupLayer, id: &str, path: &mut IndexPath) -> bool {
    for (i, layer) in group.children.iter().enumerate() {
        path.push(i);
        if layer.id() == id {
            return true;
        }
        if let Layer::Group(g) = layer {
            if search(g, id, path) {
                return true;
            }
        }
        path.pop();
    }
    false
}

/// Where a layer sits. `None` for the root itself or an unknown id.
pub fn locate(doc: &Doc, id: &str) -> Option<IndexPath> {
    let mut path = Vec::new();
    search(&doc.root, id, &mut path).then_some(path)
}

pub fn must_locate(doc: &Doc, id: &str) -> Result<IndexPath> {
    locate(doc, id).ok_or_else(|| {
        if id == doc.root.base.id {
            err!("The root group cannot be moved or removed")
        } else {
            err!("No layer with id \"{id}\"")
        }
    })
}

pub fn layer_at<'a>(doc: &'a Doc, path: &[usize]) -> &'a Layer {
    let (first, rest) = path.split_first().expect("a layer path is never empty");
    let mut layer = &doc.root.children[*first];
    for &i in rest {
        layer = &layer.children()[i];
    }
    layer
}

pub fn layer_at_mut<'a>(doc: &'a mut Doc, path: &[usize]) -> &'a mut Layer {
    let (first, rest) = path.split_first().expect("a layer path is never empty");
    let mut layer = &mut doc.root.children[*first];
    for &i in rest {
        layer = match layer {
            Layer::Group(g) => &mut g.children[i],
            _ => unreachable!("layer paths only pass through groups"),
        };
    }
    layer
}

/// The group at `path`, where an empty path means the root.
pub fn group_at_mut<'a>(doc: &'a mut Doc, path: &[usize]) -> &'a mut GroupLayer {
    if path.is_empty() {
        return &mut doc.root;
    }
    match layer_at_mut(doc, path) {
        Layer::Group(g) => g,
        _ => unreachable!("callers check the layer is a group"),
    }
}

pub fn find<'a>(doc: &'a Doc, id: &str) -> Option<&'a Layer> {
    locate(doc, id).map(|p| layer_at(doc, &p))
}

/// Path of the group with this id (empty for the root), or an error saying why not.
pub fn locate_group(doc: &Doc, id: &str) -> Result<IndexPath> {
    if id == doc.root.base.id {
        return Ok(Vec::new());
    }
    let path = locate(doc, id).ok_or_else(|| err!("No layer with id \"{id}\""))?;
    match layer_at(doc, &path) {
        Layer::Group(_) => Ok(path),
        _ => Err(err!("Layer \"{id}\" is not a group")),
    }
}

/// Call `visit` for every layer under `group`, bottom first, parents before children.
pub fn walk<'a>(group: &'a GroupLayer, visit: &mut impl FnMut(&'a Layer)) {
    for layer in &group.children {
        visit(layer);
        if let Layer::Group(g) = layer {
            walk(g, visit);
        }
    }
}

/// Every layer id under a layer, including its own.
pub fn ids_within(layer: &Layer) -> Vec<&str> {
    let mut ids = vec![layer.id()];
    if let Layer::Group(g) = layer {
        walk(g, &mut |l| ids.push(l.id()));
    }
    ids
}

/// Every layer id in the document, including the root's.
pub fn all_ids(doc: &Doc) -> Vec<&str> {
    let mut ids = vec![doc.root.base.id.as_str()];
    walk(&doc.root, &mut |l| ids.push(l.id()));
    ids
}
