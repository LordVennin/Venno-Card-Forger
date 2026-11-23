import { strToU8, zipSync } from "fflate";
import { describe, expect, it } from "vitest";
import {
  History, addLayer, batch, emptyDoc, findLayer, group, loadCardsmith, moveLayer, parseDoc, pixel, rectPath,
  removeLayer, saveCardsmith, setAsset, setDocProps, setLayerProps, shape, stringifyDoc, validateDoc, type Command, type Doc,
} from "./index.js";

function sample(): Doc {
  const doc = emptyDoc(100, 140, 800);
  doc.assets.art = { kind: "image", path: "assets/art.png", width: 2, height: 2 };
  doc.root.children = [
    shape({ id: "bg", d: rectPath(0, 0, 100, 140), color: [0, 0, 0, 1] }),
    group({ id: "frame" }, [
      pixel({ id: "art", asset: "art" }),
      shape({ id: "pin", d: rectPath(5, 5, 90, 2), color: [1, 1, 1, 1], clip: true }),
    ]),
    group({ id: "text", passThrough: false }, []),
  ];
  return doc;
}

/** Every command must restore the exact document on undo and repeat exactly on redo. */
function roundTrip(command: Command, doc = sample()) {
  const before = structuredClone(doc);
  const history = new History(doc);
  history.execute(command);
  const after = structuredClone(doc);
  expect(after).not.toEqual(before);
  expect(history.undo()).toBe(true);
  expect(doc).toEqual(before);
  expect(history.redo()).toBe(true);
  expect(doc).toEqual(after);
  history.undo();
  expect(doc).toEqual(before);
  return after;
}

describe("commands", () => {
  it("setLayerProps patches, removes and restores properties", () => {
    const after = roundTrip(setLayerProps("art", { opacity: 0.5, blend: "overlay", role: "art" }));
    expect(findLayer(after, "art")).toMatchObject({ opacity: 0.5, blend: "overlay", role: "art" });

    const doc = sample();
    const history = new History(doc);
    history.execute(setLayerProps("art", { role: "art" }));
    history.execute(setLayerProps("art", { role: undefined }));
    expect("role" in findLayer(doc, "art")!).toBe(false);
    history.undo();
    expect(findLayer(doc, "art")!.role).toBe("art");
  });

  it("setLayerProps does not alias the caller's objects", () => {
    const doc = sample();
    const effects = [{ type: "colorOverlay" as const, enabled: true, blend: "normal" as const, opacity: 1, color: [1, 0, 0, 1] as [number, number, number, number] }];
    new History(doc).execute(setLayerProps("art", { effects }));
    effects[0]!.opacity = 0;
    expect(findLayer(doc, "art")!.effects![0]).toMatchObject({ opacity: 1 });
  });

  it("setLayerProps refuses structural keys and unknown layers", () => {
    expect(() => setLayerProps("art", { id: "x" })).toThrow(/cannot be changed/);
    expect(() => setLayerProps("art", { children: [] })).toThrow(/cannot be changed/);
    expect(() => new History(sample()).execute(setLayerProps("nope", { opacity: 0 }))).toThrow(/No layer/);
  });

  it("addLayer inserts at the top by default or at an index", () => {
    const top = roundTrip(addLayer("frame", shape({ id: "new", d: "M0 0", color: [1, 0, 0, 1] })));
    expect((findLayer(top, "frame") as { children: { id: string }[] }).children.map((c) => c.id)).toEqual(["art", "pin", "new"]);
    const bottom = roundTrip(addLayer("root", group({ id: "g" }, [shape({ id: "inner", d: "M0 0", color: [1, 0, 0, 1] })]), 0));
    expect(bottom.root.children[0]!.id).toBe("g");
    expect(findLayer(bottom, "inner")).toBeDefined();
  });

  it("addLayer rejects duplicate ids, including nested ones, and bad targets", () => {
    const run = (c: Command) => new History(sample()).execute(c);
    expect(() => run(addLayer("root", shape({ id: "art", d: "", color: [0, 0, 0, 1] })))).toThrow(/already in the document/);
    expect(() => run(addLayer("root", group({ id: "g" }, [shape({ id: "pin", d: "", color: [0, 0, 0, 1] })])))).toThrow(/already/);
    expect(() => run(addLayer("art", shape({ id: "z", d: "", color: [0, 0, 0, 1] })))).toThrow(/not a group/);
    expect(() => run(addLayer("root", shape({ id: "z", d: "", color: [0, 0, 0, 1] }), 9))).toThrow(/out of range/);
  });

  it("removeLayer takes the subtree out and puts it back in place", () => {
    const after = roundTrip(removeLayer("frame"));
    expect(findLayer(after, "art")).toBeUndefined();
    expect(after.root.children.map((c) => c.id)).toEqual(["bg", "text"]);
    expect(() => new History(sample()).execute(removeLayer("root"))).toThrow(/root group/);
  });

  it("moveLayer reorders within a group and moves between groups", () => {
    const reordered = roundTrip(moveLayer("bg", "root", 2));
    expect(reordered.root.children.map((c) => c.id)).toEqual(["frame", "text", "bg"]);
    const moved = roundTrip(moveLayer("art", "text", 0));
    expect((findLayer(moved, "text") as { children: { id: string }[] }).children.map((c) => c.id)).toEqual(["art"]);
    expect((findLayer(moved, "frame") as { children: { id: string }[] }).children.map((c) => c.id)).toEqual(["pin"]);
  });

  it("moveLayer refuses to put a group inside itself or out of range", () => {
    const doc = sample();
    new History(doc).execute(addLayer("frame", group({ id: "inner" }, [])));
    const before = structuredClone(doc);
    expect(() => new History(doc).execute(moveLayer("frame", "inner", 0))).toThrow(/into itself/);
    expect(() => new History(doc).execute(moveLayer("frame", "frame", 0))).toThrow(/into itself/);
    expect(() => new History(doc).execute(moveLayer("bg", "root", 3))).toThrow(/out of range/);
    expect(doc).toEqual(before);
  });

  it("setAsset and setDocProps round-trip", () => {
    roundTrip(setAsset("mask", { kind: "image", path: "assets/mask.png" }));
    roundTrip(setAsset("art", { kind: "image", path: "assets/other.png" }));
    const after = roundTrip(setDocProps({ templateId: "normal", card: { name: "Test" } }));
    expect(after.card).toEqual({ name: "Test" });
  });

  it("batch is one undo step and rolls back when a step fails", () => {
    const after = roundTrip(batch([setLayerProps("art", { visible: false }), removeLayer("pin"), setLayerProps("bg", { opacity: 0.2 })], "Toggle"));
    expect(findLayer(after, "pin")).toBeUndefined();

    const doc = sample();
    const before = structuredClone(doc);
    const history = new History(doc);
    expect(() => history.execute(batch([setLayerProps("art", { visible: false }), removeLayer("missing")], "Bad"))).toThrow();
    expect(doc).toEqual(before);
    expect(history.canUndo).toBe(false);
  });
});

describe("History", () => {
  it("tracks labels, clears redo on a new edit, and notifies subscribers", () => {
    const history = new History(sample());
    let calls = 0;
    const off = history.subscribe(() => calls++);
    history.execute(setLayerProps("art", { opacity: 0.1 }, "Fade art"));
    history.execute(removeLayer("pin"));
    expect(history.labels).toEqual(["Fade art", "Delete layer"]);
    history.undo();
    expect(history.canRedo).toBe(true);
    history.execute(setLayerProps("bg", { visible: false }));
    expect(history.canRedo).toBe(false);
    expect(history.redo()).toBe(false);
    expect(calls).toBe(4);
    off();
    history.undo();
    expect(calls).toBe(4);
  });

  it("returns false when there is nothing to undo, and caps its length", () => {
    const history = new History(sample(), 2);
    expect(history.undo()).toBe(false);
    for (const o of [0.1, 0.2, 0.3]) history.execute(setLayerProps("art", { opacity: o }));
    expect(history.labels).toHaveLength(2);
  });
});

describe("serialization", () => {
  it("round-trips a document through JSON", () => {
    const doc = sample();
    expect(parseDoc(stringifyDoc(doc))).toEqual(doc);
  });

  it("rejects malformed documents with a reason", () => {
    const bad = (edit: (d: any) => void) => () => {
      const d = sample();
      edit(d);
      validateDoc(d);
    };
    expect(bad((d) => (d.version = 2))).toThrow(/unsupported version 2/);
    expect(bad((d) => (d.width = 0))).toThrow(/"width"/);
    expect(bad((d) => (d.root.children[0].blend = "dissolve"))).toThrow(/unsupported blend mode "dissolve"/);
    expect(bad((d) => (d.root.children[0].id = "frame"))).toThrow(/duplicate layer id "frame"/);
    expect(bad((d) => delete d.assets.art)).toThrow(/missing asset "art"/);
    expect(bad((d) => (d.assets.art.path = "../secret.png"))).toThrow(/unsafe path/);
    expect(bad((d) => (d.root.children[0].kind = "smart"))).toThrow(/unknown kind/);
    expect(() => parseDoc("{nope")).toThrow(/not JSON/);
  });

  it("saves and loads a .cardsmith file with its assets", () => {
    const doc = sample();
    const art = new Uint8Array([137, 80, 78, 71, 1, 2, 3]);
    const bytes = saveCardsmith(doc, { art });
    expect([...bytes.slice(0, 2)]).toEqual([0x50, 0x4b]); // "PK": it is a zip
    const loaded = loadCardsmith(bytes);
    expect(loaded.doc).toEqual(doc);
    expect([...loaded.assets.art!]).toEqual([...art]);
  });

  it("refuses to save or load with assets missing, and rejects non-zips", () => {
    const doc = sample();
    expect(() => saveCardsmith(doc, {})).toThrow(/no data for asset "art"/);
    expect(() => loadCardsmith(new Uint8Array([1, 2, 3]))).toThrow(/not a zip/);
    expect(() => loadCardsmith(zipSync({ "other.txt": strToU8("x") }))).toThrow(/no doc.json/);
    expect(() => loadCardsmith(zipSync({ "doc.json": strToU8(stringifyDoc(doc)) }))).toThrow(/missing asset "art"/);
  });
});
