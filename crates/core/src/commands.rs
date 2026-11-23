//! Every edit is a command (rule 3).
//!
//! An [`Edit`] is plain data: it can be built in Rust or sent from the UI as
//! JSON. Applying one returns its exact inverse, which is how undo works.

use crate::doc::{AssetId, AssetRef, Doc, GroupLayer, Layer, LayerBase, LayerId};
use crate::tree::{all_ids, group_at_mut, ids_within, layer_at, layer_at_mut, locate, locate_group, must_locate};
use crate::{err, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Edit {
    /// Patch properties on one layer, by their JSON names. `null` removes a
    /// property. Covers visibility, opacity, blend, masks, effects, transforms,
    /// text and so on.
    SetLayerProps { id: LayerId, props: Map<String, Value> },
    /// Insert a layer (with any children) into a group. Index 0 is the bottom; the default is the top.
    AddLayer {
        parent: LayerId,
        layer: Layer,
        #[serde(default)]
        index: Option<usize>,
    },
    RemoveLayer { id: LayerId },
    /// Move a layer to `index` inside `parent`. The index is the layer's position in the destination after the move.
    MoveLayer { id: LayerId, parent: LayerId, index: usize },
    /// Register, replace or (with no `asset`) remove an asset reference.
    SetAsset {
        id: AssetId,
        #[serde(default)]
        asset: Option<AssetRef>,
    },
    /// Patch `width`, `height`, `dpi`, `card` or `templateId`. `null` removes the optional ones.
    SetDocProps { props: Map<String, Value> },
    /// Several edits as one undo step. If one fails, the ones already applied are rolled back.
    Batch { edits: Vec<Edit> },
}

/// Keys that define what a layer is or where it sits. Other edits change those.
const STRUCTURAL: [&str; 3] = ["id", "kind", "children"];

fn placeholder() -> GroupLayer {
    GroupLayer { base: LayerBase::new(""), children: Vec::new(), pass_through: true }
}

/// Patch one layer through its JSON form, so property names and types are
/// checked by the same code that loads documents. Returns the previous values.
fn patch_layer(layer: &mut Layer, props: &Map<String, Value>) -> Result<Map<String, Value>> {
    for key in props.keys() {
        if STRUCTURAL.contains(&key.as_str()) {
            return Err(err!("\"{key}\" cannot be changed with setLayerProps"));
        }
    }
    // Children are set aside: they are not being edited and can be large.
    let children = match layer {
        Layer::Group(g) => Some(std::mem::take(&mut g.children)),
        _ => None,
    };
    let restore = |layer: &mut Layer, children: Option<Vec<Layer>>| {
        if let (Layer::Group(g), Some(c)) = (layer, children) {
            g.children = c;
        }
    };

    let mut value = serde_json::to_value(&*layer).map_err(|e| err!("{e}"))?;
    let object = value.as_object_mut().expect("a layer serializes to an object");
    let mut before = Map::new();
    for (key, new) in props {
        before.insert(key.clone(), object.get(key).cloned().unwrap_or(Value::Null));
        if new.is_null() {
            object.remove(key);
        } else {
            object.insert(key.clone(), new.clone());
        }
    }
    let patched = serde_json::from_value::<Layer>(value).map_err(|e| err!("Invalid layer properties: {e}"));
    let mut patched = match patched {
        Ok(p) => p,
        Err(e) => {
            restore(layer, children);
            return Err(e);
        }
    };
    // Serde ignores unknown keys, which would turn a typo into a silent no-op.
    let check = serde_json::to_value(&patched).map_err(|e| err!("{e}"))?;
    if let Some(unknown) = props.iter().find(|(k, v)| !v.is_null() && check.get(k.as_str()).is_none()) {
        restore(layer, children);
        return Err(err!("A {} layer has no property \"{}\"", layer.kind(), unknown.0));
    }
    restore(&mut patched, children);
    *layer = patched;
    Ok(before)
}

fn patch_doc(doc: &mut Doc, props: &Map<String, Value>) -> Result<Map<String, Value>> {
    let mut before = Map::new();
    let mut next = (doc.width, doc.height, doc.dpi, doc.card.clone(), doc.template_id.clone());
    let positive = |key: &str, v: &Value| v.as_f64().filter(|n| *n > 0.0).ok_or_else(|| err!("\"{key}\" must be a positive number"));
    for (key, value) in props {
        let old = match key.as_str() {
            "width" => {
                next.0 = positive(key, value)? as u32;
                Value::from(doc.width)
            }
            "height" => {
                next.1 = positive(key, value)? as u32;
                Value::from(doc.height)
            }
            "dpi" => {
                next.2 = positive(key, value)?;
                Value::from(doc.dpi)
            }
            "card" => {
                next.3 = (!value.is_null()).then(|| value.clone());
                doc.card.clone().unwrap_or(Value::Null)
            }
            "templateId" => {
                next.4 = match value {
                    Value::Null => None,
                    Value::String(s) => Some(s.clone()),
                    _ => return Err(err!("\"templateId\" must be a string")),
                };
                doc.template_id.clone().map(Value::from).unwrap_or(Value::Null)
            }
            other => return Err(err!("\"{other}\" cannot be changed with setDocProps")),
        };
        before.insert(key.clone(), old);
    }
    (doc.width, doc.height, doc.dpi, doc.card, doc.template_id) = next;
    Ok(before)
}

/// Apply an edit and return the edit that undoes it. On error the document is unchanged.
pub fn apply(doc: &mut Doc, edit: &Edit) -> Result<Edit> {
    match edit {
        Edit::SetLayerProps { id, props } => {
            let before = if *id == doc.root.base.id {
                let mut root = Layer::Group(std::mem::replace(&mut doc.root, placeholder()));
                let result = patch_layer(&mut root, props);
                let Layer::Group(group) = root else { unreachable!() };
                doc.root = group;
                result?
            } else {
                let path = locate(doc, id).ok_or_else(|| err!("No layer with id \"{id}\""))?;
                patch_layer(layer_at_mut(doc, &path), props)?
            };
            Ok(Edit::SetLayerProps { id: id.clone(), props: before })
        }

        Edit::AddLayer { parent, layer, index } => {
            let path = locate_group(doc, parent)?;
            {
                let existing: HashSet<&str> = all_ids(doc).into_iter().collect();
                let incoming = ids_within(layer);
                let mut seen = HashSet::new();
                for id in incoming {
                    if existing.contains(id) || !seen.insert(id) {
                        return Err(err!("Layer id \"{id}\" is already in the document"));
                    }
                }
            }
            let group = group_at_mut(doc, &path);
            let at = index.unwrap_or(group.children.len());
            if at > group.children.len() {
                return Err(err!("Index {at} is out of range"));
            }
            group.children.insert(at, layer.clone());
            Ok(Edit::RemoveLayer { id: layer.id().to_string() })
        }

        Edit::RemoveLayer { id } => {
            let mut path = must_locate(doc, id)?;
            let index = path.pop().expect("a layer path is never empty");
            let group = group_at_mut(doc, &path);
            let layer = group.children.remove(index);
            Ok(Edit::AddLayer { parent: group.base.id.clone(), layer, index: Some(index) })
        }

        Edit::MoveLayer { id, parent, index } => {
            let mut from = must_locate(doc, id)?;
            let dest = locate_group(doc, parent)?;
            if ids_within(layer_at(doc, &from)).contains(&parent.as_str()) {
                return Err(err!("A group cannot be moved into itself"));
            }
            let from_index = from.pop().expect("a layer path is never empty");
            let same_parent = from == dest;
            let dest_len = group_at_mut(doc, &dest).children.len();
            if *index > dest_len - usize::from(same_parent) {
                return Err(err!("Index {index} is out of range"));
            }
            let old_parent = group_at_mut(doc, &from);
            let old_parent_id = old_parent.base.id.clone();
            let layer = old_parent.children.remove(from_index);
            // Removing the layer cannot disturb the destination's path: the
            // destination is not inside the moved layer, and index paths only
            // shift for later siblings of the removed one, which we re-resolve.
            let dest = locate_group(doc, parent)?;
            group_at_mut(doc, &dest).children.insert(*index, layer);
            Ok(Edit::MoveLayer { id: id.clone(), parent: old_parent_id, index: from_index })
        }

        Edit::SetAsset { id, asset } => {
            let before = match asset {
                Some(a) => doc.assets.insert(id.clone(), a.clone()),
                None => doc.assets.remove(id),
            };
            Ok(Edit::SetAsset { id: id.clone(), asset: before })
        }

        Edit::SetDocProps { props } => Ok(Edit::SetDocProps { props: patch_doc(doc, props)? }),

        Edit::Batch { edits } => {
            let mut inverses = Vec::with_capacity(edits.len());
            for edit in edits {
                match apply(doc, edit) {
                    Ok(inverse) => inverses.push(inverse),
                    Err(e) => {
                        for inverse in inverses.iter().rev() {
                            apply(doc, inverse).expect("an inverse always applies");
                        }
                        return Err(e);
                    }
                }
            }
            inverses.reverse();
            Ok(Edit::Batch { edits: inverses })
        }
    }
}

struct Entry {
    label: String,
    edit: Edit,
    inverse: Edit,
}

/// A document with its undo and redo stacks.
pub struct History {
    doc: Doc,
    done: Vec<Entry>,
    undone: Vec<Entry>,
    limit: usize,
}

impl History {
    pub fn new(doc: Doc) -> Self {
        Self::with_limit(doc, 500)
    }

    pub fn with_limit(doc: Doc, limit: usize) -> Self {
        History { doc, done: Vec::new(), undone: Vec::new(), limit }
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    /// Apply an edit and record it. A failed edit changes nothing and is not recorded.
    pub fn execute(&mut self, edit: Edit, label: &str) -> Result<()> {
        let inverse = apply(&mut self.doc, &edit)?;
        self.done.push(Entry { label: label.to_string(), edit, inverse });
        if self.done.len() > self.limit {
            self.done.remove(0);
        }
        self.undone.clear();
        Ok(())
    }

    /// Returns false when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        let Some(entry) = self.done.pop() else { return false };
        apply(&mut self.doc, &entry.inverse).expect("an inverse always applies");
        self.undone.push(entry);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(entry) = self.undone.pop() else { return false };
        apply(&mut self.doc, &entry.edit).expect("a redone edit applies as it did the first time");
        self.done.push(entry);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Labels of the undo stack, oldest first. For the history panel.
    pub fn labels(&self) -> Vec<&str> {
        self.done.iter().map(|e| e.label.as_str()).collect()
    }
}
