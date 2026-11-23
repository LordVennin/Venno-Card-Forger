/**
 * Every edit is a command (rule 3). A command is `{ label, do, undo }` and
 * mutates the document in place. Whatever `undo` needs is captured inside `do`,
 * so a command can be redone any number of times.
 */
import { findLayer, isWithin, findGroup, mustLocate, allIds } from "./tree.js";
import type { AssetId, AssetRef, Doc, Layer, LayerId } from "./types.js";

export type Command = {
  label: string;
  do(doc: Doc): void;
  undo(doc: Doc): void;
};

const clone = <T>(value: T): T => structuredClone(value);

/** Keys that define what a layer is, or where it sits. Changed by other commands. */
const STRUCTURAL = new Set(["id", "kind", "children"]);

/**
 * Patch properties on one layer. A value of `undefined` removes the property.
 * Covers visibility, opacity, blend, masks, effects, transforms, text and so on.
 */
export function setLayerProps(id: LayerId, props: Record<string, unknown>, label = "Edit layer"): Command {
  for (const key of Object.keys(props)) {
    if (STRUCTURAL.has(key)) throw new Error(`"${key}" cannot be changed with setLayerProps`);
  }
  let before: Record<string, unknown> | undefined;
  const target = (doc: Doc) => {
    const layer = findLayer(doc, id);
    if (!layer) throw new Error(`No layer with id "${id}"`);
    return layer as unknown as Record<string, unknown>;
  };
  const assign = (layer: Record<string, unknown>, values: Record<string, unknown>) => {
    for (const [key, value] of Object.entries(values)) {
      if (value === undefined) delete layer[key];
      else layer[key] = clone(value);
    }
  };
  return {
    label,
    do(doc) {
      const layer = target(doc);
      before = {};
      for (const key of Object.keys(props)) before[key] = clone(layer[key]);
      assign(layer, props);
    },
    undo(doc) {
      if (!before) throw new Error("undo called before do");
      assign(target(doc), before);
    },
  };
}

/** Insert a layer (with any children) into a group. `index` 0 is the bottom; default is the top. */
export function addLayer(parentId: LayerId, layer: Layer, index?: number, label = "Add layer"): Command {
  const fresh = clone(layer);
  return {
    label,
    do(doc) {
      const parent = findGroup(doc, parentId);
      const existing = new Set(allIds(doc));
      const incoming = [fresh.id, ...(fresh.kind === "group" ? allIds({ root: fresh } as Doc).slice(1) : [])];
      for (const id of incoming) if (existing.has(id)) throw new Error(`Layer id "${id}" is already in the document`);
      const at = index ?? parent.children.length;
      if (at < 0 || at > parent.children.length) throw new Error(`Index ${at} is out of range`);
      parent.children.splice(at, 0, clone(fresh));
    },
    undo(doc) {
      const { parent, index: at } = mustLocate(doc, fresh.id);
      parent.children.splice(at, 1);
    },
  };
}

export function removeLayer(id: LayerId, label = "Delete layer"): Command {
  let removed: { layer: Layer; parentId: LayerId; index: number } | undefined;
  return {
    label,
    do(doc) {
      const { layer, parent, index } = mustLocate(doc, id);
      parent.children.splice(index, 1);
      removed = { layer, parentId: parent.id, index };
    },
    undo(doc) {
      if (!removed) throw new Error("undo called before do");
      findGroup(doc, removed.parentId).children.splice(removed.index, 0, removed.layer);
    },
  };
}

/**
 * Move a layer to `index` inside `parentId`. The index is the layer's position
 * in the destination after the move.
 */
export function moveLayer(id: LayerId, parentId: LayerId, index: number, label = "Move layer"): Command {
  let from: { parentId: LayerId; index: number } | undefined;
  return {
    label,
    do(doc) {
      const { layer, parent, index: at } = mustLocate(doc, id);
      const dest = findGroup(doc, parentId);
      if (isWithin(layer, parentId)) throw new Error("A group cannot be moved into itself");
      const max = dest.children.length - (dest === parent ? 1 : 0);
      if (index < 0 || index > max) throw new Error(`Index ${index} is out of range`);
      parent.children.splice(at, 1);
      dest.children.splice(index, 0, layer);
      from = { parentId: parent.id, index: at };
    },
    undo(doc) {
      if (!from) throw new Error("undo called before do");
      const { layer, parent, index: at } = mustLocate(doc, id);
      parent.children.splice(at, 1);
      findGroup(doc, from.parentId).children.splice(from.index, 0, layer);
    },
  };
}

/** Register, replace or (with `undefined`) remove an asset reference. */
export function setAsset(id: AssetId, ref: AssetRef | undefined, label = "Change asset"): Command {
  let before: AssetRef | undefined;
  const put = (doc: Doc, value: AssetRef | undefined) => {
    if (value) doc.assets[id] = clone(value);
    else delete doc.assets[id];
  };
  return {
    label,
    do(doc) {
      before = doc.assets[id] && clone(doc.assets[id]);
      put(doc, ref);
    },
    undo(doc) {
      put(doc, before);
    },
  };
}

type DocProps = Partial<Pick<Doc, "width" | "height" | "dpi" | "card" | "templateId">>;

export function setDocProps(props: DocProps, label = "Edit document"): Command {
  let before: Record<string, unknown> | undefined;
  const assign = (doc: Doc, values: Record<string, unknown>) => {
    const target = doc as unknown as Record<string, unknown>;
    for (const [key, value] of Object.entries(values)) {
      if (value === undefined) delete target[key];
      else target[key] = clone(value);
    }
  };
  return {
    label,
    do(doc) {
      before = {};
      for (const key of Object.keys(props)) before[key] = clone((doc as unknown as Record<string, unknown>)[key]);
      assign(doc, props);
    },
    undo(doc) {
      if (!before) throw new Error("undo called before do");
      assign(doc, before);
    },
  };
}

/**
 * Run several commands as one undo step. If one fails part-way, the ones that
 * already ran are undone, so the document is never left half-edited.
 */
export function batch(commands: Command[], label: string): Command {
  return {
    label,
    do(doc) {
      let done = 0;
      try {
        for (; done < commands.length; done++) commands[done]!.do(doc);
      } catch (err) {
        while (done-- > 0) commands[done]!.undo(doc);
        throw err;
      }
    },
    undo(doc) {
      for (let i = commands.length - 1; i >= 0; i--) commands[i]!.undo(doc);
    },
  };
}

/** Undo/redo stack bound to one document. */
export class History {
  private done: Command[] = [];
  private undone: Command[] = [];
  private listeners = new Set<() => void>();

  constructor(
    readonly doc: Doc,
    private limit = 500,
  ) {}

  /** Run a command and record it. A command that throws is not recorded. */
  execute(command: Command): void {
    command.do(this.doc);
    this.done.push(command);
    if (this.done.length > this.limit) this.done.shift();
    this.undone = [];
    this.emit();
  }

  undo(): boolean {
    const command = this.done.pop();
    if (!command) return false;
    command.undo(this.doc);
    this.undone.push(command);
    this.emit();
    return true;
  }

  redo(): boolean {
    const command = this.undone.pop();
    if (!command) return false;
    command.do(this.doc);
    this.done.push(command);
    this.emit();
    return true;
  }

  get canUndo(): boolean {
    return this.done.length > 0;
  }
  get canRedo(): boolean {
    return this.undone.length > 0;
  }
  /** Labels of the undo stack, oldest first. For the history panel. */
  get labels(): string[] {
    return this.done.map((c) => c.label);
  }

  /** Called after every execute, undo and redo. Returns an unsubscribe function. */
  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private emit(): void {
    for (const listener of this.listeners) listener();
  }
}
