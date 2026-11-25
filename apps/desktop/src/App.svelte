<script lang="ts">
  import type { Doc, Layer } from "@cardsmith/doc";
  import type { Brush, Core, CoreState, Edit, Rect } from "./lib/core";
  import { getCore } from "./lib/renderer";
  import { hasNative } from "./lib/native";
  import CardPanel from "./CardPanel.svelte";

  // The Rust core (WebAssembly) owns the document, its history and the pixels.
  // This component sends input and edits, and copies the part of the view the
  // core says has changed onto the canvas. The card panel builds cards into
  // the same document.

  let stage = $state<HTMLElement>();
  let canvas = $state<HTMLCanvasElement>();
  let core = $state.raw<Core>();
  let samples = $state<string[]>([]);
  let current = $state(0);
  let doc = $state.raw<Doc>();
  let history = $state.raw<CoreState>({ canUndo: false, canRedo: false, labels: [], warnings: [] });
  let active = $state<string>();
  let status = $state("Loading core…");
  let busy = $state(false);

  // The tool in hand: moving layers about, or painting.
  let tool = $state<"move" | "paint">("move");
  // The selected layer's box, in document pixels, for the outline and handles.
  let box = $state.raw<Rect | null>(null);
  // A text layer being typed into on the canvas.
  let typing = $state<{ id: string; text: string; box: Rect; size: number } | null>(null);
  // A card field the card panel should put the cursor in (after a double-click on its text on the canvas).
  let focusField = $state<string | null>(null);
  let imageInput = $state<HTMLInputElement>();

  // Brush settings. Size is in document pixels.
  let erase = $state(false);
  let size = $state(64);
  let hardness = $state(0.8);
  let opacity = $state(1);
  let color = $state("#c81e1e");

  // What part of the document is on screen: `scale` screen pixels per document pixel.
  let view = $state({ scale: 1, x: 0, y: 0 });
  let cursor = $state<{ x: number; y: number } | null>(null);
  const dpr = () => window.devicePixelRatio || 1;

  // Frame-time readout for the acceptance test: time in the core and time to reach the screen, per pointer move.
  let timings = $state.raw<{ core: number[]; blit: number[] }>({ core: [], blit: [] });
  let lastStroke = $state("");
  const stats = (values: number[]) => {
    if (!values.length) return "–";
    const sorted = [...values].sort((a, b) => a - b);
    const at = (q: number) => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * q))]!.toFixed(1);
    return `median ${at(0.5)} · 95th ${at(0.95)} · worst ${sorted[sorted.length - 1]!.toFixed(1)} ms`;
  };

  getCore()
    .then((loaded) => {
      core = loaded;
      samples = loaded.sampleNames();
      open(0);
    })
    .catch((err) => (status = `The core failed to load: ${err.message ?? err}`));

  // ---- drawing ------------------------------------------------------------------

  /** Copy part of the core's view (all of it by default) onto the canvas. */
  function blit(rect?: Rect | null) {
    if (!core || !canvas || !canvas.width || !canvas.height) return;
    // A window onto the core's memory: the only copy is the one onto the canvas.
    const image = new ImageData(core.viewPixels(), canvas.width, canvas.height);
    const ctx = canvas.getContext("2d")!;
    if (rect) ctx.putImageData(image, 0, 0, rect.x, rect.y, rect.width, rect.height);
    else ctx.putImageData(image, 0, 0);
  }

  /** Tell the core what is on screen and redraw everything. */
  function applyView() {
    if (!core || !canvas || !stage) return;
    const width = Math.max(1, Math.round(stage.clientWidth * dpr()));
    const height = Math.max(1, Math.round(stage.clientHeight * dpr()));
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
    }
    core.setView({ ...view, width, height });
    blit();
  }

  function fit() {
    if (!doc || !stage) return;
    const [w, h] = [stage.clientWidth * dpr(), stage.clientHeight * dpr()];
    const scale = Math.min(w / doc.width, h / doc.height) * 0.94;
    view = { scale, x: -(w / scale - doc.width) / 2, y: -(h / scale - doc.height) / 2 };
    applyView();
  }

  /** Re-read the document and history from the core. */
  function refresh() {
    if (!core) return;
    doc = core.doc();
    history = core.state();
    if (active && !find(active)) active = undefined;
    box = active ? core.layerBox(active) : null;
  }

  /** A layer by id, with the group it is in and its place there. */
  function find(id: string, group: Layer & { kind: "group" } = doc!.root as Layer & { kind: "group" }): { layer: Layer; parent: string; index: number } | null {
    for (const [index, layer] of group.children.entries()) {
      if (layer.id === id) return { layer, parent: group.id, index };
      if (layer.kind === "group") {
        const inner = find(id, layer);
        if (inner) return inner;
      }
    }
    return null;
  }

  function select(id: string | undefined) {
    active = id;
    box = id && core ? core.layerBox(id) : null;
  }

  function open(index: number) {
    if (!core) return;
    current = index;
    busy = true;
    status = `Opening ${samples[index]}…`;
    // Let the status paint before a large document blocks the thread.
    setTimeout(() => {
      const started = performance.now();
      core!.openSample(index);
      refresh();
      active = undefined;
      timings = { core: [], blit: [] };
      lastStroke = "";
      fit();
      busy = false;
      status = `${doc!.width}×${doc!.height} · opened and rendered in ${Math.round(performance.now() - started)} ms`;
    }, 30);
  }

  /** Load the chosen font files into the core. Text that was waiting for them is drawn. */
  async function loadFonts(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const files = [...(input.files ?? [])];
    input.value = "";
    const loaded: string[] = [];
    const refused: string[] = [];
    for (const file of files) {
      const bytes = new Uint8Array(await file.arrayBuffer());
      try {
        loaded.push(...core!.addFont(bytes));
      } catch {
        refused.push(file.name);
      }
    }
    blit(core!.dirty());
    refresh();
    status = `Loaded ${loaded.length} font${loaded.length === 1 ? "" : "s"}` + (refused.length ? `; not usable: ${refused.join(", ")}` : "");
  }

  /** The card panel changed the document: show what was redrawn. */
  function cardChanged() {
    blit(core!.dirty());
    refresh();
  }

  /** The card panel opened a template. */
  function cardOpened() {
    refresh();
    active = undefined;
    current = -1;
    fit();
  }

  /** Run something that changes the document, then show the result. */
  function run(label: string, action: () => void) {
    const started = performance.now();
    try {
      action();
    } catch (err) {
      status = `${label} refused: ${err instanceof Error ? err.message : err}`;
      return;
    }
    const inCore = performance.now() - started;
    blit(core!.dirty());
    refresh();
    status = `${label}: ${inCore.toFixed(1)} ms in the core, ${(performance.now() - started - inCore).toFixed(1)} ms to screen`;
  }

  const toggle = (layer: Layer) =>
    run(layer.visible ? "Hide" : "Show", () => core!.execute({ type: "setLayerProps", id: layer.id, props: { visible: !layer.visible } }, `${layer.visible ? "Hide" : "Show"} ${layer.name}`));

  function newPaintLayer() {
    run("New paint layer", () => (active = core!.addPaintLayer("Paint")));
  }

  // ---- pointer input --------------------------------------------------------------

  const toDoc = (e: PointerEvent | WheelEvent) => ({ x: view.x + (e.offsetX * dpr()) / view.scale, y: view.y + (e.offsetY * dpr()) / view.scale });

  /** A pointer event on something over the canvas, with its place measured from the stage like the canvas's own. */
  function stagePoint(e: PointerEvent): PointerEvent {
    const r = stage!.getBoundingClientRect();
    return { offsetX: e.clientX - r.left, offsetY: e.clientY - r.top, shiftKey: e.shiftKey, ctrlKey: e.ctrlKey } as PointerEvent;
  }

  let painting = false;
  let panning: { x: number; y: number } | null = null;
  let strokeCore: number[] = [];
  let strokeBlit: number[] = [];

  function brush(): Brush {
    const n = parseInt(color.slice(1), 16);
    return { size, hardness, opacity, erase, color: [(n >> 16) & 255, (n >> 8) & 255, n & 255] };
  }

  function strokePoints(events: PointerEvent[]) {
    if (!core) return;
    const started = performance.now();
    let dirty: Rect | null = null;
    for (const e of events) {
      const p = toDoc(e);
      // Mice report no pressure; pens and touch do.
      core.strokeTo(p.x, p.y, e.pointerType === "mouse" ? 1 : Math.max(e.pressure, 0.05));
      const d = core.dirty();
      if (!d) continue;
      if (!dirty) dirty = d;
      else {
        const x = Math.min(dirty.x, d.x), y = Math.min(dirty.y, d.y);
        dirty = { x, y, width: Math.max(dirty.x + dirty.width, d.x + d.width) - x, height: Math.max(dirty.y + dirty.height, d.y + d.height) - y };
      }
    }
    const inCore = performance.now() - started;
    if (dirty) blit(dirty);
    strokeCore.push(inCore);
    strokeBlit.push(performance.now() - started - inCore);
  }

  // ---- moving and scaling ------------------------------------------------------------

  type Drag = {
    id: string;
    name: string;
    /** Where the pointer went down, and the layer's box then, in document pixels. */
    from: { x: number; y: number };
    box: Rect | null;
    /** For a scale: the corner that stays put. */
    anchor?: { x: number; y: number };
    moved: boolean;
    /** Lines to snap to: the card's edges and middle, and the template's reference boxes. */
    snapX: number[];
    snapY: number[];
  };
  let drag: Drag | null = null;

  function snapLines(skip: string): { x: number[]; y: number[] } {
    const x = [0, doc!.width / 2, doc!.width];
    const y = [0, doc!.height / 2, doc!.height];
    for (const { layer } of layers) {
      if (layer.id === skip || !/reference/i.test(layer.name)) continue;
      const b = core!.layerBox(layer.id);
      if (!b) continue;
      x.push(b.x, b.x + b.width / 2, b.x + b.width);
      y.push(b.y, b.y + b.height / 2, b.y + b.height);
    }
    return { x, y };
  }

  /** How far to nudge a box so one of its edges or its middle lands on a nearby line. */
  function snap(at: number, size: number, lines: number[]): number {
    const reach = (6 * dpr()) / view.scale;
    let best = 0;
    let near = reach;
    for (const line of lines) {
      for (const edge of [at, at + size / 2, at + size]) {
        if (Math.abs(line - edge) < near) {
          near = Math.abs(line - edge);
          best = line - edge;
        }
      }
    }
    return best;
  }

  function beginDrag(e: PointerEvent, id: string, corner?: { x: number; y: number }) {
    const found = find(id);
    if (!found || !core) return;
    if (found.layer.locked?.all || found.layer.locked?.position) {
      status = `"${found.layer.name}" is locked in place.`;
      return;
    }
    const b = core.layerBox(id);
    const lines = snapLines(id);
    drag = { id, name: found.layer.name, from: toDoc(e), box: b, moved: false, snapX: lines.x, snapY: lines.y };
    if (corner && b) drag.anchor = { x: b.x + b.width - (corner.x - b.x), y: b.y + b.height - (corner.y - b.y) };
  }

  function dragTo(e: PointerEvent) {
    if (!drag || !core) return;
    const p = toDoc(e);
    let [dx, dy] = [p.x - drag.from.x, p.y - drag.from.y];
    if (!drag.moved && Math.hypot(dx, dy) * view.scale < 3 * dpr()) return;
    drag.moved = true;
    try {
      if (drag.anchor && drag.box) {
        // Scaling keeps the opposite corner where it is and the layer's proportions as they are.
        const was = Math.hypot(drag.from.x - drag.anchor.x, drag.from.y - drag.anchor.y);
        const scale = Math.max(0.01, Math.hypot(p.x - drag.anchor.x, p.y - drag.anchor.y) / Math.max(was, 1));
        core.drag(drag.id, scale, drag.anchor.x * (1 - scale), drag.anchor.y * (1 - scale));
      } else {
        // Shift keeps the move to one direction; Ctrl turns snapping off.
        if (e.shiftKey) Math.abs(dx) > Math.abs(dy) ? (dy = 0) : (dx = 0);
        if (drag.box && !e.ctrlKey) {
          dx += snap(drag.box.x + dx, drag.box.width, drag.snapX);
          dy += snap(drag.box.y + dy, drag.box.height, drag.snapY);
        }
        core.drag(drag.id, 1, Math.round(dx), Math.round(dy));
      }
    } catch (err) {
      status = err instanceof Error ? err.message : String(err);
      core.dragEnd(false);
      drag = null;
      return;
    }
    blit(core.dirty());
    box = core.layerBox(drag.id);
  }

  function endDrag(e?: PointerEvent) {
    if (!drag || !core) return;
    const done = drag;
    drag = null;
    if (done.moved) {
      core.dragEnd(true, `${done.anchor ? "Scale" : "Move"} ${done.name}`);
      blit(core.dirty());
      refresh();
    } else if (e && !done.anchor) {
      // A click without a drag picks what is under the pointer.
      select(core.hit(toDoc(e).x, toDoc(e).y) ?? undefined);
    }
  }

  function nudge(dx: number, dy: number) {
    if (!active || !core) return;
    run("Nudge", () => {
      core!.drag(active!, 1, dx, dy);
      core!.dragEnd(true, "Nudge");
    });
  }

  // ---- text on the canvas ---------------------------------------------------------------

  const CARD_FIELDS: Record<string, string> = { "card.name": "name", "card.mana": "mana_cost", "card.type": "type_line", "card.rules": "oracle_text", "card.pt": "power", "card.collector": "collector_number" };

  /** Start typing into a text layer: a card's own text in the card panel, any other where it sits. */
  function editText(id: string) {
    const layer = find(id)?.layer;
    if (!layer || layer.kind !== "text" || !core) return;
    select(id);
    const field = layer.role ? CARD_FIELDS[layer.role] : undefined;
    if (field && doc?.card) {
      focusField = field;
      return;
    }
    const b = core.layerBox(id) ?? { x: layer.box.x, y: layer.box.y, width: layer.box.w, height: layer.box.h };
    typing = { id, text: layer.content.runs.map((r) => r.text).join(""), box: { x: Math.min(b.x, layer.box.x), y: Math.min(b.y, layer.box.y), width: Math.max(b.width, layer.box.w), height: Math.max(b.height, layer.box.h) }, size: layer.style.size };
  }

  /** Put the cursor in the box as it appears, with its words selected, ready to be typed over. */
  function takeFocus(node: HTMLTextAreaElement) {
    node.focus();
    node.select();
  }

  function commitText(keep: boolean) {
    const done = typing;
    typing = null;
    const layer = done && find(done.id)?.layer;
    if (!keep || !done || !layer || layer.kind !== "text") return;
    if (done.text === layer.content.runs.map((r) => r.text).join("")) return;
    // The new words take the look of the first run; each line is a paragraph.
    const { text: _, ...look } = layer.content.runs[0] ?? { text: "" };
    run("Edit text", () => core!.execute({ type: "setLayerProps", id: done.id, props: { content: { runs: [{ ...look, text: done.text }] } } }, `Edit ${layer.name}`));
  }

  function doubleClick(e: MouseEvent) {
    if (!core || tool !== "move") return;
    const hit = core.hit(view.x + (e.offsetX * dpr()) / view.scale, view.y + (e.offsetY * dpr()) / view.scale);
    if (hit) editText(hit);
  }

  // ---- new layers ---------------------------------------------------------------------

  const newId = (kind: string) => `${kind}-${Date.now().toString(36)}`;
  const base = (id: string, name: string) => ({ id, name, visible: true, opacity: 1, blend: "normal", clip: false });

  /** Put a new layer just above the selected one, or on top. */
  function addLayer(layer: Record<string, unknown>, label: string, extra: Edit[] = []) {
    const above = active ? find(active) : null;
    const edit: Edit = { type: "addLayer", parent: above?.parent ?? doc!.root.id, layer, ...(above ? { index: above.index + 1 } : {}) };
    run(label, () => core!.execute(extra.length ? { type: "batch", edits: [...extra, edit] } : edit, label));
    select(layer.id as string);
  }

  function newText() {
    if (!core || !doc) return;
    const fonts = core.fontNames();
    if (!fonts.length) {
      status = "Load fonts first (Fonts… in the card panel or the toolbar): a text layer needs one.";
      return;
    }
    const id = newId("text");
    const size = Math.round(doc.height / 22);
    addLayer(
      { ...base(id, "Text"), kind: "text", box: { x: doc.width * 0.15, y: doc.height * 0.45, w: doc.width * 0.7, h: size * 1.6 }, content: { runs: [{ text: "Text" }] }, style: { font: fonts.find((f) => /beleren|plantin/i.test(f)) ?? fonts[0], size, color: [0, 0, 0, 1], align: "center" }, fit: "none" },
      "New text layer",
    );
    tool = "move";
    editText(id);
  }

  function newShape() {
    if (!doc) return;
    const [w, h] = [doc.width * 0.3, doc.height * 0.12];
    const [x, y] = [(doc.width - w) / 2, (doc.height - h) / 2];
    addLayer({ ...base(newId("shape"), "Rectangle"), kind: "shape", path: { d: `M${x} ${y}L${x + w} ${y}L${x + w} ${y + h}L${x} ${y + h}Z` }, fill: { type: "solid", color: [0.5, 0.5, 0.5, 1] } }, "New rectangle");
    tool = "move";
  }

  async function newImage(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file || !core || !doc) return;
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const picture = await createImageBitmap(file);
      const id = newId("image");
      // At its own size unless it is bigger than the card, in the middle.
      const scale = Math.min(1, doc.width / picture.width, doc.height / picture.height);
      core.addAsset(id, bytes, false);
      addLayer(
        { ...base(id, file.name.replace(/\.[^.]+$/, "")), kind: "pixel", asset: id, transform: [scale, 0, Math.round((doc.width - picture.width * scale) / 2), 0, scale, Math.round((doc.height - picture.height * scale) / 2)] },
        "New image layer",
        [{ type: "setAsset", id, asset: { kind: "image", path: `assets/${id}.png`, width: picture.width, height: picture.height } }],
      );
      tool = "move";
    } catch (err) {
      status = `That picture could not be added: ${err instanceof Error ? err.message : err}`;
    }
  }

  function removeSelected() {
    const found = active ? find(active) : null;
    if (!found) return;
    run("Delete layer", () => core!.execute({ type: "removeLayer", id: found.layer.id }, `Delete ${found.layer.name}`));
  }

  /** Change a property of the selected layer. */
  function setProp(props: Record<string, unknown>, label: string) {
    if (!active) return;
    run(label, () => core!.execute({ type: "setLayerProps", id: active!, props }, label));
  }

  const selected = $derived(active && doc ? (find(active)?.layer ?? null) : null);
  const hex = (c: number[]) => "#" + c.slice(0, 3).map((v) => Math.round(v * 255).toString(16).padStart(2, "0")).join("");
  const rgb = (h: string, a = 1) => [parseInt(h.slice(1, 3), 16) / 255, parseInt(h.slice(3, 5), 16) / 255, parseInt(h.slice(5, 7), 16) / 255, a];
  /** A document box as a place on screen, in CSS pixels. */
  const onScreen = (b: Rect) => ({ left: ((b.x - view.x) * view.scale) / dpr(), top: ((b.y - view.y) * view.scale) / dpr(), width: (b.width * view.scale) / dpr(), height: (b.height * view.scale) / dpr() });

  function pointerDown(e: PointerEvent) {
    if (!core || busy) return;
    canvas!.setPointerCapture(e.pointerId);
    if (e.button === 1 || e.button === 2 || e.altKey) {
      panning = { x: e.clientX, y: e.clientY };
      return;
    }
    if (e.button !== 0) return;
    if (tool === "move") {
      if (typing) commitText(true);
      // Drag moves the selected layer wherever the pointer is; with none selected, or with Ctrl, what is under it.
      const p = toDoc(e);
      const under = e.ctrlKey || !active ? core.hit(p.x, p.y) : null;
      if (under) select(under);
      if (active) beginDrag(e, active);
      return;
    }
    if (!active) {
      status = "Select a pixel layer to paint on, or add a paint layer.";
      return;
    }
    const started = performance.now();
    try {
      core.strokeBegin(active, brush());
    } catch (err) {
      status = err instanceof Error ? err.message : String(err);
      return;
    }
    painting = true;
    strokeCore = [];
    strokeBlit = [];
    lastStroke = `stroke start ${(performance.now() - started).toFixed(1)} ms`;
    strokePoints([e]);
  }

  function pointerMove(e: PointerEvent) {
    cursor = { x: e.offsetX, y: e.offsetY };
    if (panning) {
      view = { ...view, x: view.x - ((e.clientX - panning.x) * dpr()) / view.scale, y: view.y - ((e.clientY - panning.y) * dpr()) / view.scale };
      panning = { x: e.clientX, y: e.clientY };
      applyView();
    } else if (drag) {
      dragTo(e);
    } else if (painting) {
      // Browsers batch fast pointer movement; take every point so strokes stay smooth.
      const events = e.getCoalescedEvents?.() ?? [];
      strokePoints(events.length ? events : [e]);
    }
  }

  function pointerUp(e?: PointerEvent) {
    panning = null;
    endDrag(e);
    if (!painting || !core) return;
    painting = false;
    core.strokeEnd(erase ? "Eraser" : "Brush");
    timings = { core: strokeCore, blit: strokeBlit };
    refresh();
  }

  function wheel(e: WheelEvent) {
    e.preventDefault();
    if (!core) return;
    // Zoom about the pointer: the document point under it stays put.
    const before = toDoc(e);
    const scale = Math.min(16, Math.max(0.02, view.scale * Math.exp(-e.deltaY * 0.0015)));
    view = { scale, x: before.x - (e.offsetX * dpr()) / scale, y: before.y - (e.offsetY * dpr()) / scale };
    applyView();
  }

  function key(e: KeyboardEvent) {
    if (e.target instanceof HTMLInputElement && e.target.type !== "range" && e.target.type !== "checkbox") return;
    if (e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLSelectElement) return;
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === "z") {
      e.preventDefault();
      if (e.shiftKey) run("Redo", () => core!.redo());
      else run("Undo", () => core!.undo());
    } else if (mod && e.key.toLowerCase() === "y") {
      e.preventDefault();
      run("Redo", () => core!.redo());
    } else if (e.key === "Escape" && drag) {
      core!.dragEnd(false);
      drag = null;
      blit(core!.dirty());
      refresh();
    } else if ((e.key === "Delete" || e.key === "Backspace") && active && tool === "move") {
      e.preventDefault();
      removeSelected();
    } else if (e.key.startsWith("Arrow") && active && tool === "move") {
      e.preventDefault();
      const step = e.shiftKey ? 10 : 1;
      nudge(e.key === "ArrowLeft" ? -step : e.key === "ArrowRight" ? step : 0, e.key === "ArrowUp" ? -step : e.key === "ArrowDown" ? step : 0);
    } else if (e.key.toLowerCase() === "v") tool = "move";
    else if (e.key.toLowerCase() === "t" && !mod) newText();
    else if (e.key === "[") size = Math.max(1, Math.round(size / 1.25));
    else if (e.key === "]") size = Math.min(1000, Math.round(size * 1.25) + 1);
    else if (e.key.toLowerCase() === "e") (tool = "paint"), (erase = true);
    else if (e.key.toLowerCase() === "b") (tool = "paint"), (erase = false);
    else if (e.key === "0") fit();
  }

  // Top layer first, like every layer panel, with children under their group.
  const layers = $derived.by(() => {
    const list: { layer: Layer; depth: number }[] = [];
    const add = (children: Layer[], depth: number) => {
      for (let i = children.length - 1; i >= 0; i--) {
        const layer = children[i]!;
        list.push({ layer, depth });
        if (layer.kind === "group") add(layer.children, depth + 1);
      }
    };
    if (doc) add(doc.root.children, 0);
    return list;
  });

  // Keep the view in step with the window.
  $effect(() => {
    if (!stage) return;
    const observer = new ResizeObserver(() => applyView());
    observer.observe(stage);
    return () => observer.disconnect();
  });

  const cursorSize = $derived((size * view.scale) / dpr());
</script>

<svelte:window onkeydown={key} />

<main class:with-card={hasNative}>
  <header>
    <strong>Cardsmith</strong>
    {#each samples as sample, i}
      <button class:active={i === current} disabled={busy} onclick={() => open(i)}>{sample}</button>
    {/each}
    <label class="file" title="Load font files (TTF, OTF) so text layers can be drawn. Fonts are never stored in documents.">
      Fonts… <input type="file" accept=".ttf,.otf,.ttc" multiple onchange={loadFonts} />
    </label>
    <span class="divider"></span>
    <button class:active={tool === "move"} onclick={() => (tool = "move")} title="Move and scale layers (V)">Move</button>
    <button class:active={tool === "paint" && !erase} onclick={() => ((tool = "paint"), (erase = false))} title="Brush (B)">Brush</button>
    <button class:active={tool === "paint" && erase} onclick={() => ((tool = "paint"), (erase = true))} title="Eraser (E)">Eraser</button>
    <label title="Brush size in document pixels ([ and ])">Size <input type="range" min="1" max="800" bind:value={size} /> <output>{size}</output></label>
    <label>Hardness <input type="range" min="0" max="1" step="0.05" bind:value={hardness} /> <output>{Math.round(hardness * 100)}%</output></label>
    <label>Opacity <input type="range" min="0.05" max="1" step="0.05" bind:value={opacity} /> <output>{Math.round(opacity * 100)}%</output></label>
    <input type="color" bind:value={color} title="Brush color" disabled={erase} />
    <span class="spacer"></span>
    <button onclick={fit} title="Fit to window (0)">Fit</button>
    <output class="zoom">{Math.round(view.scale * 100)}%</output>
    <button disabled={!history.canUndo} onclick={() => run("Undo", () => core!.undo())}>Undo</button>
    <button disabled={!history.canRedo} onclick={() => run("Redo", () => core!.redo())}>Redo</button>
  </header>

  {#if hasNative && core}
    <CardPanel {core} bind:busy bind:focusField say={(text) => (status = text)} changed={cardChanged} opened={cardOpened} />
  {/if}

  <section class="stage" bind:this={stage}>
    <canvas
      bind:this={canvas}
      onpointerdown={pointerDown}
      onpointermove={pointerMove}
      onpointerup={pointerUp}
      onpointercancel={() => pointerUp()}
      ondblclick={doubleClick}
      class:moving={tool === "move"}
      onpointerleave={() => (cursor = null)}
      onwheel={wheel}
      oncontextmenu={(e) => e.preventDefault()}
    ></canvas>
    {#if tool === "move" && box && !typing}
      {@const at = onScreen(box)}
      <div class="outline" style:left="{at.left}px" style:top="{at.top}px" style:width="{at.width}px" style:height="{at.height}px">
        {#each [{ hx: 0, hy: 0 }, { hx: 1, hy: 0 }, { hx: 0, hy: 1 }, { hx: 1, hy: 1 }] as { hx, hy }}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="handle"
            style:left="{hx * 100}%"
            style:top="{hy * 100}%"
            style:cursor={hx === hy ? "nwse-resize" : "nesw-resize"}
            onpointerdown={(e) => {
              e.stopPropagation();
              (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
              beginDrag(stagePoint(e), active!, { x: box!.x + hx * box!.width, y: box!.y + hy * box!.height });
            }}
            onpointermove={(e) => drag && dragTo(stagePoint(e))}
            onpointerup={() => endDrag()}
          ></div>
        {/each}
      </div>
    {/if}
    {#if typing}
      {@const at = onScreen(typing.box)}
      <textarea
        class="typing"
        use:takeFocus
        style:left="{at.left}px"
        style:top="{at.top}px"
        style:width="{Math.max(at.width, 160)}px"
        style:height="{Math.max(at.height, 40)}px"
        style:font-size="{Math.max(12, (typing.size * view.scale) / dpr())}px"
        bind:value={typing.text}
        onblur={() => commitText(true)}
        onkeydown={(e) => {
          e.stopPropagation();
          if (e.key === "Escape") commitText(false);
          else if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) commitText(true);
        }}
      ></textarea>
    {/if}
    {#if cursor && active && tool === "paint"}
      <div class="brush" style:left="{cursor.x}px" style:top="{cursor.y}px" style:width="{cursorSize}px" style:height="{cursorSize}px"></div>
    {/if}
  </section>

  <aside>
    <h2>Layers</h2>
    <div class="new">
      <button class="small" onclick={newText} disabled={busy} title="New text layer (T)">+ Text</button>
      <button class="small" onclick={newShape} disabled={busy} title="New rectangle">+ Shape</button>
      <button class="small" onclick={() => imageInput?.click()} disabled={busy} title="New layer from a picture file">+ Image</button>
      <button class="small" onclick={newPaintLayer} disabled={busy} title="New empty layer to paint on">+ Paint</button>
      <input bind:this={imageInput} type="file" accept=".png,.jpg,.jpeg,.webp" hidden onchange={newImage} />
    </div>
    {#if selected}
      <div class="props">
        <label>Opacity <input type="range" min="0" max="1" step="0.01" value={selected.opacity} onchange={(e) => setProp({ opacity: Number(e.currentTarget.value) }, "Opacity")} /> <output>{Math.round(selected.opacity * 100)}%</output></label>
        <label
          >Blend
          <select value={selected.blend} onchange={(e) => setProp({ blend: e.currentTarget.value }, "Blend mode")}>
            {#each ["normal", "multiply", "screen", "overlay", "softLight", "linearLight", "luminosity"] as mode}<option value={mode}>{mode}</option>{/each}
          </select>
        </label>
        {#if selected.kind === "text"}
          {@const text = selected}
          <label
            >Font
            <select value={text.style.font} onchange={(e) => setProp({ style: { ...text.style, font: e.currentTarget.value } }, "Font")}>
              {#each core?.fontNames() ?? [] as font}<option value={font}>{font}</option>{/each}
            </select>
          </label>
          <label>
            Size <input class="number" type="number" min="1" value={Math.round(text.style.size)} onchange={(e) => setProp({ style: { ...text.style, size: Math.max(1, Number(e.currentTarget.value)) } }, "Text size")} />
            <input type="color" value={hex(text.style.color)} onchange={(e) => setProp({ style: { ...text.style, color: rgb(e.currentTarget.value, text.style.color[3]) } }, "Text color")} />
            <select value={text.style.align} onchange={(e) => setProp({ style: { ...text.style, align: e.currentTarget.value } }, "Alignment")}>
              {#each ["left", "center", "right"] as align}<option value={align}>{align}</option>{/each}
            </select>
          </label>
          <button class="small" onclick={() => editText(text.id)}>Edit the text…</button>
        {/if}
        {#if selected.kind === "shape" && selected.fill.type === "solid"}
          {@const shape = selected}
          <label>Color <input type="color" value={hex(shape.fill.type === "solid" ? shape.fill.color : [0, 0, 0, 1])} onchange={(e) => setProp({ fill: { type: "solid", color: rgb(e.currentTarget.value) } }, "Shape color")} /></label>
        {/if}
        <button class="small" onclick={removeSelected} title="Delete this layer (Del)">Delete layer</button>
      </div>
    {/if}
    <ul>
      {#each layers as { layer, depth } (layer.id)}
        <li style:padding-left="{depth * 14}px" class:hidden={!layer.visible} class:selected={layer.id === active}>
          <input type="checkbox" checked={layer.visible} onchange={() => toggle(layer)} title="Visible" />
          <button class="name" onclick={() => select(layer.id)} ondblclick={() => editText(layer.id)} title={layer.kind === "text" ? "Double-click to edit the text" : "Select this layer"}>
            {layer.name}
            <small>{layer.kind}{layer.clip ? " · clipped" : ""}</small>
          </button>
        </li>
      {/each}
    </ul>
    {#if history.labels.length}
      <h2>History</h2>
      <ol>
        {#each history.labels.slice(-12) as label}<li>{label}</li>{/each}
      </ol>
    {/if}
  </aside>

  <footer>
    <span>{status}</span>
    {#if timings.core.length}
      <span class="perf">
        Last stroke, per pointer move ({timings.core.length}): core {stats(timings.core)} · to screen {stats(timings.blit)} · {lastStroke}
      </span>
    {/if}
    {#each history.warnings as warning}<span class="warning">{warning}</span>{/each}
    <span class="hint">V: move (drag moves the selected layer, click picks one, Ctrl-drag takes what is under the pointer) · double-click text to edit · T: new text · arrows: nudge · Wheel: zoom · Alt-drag: pan · B/E: brush/eraser · Ctrl+Z: undo</span>
  </footer>
</main>

<style>
  main {
    display: grid;
    grid-template: "header header" auto "stage aside" 1fr "footer footer" auto / 1fr 270px;
    height: 100vh;
  }
  main.with-card {
    grid-template: "header header header" auto "card stage aside" 1fr "footer footer footer" auto / 300px 1fr 270px;
  }
  header {
    grid-area: header;
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    padding: 6px 12px;
    background: #2b2d31;
    border-bottom: 1px solid #111;
    font-size: 13px;
  }
  .spacer { flex: 1; }
  .divider { width: 1px; align-self: stretch; background: #50545c; margin-inline: 4px; }
  label { display: flex; align-items: center; gap: 4px; color: #b5b7bd; }
  input[type="range"] { width: 70px; }
  label.file { cursor: pointer; }
  label.file input { display: none; }
  output { min-width: 2.6em; color: #e6e6e6; font-variant-numeric: tabular-nums; }
  .zoom { text-align: right; }
  button {
    background: #3a3d43;
    color: inherit;
    border: 1px solid #50545c;
    border-radius: 4px;
    padding: 4px 10px;
    font: inherit;
  }
  button.active { background: #5865f2; border-color: #5865f2; }
  button:disabled { opacity: 0.4; }
  .stage {
    grid-area: stage;
    position: relative;
    overflow: hidden;
    /* Checkerboard, so transparency and the area around the document are visible. */
    background: repeating-conic-gradient(#3a3a3a 0% 25%, #2e2e2e 0% 50%) 0 0 / 20px 20px;
  }
  canvas { position: absolute; inset: 0; width: 100%; height: 100%; touch-action: none; cursor: crosshair; }
  canvas.moving { cursor: default; }
  .outline { position: absolute; border: 1px solid #5b9dff; outline: 1px solid rgba(0, 0, 0, 0.5); pointer-events: none; box-sizing: border-box; }
  .handle { position: absolute; width: 10px; height: 10px; margin: -5px 0 0 -5px; background: #fff; border: 1px solid #1f5fd0; pointer-events: auto; touch-action: none; }
  .typing { position: absolute; background: rgba(255, 255, 255, 0.92); color: #111; border: 1px solid #5b9dff; padding: 2px 4px; resize: both; font-family: inherit; z-index: 2; }
  .new { display: flex; flex-wrap: wrap; gap: 4px; margin-bottom: 8px; }
  .props { display: flex; flex-direction: column; gap: 5px; padding: 6px; margin-bottom: 8px; background: #1e1f22; border-radius: 4px; font-size: 12px; }
  .props select, .props input[type="number"] { background: #2b2d31; color: inherit; border: 1px solid #50545c; border-radius: 4px; padding: 2px 4px; font: inherit; min-width: 0; flex: 1; }
  .props .number { width: 4em; flex: none; }
  .brush {
    position: absolute;
    border: 1px solid #fff;
    outline: 1px solid #000;
    border-radius: 50%;
    transform: translate(-50%, -50%);
    pointer-events: none;
  }
  aside {
    grid-area: aside;
    overflow: auto;
    background: #26282c;
    border-left: 1px solid #111;
    padding: 8px 10px;
  }
  h2 {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: #9a9ca3;
    margin: 4px 0 8px;
  }
  .small { font-size: 11px; padding: 2px 8px; text-transform: none; letter-spacing: 0; }
  ul, ol { list-style: none; margin: 0 0 12px; padding: 0; font-size: 13px; }
  li { display: flex; align-items: center; gap: 4px; padding-block: 1px; border-radius: 4px; }
  li.selected { background: #3d4270; }
  li.hidden .name, ol li { color: #777; }
  .name { flex: 1; text-align: left; background: none; border: none; padding: 2px 4px; }
  small { color: #777; margin-left: 4px; }
  footer {
    grid-area: footer;
    display: flex;
    flex-wrap: wrap;
    gap: 4px 16px;
    padding: 4px 12px;
    font-size: 12px;
    color: #9a9ca3;
    background: #2b2d31;
    border-top: 1px solid #111;
    font-variant-numeric: tabular-nums;
  }
  .perf { color: #7ee0a1; }
  .warning { color: #f0b232; }
  .hint { margin-left: auto; color: #6d7078; }
</style>
