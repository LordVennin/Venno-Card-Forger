/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 *
 * The layer names below are ported from Proxyshop src/enums/layers.py, and the
 * way they are looked up from Proxyshop src/templates/_core.py.
 */

/**
 * Bindings connect what the card engine needs ("the white pinlines", "the
 * rules text for creatures") to layers in an imported template, so the engine
 * never searches by name at render time (SPEC §3). They are generated here
 * from Proxyshop's naming conventions and stored in the template's manifest,
 * where they can be corrected by hand.
 */
import type { Doc, GroupLayer, Layer } from "@cardsmith/doc";

const NAMES = {
  art: "Layer 1",
  artFrame: "Art Frame",
  fullArtFrame: "Full Art Frame",
  textGroup: "Text and Icons",
  legalGroup: "Legal",
  borderGroup: "Border",
  masksGroup: "Masks",
  name: "Card Name",
  nameShift: "Card Name Shift",
  mana: "Mana Cost",
  type: "Typeline",
  typeShift: "Typeline Shift",
  rulesCreature: "Rules Text - Creature",
  rulesNoncreature: "Rules Text - Noncreature",
  pt: "Power / Toughness",
  divider: "Divider",
  textboxReference: "Textbox Reference",
  ptReference: "PT Reference",
  expansionReference: "Expansion Reference",
  expansionSymbol: "Expansion Symbol",
  artist: "Artist",
  set: "Set",
  collector: "Collector",
  creator: "Creator",
  twins: "Name & Title Boxes",
  pinlinesTextbox: "Pinlines & Textbox",
  landPinlinesTextbox: "Land Pinlines & Textbox",
  background: "Background",
  nyx: "Nyx",
  companion: "Companion",
  ptBox: "PT Box",
  crown: "Legendary Crown",
  crownShadow: "Hollow Crown Shadow",
  colorIndicator: "Color Indicator",
  shadows: "Shadows",
  // Vector templates (shapes recolored at render time).
  pinlines: "Pinlines",
  textbox: "Textbox",
  shape: "Shape",
} as const;

/** Rules-text size variants a vector template keeps in separate groups. */
const SIZES = ["Textless", "Short", "Medium", "Normal", "Tall"] as const;

export type Bindings = {
  /**
   * "raster": one pre-drawn layer per color, switched on and off.
   * "vector": shapes that are recolored at render time (bound in a later phase).
   */
  kind: "raster" | "vector";
  layouts: string[];
  /** The layer the card's art goes on. */
  art?: string;
  /** Reference shapes the art is framed to, by name: "normal", "fullArt", and size variants. */
  artRefs: Record<string, string>;
  groups: Record<string, string>;
  /** Text layers by purpose. */
  text: Record<string, string>;
  /** Reference layers that text and symbols are positioned against. */
  textRefs: Record<string, string>;
  /** Groups of alternatives: for each, the layer id for every choice (usually a color). */
  toggles: Record<string, Record<string, string>>;
  /** Other single layers the engine switches or styles. */
  layers: Record<string, string>;
  /** Slots this kind of template should have but that no layer was found for. */
  unresolved: string[];
};

const child = (group: GroupLayer | undefined, name: string): Layer | undefined => group?.children.find((l) => l.name === name);
const asGroup = (layer: Layer | undefined): GroupLayer | undefined => (layer?.kind === "group" ? layer : undefined);

export function generateBindings(doc: Doc): Bindings {
  const root = doc.root;
  const text = asGroup(child(root, NAMES.textGroup));
  const legal = asGroup(child(root, NAMES.legalGroup));
  const vector = !child(root, NAMES.pinlinesTextbox) && !!child(asGroup(child(root, NAMES.pinlines)), NAMES.shape);
  const b: Bindings = { kind: vector ? "vector" : "raster", layouts: ["normal"], artRefs: {}, groups: {}, text: {}, textRefs: {}, toggles: {}, layers: {}, unresolved: [] };

  /** Record a layer under `slot` of one table, or note that it is missing. */
  const bind = (table: Record<string, string>, tableName: string, slot: string, layer: Layer | undefined, required: boolean) => {
    if (layer) table[slot] = layer.id;
    else if (required) b.unresolved.push(`${tableName}.${slot}`);
  };

  const art = child(root, NAMES.art);
  if (art) b.art = art.id;
  else b.unresolved.push("art");

  bind(b.artRefs, "artRefs", "fullArt", child(root, NAMES.fullArtFrame), true);
  bind(b.artRefs, "artRefs", "normal", child(root, NAMES.artFrame) ?? child(root, `${NAMES.artFrame} Normal`), true);
  for (const size of SIZES) bind(b.artRefs, "artRefs", size.toLowerCase(), child(root, `${NAMES.artFrame} ${size}`), false);

  bind(b.groups, "groups", "text", text, true);
  bind(b.groups, "groups", "legal", legal, true);
  bind(b.groups, "groups", "border", child(root, NAMES.borderGroup), false);
  bind(b.groups, "groups", "masks", child(root, NAMES.masksGroup), false);

  bind(b.text, "text", "name", child(text, NAMES.name), true);
  bind(b.text, "text", "nameShift", child(text, NAMES.nameShift), false);
  bind(b.text, "text", "mana", child(text, NAMES.mana), true);
  bind(b.text, "text", "type", child(text, NAMES.type), true);
  bind(b.text, "text", "typeShift", child(text, NAMES.typeShift), false);
  bind(b.text, "text", "pt", child(text, NAMES.pt), true);
  bind(b.text, "text", "artist", child(legal, NAMES.artist), true);
  bind(b.text, "text", "set", child(legal, NAMES.set), true);
  bind(b.text, "text", "collector", child(legal, NAMES.collector), false);
  bind(b.text, "text", "creator", child(legal, NAMES.creator), false);
  // Rules text sits directly in the text group, or once per textbox size.
  const sized = SIZES.filter((size) => asGroup(child(text, size)));
  if (sized.length === 0) {
    bind(b.text, "text", "rulesCreature", child(text, NAMES.rulesCreature), true);
    bind(b.text, "text", "rulesNoncreature", child(text, NAMES.rulesNoncreature), true);
  }
  for (const size of sized) {
    const group = asGroup(child(text, size));
    bind(b.text, "text", `rulesCreature.${size.toLowerCase()}`, child(group, NAMES.rulesCreature), true);
    bind(b.text, "text", `rulesNoncreature.${size.toLowerCase()}`, child(group, NAMES.rulesNoncreature), true);
  }

  bind(b.textRefs, "textRefs", "textbox", child(text, NAMES.textboxReference), true);
  bind(b.textRefs, "textRefs", "pt", child(text, NAMES.ptReference), true);
  bind(b.textRefs, "textRefs", "expansion", child(text, NAMES.expansionReference), true);

  bind(b.layers, "layers", "divider", child(text, NAMES.divider), true);
  bind(b.layers, "layers", "expansionSymbol", child(text, NAMES.expansionSymbol), false);
  bind(b.layers, "layers", "crownShadow", child(root, NAMES.crownShadow), !vector);
  bind(b.layers, "layers", "shadows", child(root, NAMES.shadows), false);

  // Alternatives: every child of the group, by its own name ("W", "WU", "Gold", "Land"…).
  const toggle = (slot: string, name: string, required: boolean) => {
    const group = asGroup(child(root, name));
    if (group && group.children.length) b.toggles[slot] = Object.fromEntries(group.children.map((l) => [l.name, l.id]));
    else if (required) b.unresolved.push(`toggles.${slot}`);
  };
  toggle("background", NAMES.background, !vector);
  toggle("nyx", NAMES.nyx, false);
  toggle("companion", NAMES.companion, false);
  toggle("pinlines", NAMES.pinlinesTextbox, !vector);
  toggle("landPinlines", NAMES.landPinlinesTextbox, !vector);
  toggle("twins", NAMES.twins, !vector);
  toggle("ptBox", NAMES.ptBox, !vector);
  toggle("crown", NAMES.crown, !vector);
  toggle("colorIndicator", NAMES.colorIndicator, !vector);
  toggle("border", NAMES.borderGroup, false);

  if (vector) {
    // The shapes a vector template recolors. How they are colored is the card engine's job (Phase 5).
    for (const [slot, name] of [["pinlines", NAMES.pinlines], ["textbox", NAMES.textbox], ["twins", NAMES.twins], ["ptBox", NAMES.ptBox], ["crown", NAMES.crown]] as const) {
      bind(b.groups, "groups", `${slot}Shapes`, child(root, name), slot !== "crown");
    }
  }
  return b;
}

/** A short, readable account of what was and was not bound. */
export function describeBindings(b: Bindings): string {
  const count = (t: Record<string, unknown>) => Object.keys(t).length;
  const toggles = Object.entries(b.toggles).map(([slot, choices]) => `${slot} (${count(choices)})`);
  return [
    `bindings (${b.kind} template): art ${b.art ? "bound" : "MISSING"}, ${count(b.artRefs)} art frames, ${count(b.text)} text layers, ${count(b.textRefs)} references, ${count(b.layers)} single layers`,
    `  toggles: ${toggles.join(", ") || "none"}`,
    b.unresolved.length ? `  UNRESOLVED (edit "bindings" in manifest.json): ${b.unresolved.join(", ")}` : "  nothing unresolved",
  ].join("\n");
}
