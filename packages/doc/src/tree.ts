import type { Doc, GroupLayer, Layer, LayerId } from "./types.js";

export type Located = { layer: Layer; parent: GroupLayer; index: number };

/** Depth-first walk, bottom layer first. Return false from `visit` to stop. */
export function walk(root: GroupLayer, visit: (layer: Layer, parent: GroupLayer, index: number) => boolean | void): void {
  const go = (parent: GroupLayer): boolean => {
    for (let i = 0; i < parent.children.length; i++) {
      const layer = parent.children[i]!;
      if (visit(layer, parent, i) === false) return false;
      if (layer.kind === "group" && !go(layer)) return false;
    }
    return true;
  };
  go(root);
}

export function locate(doc: Doc, id: LayerId): Located | undefined {
  let found: Located | undefined;
  walk(doc.root, (layer, parent, index) => {
    if (layer.id !== id) return;
    found = { layer, parent, index };
    return false;
  });
  return found;
}

export function findLayer(doc: Doc, id: LayerId): Layer | undefined {
  return id === doc.root.id ? doc.root : locate(doc, id)?.layer;
}

export function findGroup(doc: Doc, id: LayerId): GroupLayer {
  const layer = findLayer(doc, id);
  if (!layer) throw new Error(`No layer with id "${id}"`);
  if (layer.kind !== "group") throw new Error(`Layer "${id}" is not a group`);
  return layer;
}

export function mustLocate(doc: Doc, id: LayerId): Located {
  const at = locate(doc, id);
  if (!at) throw new Error(id === doc.root.id ? "The root group cannot be moved or removed" : `No layer with id "${id}"`);
  return at;
}

/** Every layer id in the document, including the root. */
export function allIds(doc: Doc): LayerId[] {
  const ids = [doc.root.id];
  walk(doc.root, (layer) => void ids.push(layer.id));
  return ids;
}

/** True if `id` is `ancestor` or sits anywhere beneath it. */
export function isWithin(ancestor: Layer, id: LayerId): boolean {
  if (ancestor.id === id) return true;
  if (ancestor.kind !== "group") return false;
  let hit = false;
  walk(ancestor, (layer) => {
    if (layer.id === id) {
      hit = true;
      return false;
    }
  });
  return hit;
}
