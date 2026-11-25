<script lang="ts">
  import type { Doc, Layer } from "@cardsmith/doc";
  import type { Brush, Core, CoreState, Edit, Rect } from "./lib/core";
  import { getCore } from "./lib/renderer";
  import { hasNative, native } from "./lib/native";
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
  let history = $state.raw<CoreState>({ canUndo: false, canRedo: false, labels: [], redoLabels: [], warnings: [] });
  let active = $state<string>();
  let status = $state("Loading core…");
  let busy = $state(false);

  // The tool in hand: moving layers about, or painting.
  let tool = $state<"move" | "paint" | "pick" | "select" | "magic">("move");
  // The eraser can also bring back what it took (on layers erased through a mask).
  let restore = $state(false);
  // The magic eraser: how alike a color must be to go, and whether only the part touching the click goes.
  let tolerance = $state(32);
  let contiguous = $state(true);
  // A rectangle marked out with the selection tool, in document pixels.
  let selection = $state.raw<Rect | null>(null);
  let selecting: { x: number; y: number } | null = null;
  // Drawn freehand instead: the points of the outline, in document pixels. The rectangle is then its bounds.
  let lasso = $state(false);
  let outline = $state.raw<{ x: number; y: number }[] | null>(null);
  let fillTier = $state("auto");
  let fillSeed = 1;
  // The selected layer's box, in document pixels, for the outline and handles.
  let box = $state.raw<Rect | null>(null);
  // A text layer being typed into on the canvas.
  let typing = $state<{ id: string; text: string; box: Rect; size: number } | null>(null);
  // A card field the card panel should put the cursor in (after a double-click on its text on the canvas).
  let focusField = $state<string | null>(null);
  let imageInput = $state<HTMLInputElement>();
  // The loaded fonts' names, read when the document is re-read, not every time the panel is drawn.
  let fonts = $state.raw<string[]>([]);
  // The card panel, which fetches a template's images as layers come to need them.
  let panel = $state<{ loadImages(): Promise<void> }>();

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
      loaded.setDirect(true);
      samples = loaded.sampleNames();
      open(0);
    })
    .catch((err) => (status = `The core failed to load: ${err.message ?? err}`));

  // ---- drawing ------------------------------------------------------------------

  // The core hands over its frame (the real one, or a smaller copy when zoomed out) and says
  // which part changed. That part is copied into a canvas of the frame's own size, kept off
  // screen, and the canvas on screen draws that one scaled and moved. Panning and zooming
  // are then a redraw of the canvas: the core does nothing.
  // One canvas for each size of frame the core has handed over (the real one and its smaller
  // copies): going back to one, the core sends only what changed in it since.
  const backings = new Map<number, HTMLCanvasElement>();
  let backing: HTMLCanvasElement | undefined;
  let per = 1;
  let painting_due = 0;

  /** Take in what the core changed and show it. The argument is ignored; it is there for callers that pass the old dirty rectangle. */
  function blit(_?: Rect | null) {
    if (!core || !canvas) return;
    const shown = core.display();
    per = shown.per;
    backing = backings.get(per);
    if (!backing) backings.set(per, (backing = document.createElement("canvas")));
    const resized = backing.width !== shown.width || backing.height !== shown.height;
    if (resized) {
      backing.width = shown.width;
      backing.height = shown.height;
    }
    // A canvas that was resized is blank: it needs the whole frame, not only what changed.
    const dirty = resized ? { x: 0, y: 0, width: shown.width, height: shown.height } : shown.dirty;
    if (dirty) {
      // A window onto the core's memory: the only copy is the one into the canvas.
      backing.getContext("2d")!.putImageData(new ImageData(shown.pixels, shown.width, shown.height), 0, 0, dirty.x, dirty.y, dirty.width, dirty.height);
    }
    painting_due ||= requestAnimationFrame(paint);
  }

  /** Draw the frame onto the screen at the view's zoom and place. */
  function paint() {
    painting_due = 0;
    if (!canvas || !backing || !backing.width) return;
    const ctx = canvas.getContext("2d")!;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    // One pixel of the frame is `per` document pixels, and a document pixel is `view.scale` screen pixels.
    const zoom = view.scale * per;
    // Smoothed when the frame is shrunk or only a little enlarged; crisp pixels when zoomed well in.
    ctx.imageSmoothingEnabled = zoom < 3;
    ctx.imageSmoothingQuality = "high";
    ctx.setTransform(zoom, 0, 0, zoom, -view.x * view.scale, -view.y * view.scale);
    ctx.drawImage(backing, 0, 0);
  }

  /** Tell the core what is on screen (so it knows what to draw, and at what size) and show it. */
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
    warmSoon();
  }

  /** After a drag that ended zoomed in: bring the real pixels in a slice a frame, so letting go does not stall. */
  function refine() {
    if (!core) return;
    const done = core.refine(160_000);
    blit();
    if (!done) requestAnimationFrame(refine);
  }

  // When nothing has happened for a moment, the smaller copy a drag would use is made ready.
  let warming: ReturnType<typeof setTimeout> | undefined;
  function warmSoon() {
    clearTimeout(warming);
    warming = setTimeout(() => !busy && !drag && !painting && core?.warm(), 700);
  }

  // Panning and zooming arrive faster than the screen redraws: the view is set once a frame.
  let view_due = 0;
  function viewChanged() {
    view_due ||= requestAnimationFrame(() => {
      view_due = 0;
      applyView();
    });
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
    fonts = core.fontNames();
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

  /** Write the card, drawn at full size, to a PNG file of the user's choosing. */
  async function exportPng() {
    if (!core || !doc || busy) return;
    const name = String((doc.card as { name?: string } | undefined)?.name ?? "card").replace(/[\\/:*?"<>|]/g, "") || "card";
    busy = true;
    try {
      const started = performance.now();
      const path = await native.savePng(`${name}.png`, () => {
        status = "Drawing the card at full size…";
        return core!.exportPng();
      });
      status = path ? `Exported ${doc.width}×${doc.height} to ${path} in ${((performance.now() - started) / 1000).toFixed(1)} s` : "Export cancelled";
    } catch (err) {
      status = `The card could not be exported: ${err instanceof Error ? err.message : err}`;
    } finally {
      busy = false;
    }
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
    // A layer just shown may need a picture that has not been loaded yet.
    panel?.loadImages();
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
    return { size, hardness, opacity, erase, restore: erase && restore, color: [(n >> 16) & 255, (n >> 8) & 255, n & 255] };
  }

  // Pointer positions arrive faster than the screen redraws. They are kept here and the core is
  // asked to paint (or to move the dragged layer) once a frame, so slow redraws never pile up.
  let waiting: [number, number, number][] = [];
  let frame = 0;
  let dragPoint: PointerEvent | null = null;

  function onFrame() {
    frame = 0;
    if (!core) return;
    if (waiting.length) {
      const points = waiting;
      waiting = [];
      const started = performance.now();
      core.strokeThrough(points);
      const inCore = performance.now() - started;
      blit(core.dirty());
      strokeCore.push(inCore);
      strokeBlit.push(performance.now() - started - inCore);
    }
    if (dragPoint) {
      const e = dragPoint;
      dragPoint = null;
      dragTo(e);
    }
  }

  /** Do what is waiting now, without waiting for the next frame: before a stroke or a drag ends. */
  function flush() {
    if (frame) cancelAnimationFrame(frame);
    onFrame();
  }

  function strokePoints(events: PointerEvent[]) {
    for (const e of events) {
      const p = toDoc(e);
      // Mice report no pressure; pens and touch do.
      waiting.push([p.x, p.y, e.pointerType === "mouse" ? 1 : Math.max(e.pressure, 0.05)]);
    }
    frame ||= requestAnimationFrame(onFrame);
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
    /** For a turn: the point turned about (the middle of the box). */
    pivot?: { x: number; y: number };
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

  function beginDrag(e: PointerEvent, id: string, corner?: { x: number; y: number }, turning = false) {
    const found = find(id);
    if (!found || !core) return;
    if (found.layer.locked?.all || found.layer.locked?.position) {
      status = `"${found.layer.name}" is locked in place.`;
      return;
    }
    const b = core.layerBox(id);
    const lines = snapLines(id);
    drag = { id, name: found.layer.name, from: toDoc(e), box: b, moved: false, snapX: lines.x, snapY: lines.y };
    if (turning && b) drag.pivot = { x: b.x + b.width / 2, y: b.y + b.height / 2 };
    else if (corner && b) drag.anchor = { x: b.x + b.width - (corner.x - b.x), y: b.y + b.height - (corner.y - b.y) };
  }

  function dragTo(e: PointerEvent) {
    if (!drag || !core) return;
    const p = toDoc(e);
    let [dx, dy] = [p.x - drag.from.x, p.y - drag.from.y];
    if (!drag.moved && Math.hypot(dx, dy) * view.scale < 3 * dpr()) return;
    drag.moved = true;
    try {
      if (drag.pivot) {
        // Turning about the middle; Shift keeps to steps of fifteen degrees.
        const angle = (q: { x: number; y: number }) => Math.atan2(q.y - drag!.pivot!.y, q.x - drag!.pivot!.x);
        let turn = angle(p) - angle(drag.from);
        if (e.shiftKey) turn = Math.round(turn / (Math.PI / 12)) * (Math.PI / 12);
        const [sin, cos] = [Math.sin(turn), Math.cos(turn)];
        const { x, y } = drag.pivot;
        core.drag(drag.id, 1, x - (cos * x - sin * y), y - (sin * x + cos * y), turn);
      } else if (drag.anchor && drag.box) {
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
      refine();
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
      core.dragEnd(true, `${done.pivot ? "Turn" : done.anchor ? "Scale" : "Move"} ${done.name}`);
      refresh();
      refine();
    } else if (e && !done.anchor && !done.pivot) {
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
    refine();
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

  // ---- the layers list ---------------------------------------------------------------

  /** Go back or forward through history to a step: `steps` undos if negative, redos if positive. */
  function travel(steps: number) {
    run(steps < 0 ? "Undo" : "Redo", () => {
      for (let i = 0; i < Math.abs(steps); i++) steps < 0 ? core!.undo() : core!.redo();
    });
  }

  // Reordering in the list: a row is picked up by its grip and dropped on another row. Done with
  // plain pointer events, because the desktop window keeps the browser's own drag and drop for itself.
  let dragged = $state<string | null>(null);
  let dropAt = $state<{ id: string; where: "above" | "below" | "inside" } | null>(null);

  // A press on a row becomes a drag once the pointer has moved a little; until then it is a
  // click, which selects. The moves and the release are listened for on the window, so the
  // drag carries on wherever the pointer goes, and the list scrolls when it nears an edge.
  let pressed: { id: string; x: number; y: number } | null = null;
  let listPanel = $state<HTMLElement>();
  let scrolling = 0;

  function rowDown(e: PointerEvent, id: string) {
    // Not from the tick box, and not with a button other than the main one.
    if (e.button !== 0 || (e.target as HTMLElement).closest("input")) return;
    pressed = { id, x: e.clientX, y: e.clientY };
    window.addEventListener("pointermove", rowMove);
    window.addEventListener("pointerup", rowUp, { once: true });
    window.addEventListener("pointercancel", rowUp, { once: true });
  }

  function rowMove(e: PointerEvent) {
    if (!pressed) return;
    if (!dragged) {
      if (Math.hypot(e.clientX - pressed.x, e.clientY - pressed.y) < 5) return;
      dragged = pressed.id;
      document.getSelection()?.removeAllRanges();
    }
    e.preventDefault();
    dropAt = rowUnder(e);
    // Near the top or bottom of the panel: scroll, faster the nearer the edge.
    cancelAnimationFrame(scrolling);
    const panel = listPanel?.getBoundingClientRect();
    if (!panel || !listPanel) return;
    const speed = e.clientY < panel.top + 40 ? -(panel.top + 40 - e.clientY) / 3 : e.clientY > panel.bottom - 40 ? (e.clientY - (panel.bottom - 40)) / 3 : 0;
    if (!speed) return;
    const [x, y] = [e.clientX, e.clientY];
    const scroll = () => {
      if (!dragged || !listPanel) return;
      listPanel.scrollTop += Math.max(-24, Math.min(24, speed));
      dropAt = rowUnder({ clientX: x, clientY: y } as PointerEvent);
      scrolling = requestAnimationFrame(scroll);
    };
    scrolling = requestAnimationFrame(scroll);
  }

  function rowUp() {
    window.removeEventListener("pointermove", rowMove);
    window.removeEventListener("pointerup", rowUp);
    window.removeEventListener("pointercancel", rowUp);
    cancelAnimationFrame(scrolling);
    pressed = null;
    if (dragged) {
      // The click that ends a drag is not a click on the row.
      suppressClick = true;
      setTimeout(() => (suppressClick = false), 0);
      dropLayer();
    }
  }
  let suppressClick = false;

  function rowUnder(e: PointerEvent): { id: string; where: "above" | "below" | "inside" } | null {
    const row = (document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null)?.closest<HTMLElement>("li[data-layer]");
    if (!row || row.dataset.layer === dragged) return null;
    const r = row.getBoundingClientRect();
    const lower = e.clientY > r.top + r.height / 2;
    // The lower half of a group's row means "into the group".
    return { id: row.dataset.layer!, where: lower ? (row.dataset.kind === "group" ? "inside" : "below") : "above" };
  }

  /** Put the dragged layer above or below another in that one's group, or into a group at its top. */
  function dropLayer() {
    const [from, target] = [dragged && find(dragged), dropAt && find(dropAt.id)];
    const [id, where] = [dragged, dropAt?.where];
    dragged = null;
    dropAt = null;
    if (!from || !target || !id || !where) return;
    // A group cannot go inside itself.
    if (from.layer.kind === "group" && (target.layer.id === id || find(target.layer.id, from.layer as Layer & { kind: "group" }))) {
      status = "A group cannot be moved into itself.";
      return;
    }
    let parent = target.parent;
    let index: number;
    if (where === "inside" && target.layer.kind === "group") {
      parent = target.layer.id;
      index = target.layer.children.length - (from.parent === parent ? 1 : 0);
    } else {
      // The list shows the top layer first, so "above" in the list is a higher place in the group.
      // Places are counted after the layer has left its old one.
      const shift = from.parent === parent && from.index < target.index ? 1 : 0;
      index = target.index - shift + (where === "above" ? 1 : 0);
    }
    run("Reorder", () => core!.execute({ type: "moveLayer", id, parent, index }, `Reorder ${from.layer.name}`));
    select(id);
  }

  /** Move the selected layer one place up or down among the layers of its group. */
  function step(by: 1 | -1) {
    const found = active ? find(active) : null;
    if (!found) return;
    const siblings = found.parent === doc!.root.id ? doc!.root.children.length : ((find(found.parent)?.layer as Layer & { kind: "group" }).children.length ?? 0);
    const index = found.index + by;
    if (index < 0 || index >= siblings) {
      status = by > 0 ? "It is already the top layer of its group." : "It is already the bottom layer of its group.";
      return;
    }
    run("Reorder", () => core!.execute({ type: "moveLayer", id: found.layer.id, parent: found.parent, index }, `Move ${found.layer.name} ${by > 0 ? "up" : "down"}`));
  }

  /** Switch one of the selected layer's effects on or off, or change one of its numbers. */
  function setEffect(index: number, change: Record<string, unknown>) {
    if (!selected?.effects) return;
    const effects = selected.effects.map((effect, i) => (i === index ? { ...effect, ...change } : effect));
    setProp({ effects }, "Layer effect");
  }
  const EFFECT_NAMES: Record<string, string> = { colorOverlay: "Color overlay", gradientOverlay: "Gradient overlay", stroke: "Stroke", dropShadow: "Drop shadow", innerShadow: "Inner shadow", outerGlow: "Outer glow", bevel: "Bevel" };

  /**
   * Paint over the selected rectangle of the selected layer's picture, made up from what is
   * around it. The patch goes on a layer of its own just above, so the picture itself is untouched.
   */
  async function fillSelected() {
    const found = active ? find(active) : null;
    if (!core || !selection || !found) return;
    const layer = found.layer;
    if (layer.kind !== "pixel" && layer.kind !== "art" && layer.kind !== "fill") {
      status = "Select an image layer (a picture, the art or a fill) to fill part of.";
      return;
    }
    const [a, b, tx, c, d, ty] = layer.transform;
    if (!layer.asset || b !== 0 || c !== 0 || Math.abs(a - d) > 1e-9 || a <= 0) {
      status = "A turned or stretched layer cannot be filled this way yet.";
      return;
    }
    // The rectangle in the picture's own pixels.
    const hole: [number, number, number, number] = [(selection.x - tx) / a, (selection.y - ty) / a, (selection.x + selection.width - tx) / a, (selection.y + selection.height - ty) / a];
    busy = true;
    status = "Filling the selection…";
    try {
      const made = await native.fillSelection(core.assetBytes(layer.asset), hole, fillTier, fillSeed++);
      const id = newId("fill");
      core.addAsset(id, await native.fillImage(made.ticket), false);
      const s = a * made.scale;
      const fill = { ...base(id, "Fill"), kind: "fill", source: made.provider, params: { seed: fillSeed - 1, selection: hole }, asset: id, transform: [s, 0, tx + made.x * a, 0, s, ty + made.y * a] };
      const edits: Edit[] = [{ type: "setAsset", id, asset: { kind: "image", path: `assets/${id}.png`, width: made.width, height: made.height } }, { type: "addLayer", parent: found.parent, layer: fill, index: found.index + 1 }];
      core.execute({ type: "batch", edits }, "Fill the selection");
      blit(core.dirty());
      refresh();
      status = `Filled the selection with ${made.provider} in ${(made.ms / 1000).toFixed(1)} s` + (made.notices.length && fillTier !== "auto" ? ` (${made.notices.join("; ")})` : "");
    } catch (err) {
      status = `The selection could not be filled: ${err instanceof Error ? err.message : err}`;
    } finally {
      busy = false;
    }
  }

  /** The selection as a closed path in document pixels: the lasso's outline, or the rectangle. */
  function selectionPath(): string | null {
    if (!selection) return null;
    if (outline && outline.length > 2) return "M" + outline.map((p) => `${p.x} ${p.y}`).join("L") + "Z";
    const { x, y, width, height } = selection;
    return `M${x} ${y}L${x + width} ${y}L${x + width} ${y + height}L${x} ${y + height}Z`;
  }

  /** Erase the selected layer inside the selection, or everything of it outside: how a figure is cut out. */
  function eraseSelection(outside: boolean) {
    const path = selectionPath();
    if (!path || !active) {
      status = "Select a layer and mark out an area first.";
      return;
    }
    run(outside ? "Erase outside" : "Erase inside", () => core!.eraseArea(active!, path, outside));
  }

  /** Take the brush color from what is on screen under the pointer. */
  function pickColor(e: PointerEvent) {
    if (!core || !canvas) return;
    const [x, y] = [Math.floor(e.offsetX * dpr()), Math.floor(e.offsetY * dpr())];
    if (x < 0 || y < 0 || x >= canvas.width || y >= canvas.height) return;
    const pixel = canvas.getContext("2d")!.getImageData(x, y, 1, 1).data;
    if (pixel[3] === 0) return;
    color = "#" + [pixel[0], pixel[1], pixel[2]].map((v) => v!.toString(16).padStart(2, "0")).join("");
    status = `Brush color ${color}`;
  }

  function duplicateSelected() {
    if (!active) return;
    run("Duplicate", () => select(core!.duplicate(active!)));
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
    if (tool === "pick") {
      pickColor(e);
      return;
    }
    if (tool === "select") {
      selecting = toDoc(e);
      selection = null;
      outline = lasso ? [selecting] : null;
      return;
    }
    if (tool === "move") {
      if (typing) commitText(true);
      // Drag moves the selected layer wherever the pointer is; with none selected, or with Ctrl, what is under it.
      const p = toDoc(e);
      const under = e.ctrlKey || !active ? core.hit(p.x, p.y) : null;
      if (under) select(under);
      if (active) beginDrag(e, active);
      return;
    }
    if (tool === "magic") {
      if (!active) {
        status = "Select the layer to cut from first.";
        return;
      }
      const p = toDoc(e);
      run("Magic eraser", () => {
        const gone = core!.magicErase(active!, p.x, p.y, tolerance, contiguous);
        if (!gone) throw new Error("nothing was that color there");
      });
      return;
    }
    if (!active) {
      status = "Select a layer first: the eraser works on any layer, the brush on a paint layer.";
      return;
    }
    // Color goes on a paint layer. With any other layer selected, the brush gets a new one of its own on top.
    if (!erase && !active.startsWith("paint-")) {
      try {
        active = core.addPaintLayer("Paint");
        refresh();
      } catch (err) {
        status = err instanceof Error ? err.message : String(err);
        return;
      }
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
      viewChanged();
    } else if (selecting && outline) {
      // A lasso: the pointer's path, a point every few screen pixels.
      const p = toDoc(e);
      const last = outline[outline.length - 1]!;
      if (Math.hypot(p.x - last.x, p.y - last.y) * view.scale > 3 * dpr()) outline = [...outline, { x: Math.round(p.x), y: Math.round(p.y) }];
      const xs = outline.map((q) => q.x);
      const ys = outline.map((q) => q.y);
      const [x, y] = [Math.max(0, Math.min(...xs)), Math.max(0, Math.min(...ys))];
      const [x1, y1] = [Math.min(doc!.width, Math.max(...xs)), Math.min(doc!.height, Math.max(...ys))];
      selection = outline.length > 2 && x1 - x > 1 && y1 - y > 1 ? { x, y, width: x1 - x, height: y1 - y } : null;
    } else if (selecting) {
      const p = toDoc(e);
      const [x, y] = [Math.max(0, Math.min(selecting.x, p.x)), Math.max(0, Math.min(selecting.y, p.y))];
      const [x1, y1] = [Math.min(doc!.width, Math.max(selecting.x, p.x)), Math.min(doc!.height, Math.max(selecting.y, p.y))];
      selection = x1 - x > 1 && y1 - y > 1 ? { x: Math.round(x), y: Math.round(y), width: Math.round(x1 - x), height: Math.round(y1 - y) } : null;
    } else if (drag) {
      // Only the latest position matters; it is acted on once a frame.
      dragPoint = e;
      frame ||= requestAnimationFrame(onFrame);
    } else if (painting) {
      // Browsers batch fast pointer movement; take every point so strokes stay smooth.
      const events = e.getCoalescedEvents?.() ?? [];
      strokePoints(events.length ? events : [e]);
    }
  }

  function pointerUp(e?: PointerEvent) {
    flush();
    panning = null;
    selecting = null;
    endDrag(e);
    if (!painting || !core) return;
    painting = false;
    core.strokeEnd(erase ? (restore ? "Restore" : "Eraser") : "Brush");
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
    viewChanged();
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
    } else if (mod && (e.key === "]" || e.key === "[")) {
      e.preventDefault();
      step(e.key === "]" ? 1 : -1);
    } else if (mod && e.key.toLowerCase() === "j") {
      e.preventDefault();
      duplicateSelected();
    } else if (e.key === "Escape" && drag) {
      core!.dragEnd(false);
      drag = null;
      refresh();
      refine();
    } else if ((e.key === "Delete" || e.key === "Backspace") && active && tool === "select" && selection) {
      e.preventDefault();
      eraseSelection(false);
    } else if ((e.key === "Delete" || e.key === "Backspace") && active && tool === "move") {
      e.preventDefault();
      removeSelected();
    } else if (e.key.startsWith("Arrow") && active && tool === "move") {
      e.preventDefault();
      const step = e.shiftKey ? 10 : 1;
      nudge(e.key === "ArrowLeft" ? -step : e.key === "ArrowRight" ? step : 0, e.key === "ArrowUp" ? -step : e.key === "ArrowDown" ? step : 0);
    } else if (e.key.toLowerCase() === "v") tool = "move";
    else if (e.key.toLowerCase() === "i") tool = "pick";
    else if (e.key.toLowerCase() === "m") tool = "select";
    else if (e.key.toLowerCase() === "w") tool = "magic";
    else if (e.key === "Escape" && selection) (selection = null), (outline = null);
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
    {#if tool === "paint" && erase}
      <label title="Bring back what was erased, instead of erasing. Works on any layer that is not a paint layer: those are erased through a mask, so nothing is lost."><input type="checkbox" bind:checked={restore} /> Restore</label>
    {/if}
    <button class:active={tool === "magic"} onclick={() => (tool = "magic")} title="Magic eraser (W): click a color on the selected layer to erase that color where it touches the click. For cutting a character out of its background.">Magic eraser</button>
    {#if tool === "magic"}
      <label title="How alike a color must be to be erased: low takes only the color clicked, high takes more">Tolerance <input type="range" min="0" max="128" bind:value={tolerance} /> <output>{tolerance}</output></label>
      <label title="Off: erase that color everywhere on the layer, not only where it touches the click"><input type="checkbox" bind:checked={contiguous} /> Touching only</label>
    {/if}
    <button class:active={tool === "pick"} onclick={() => (tool = "pick")} title="Take the brush color from the picture (I)">Eyedropper</button>
    <button class:active={tool === "select"} onclick={() => (tool = "select")} title="Mark out a rectangle (M)">Select</button>
    {#if tool === "select"}
      <label title="Draw the selection freehand instead of as a rectangle"><input type="checkbox" bind:checked={lasso} /> Lasso</label>
    {/if}
    {#if selection}
      <button onclick={() => eraseSelection(false)} disabled={busy || !active} title="Erase the selected layer inside the selection (Delete). Nothing is lost: the eraser's Restore brings it back.">Erase inside</button>
      <button onclick={() => eraseSelection(true)} disabled={busy || !active} title="Erase everything of the selected layer outside the selection: draw around a figure, then this cuts it out.">Erase outside</button>
    {/if}
    {#if hasNative && selection}
      <button onclick={fillSelected} disabled={busy || !active} title="Paint over the selection on the selected image layer, made up from what is around it. The result is its own layer.">Fill selection</button>
      <select bind:value={fillTier} title="How to make the fill">
        <option value="auto">Automatic</option>
        <option value="sd">Stable Diffusion</option>
        <option value="lama">LaMa model</option>
        <option value="patchmatch">PatchMatch</option>
      </select>
    {/if}
    <label title="Brush size in document pixels ([ and ])">Size <input type="range" min="1" max="800" bind:value={size} /> <output>{size}</output></label>
    <label>Hardness <input type="range" min="0" max="1" step="0.05" bind:value={hardness} /> <output>{Math.round(hardness * 100)}%</output></label>
    <label>Opacity <input type="range" min="0.05" max="1" step="0.05" bind:value={opacity} /> <output>{Math.round(opacity * 100)}%</output></label>
    <input type="color" bind:value={color} title="Brush color" disabled={erase} />
    <span class="spacer"></span>
    {#if hasNative}<button onclick={exportPng} disabled={busy || !doc} title="Save the card as a PNG picture at its full size">Export PNG…</button>{/if}
    <button onclick={fit} title="Fit to window (0)">Fit</button>
    <output class="zoom">{Math.round(view.scale * 100)}%</output>
    <button disabled={!history.canUndo} onclick={() => run("Undo", () => core!.undo())}>Undo</button>
    <button disabled={!history.canRedo} onclick={() => run("Redo", () => core!.redo())}>Redo</button>
  </header>

  {#if hasNative && core}
    <CardPanel bind:this={panel} {core} bind:busy bind:focusField say={(text) => (status = text)} changed={cardChanged} opened={cardOpened} demoFill={async ([x, y, width, height]) => {
      select(layers.find((l) => l.layer.role === "card.art")?.layer.id);
      selection = { x, y, width, height };
      fillTier = "patchmatch";
      await fillSelected();
    }} demoReorder={async () => {
      // Drag the fourth row up onto the top half of the first, then the first row down below the third.
      const rows = () => [...document.querySelectorAll<HTMLElement>("aside ul li[data-layer]")];
      const names = () => rows().slice(0, 5).map((r) => r.querySelector(".name")!.textContent!.trim().split(/\s+/)[0]).join(", ");
      const dragRow = async (from: number, to: number, fraction: number) => {
        const [a, b] = [rows()[from]!.getBoundingClientRect(), rows()[to]!.getBoundingClientRect()];
        const at = (x: number, y: number) => ({ clientX: x, clientY: y, button: 0, bubbles: true, pointerId: 1 });
        rows()[from]!.dispatchEvent(new PointerEvent("pointerdown", at(a.left + 60, a.top + a.height / 2)));
        for (let i = 1; i <= 5; i++) window.dispatchEvent(new PointerEvent("pointermove", at(a.left + 60, a.top + ((b.top + b.height * fraction - a.top) * i) / 5)));
        window.dispatchEvent(new PointerEvent("pointerup", at(b.left + 60, b.top + b.height * fraction)));
        await new Promise((r) => setTimeout(r, 50));
      };
      const before = names();
      await dragRow(3, 0, 0.25);
      const up = names();
      await dragRow(0, 2, 0.75);
      return `rows: ${before} | 4th dragged up to the top: ${up} | then the top one dragged down: ${names()}`;
    }} />
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
      class:picking={tool === "pick"}
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
            onpointermove={(e) => drag && ((dragPoint = stagePoint(e)), (frame ||= requestAnimationFrame(onFrame)))}
            onpointerup={() => (flush(), endDrag())}
          ></div>
        {/each}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="handle turn"
          title="Drag to turn (Shift: steps of 15°)"
          onpointerdown={(e) => {
            e.stopPropagation();
            (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
            beginDrag(stagePoint(e), active!, undefined, true);
          }}
          onpointermove={(e) => drag && ((dragPoint = stagePoint(e)), (frame ||= requestAnimationFrame(onFrame)))}
          onpointerup={() => (flush(), endDrag())}
        ></div>
      </div>
    {/if}
    {#if selection && outline && outline.length > 2}
      <svg class="lasso"><polygon points={outline.map((p) => `${((p.x - view.x) * view.scale) / dpr()},${((p.y - view.y) * view.scale) / dpr()}`).join(" ")} /></svg>
    {:else if selection}
      {@const at = onScreen(selection)}
      <div class="marquee" style:left="{at.left}px" style:top="{at.top}px" style:width="{at.width}px" style:height="{at.height}px"></div>
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
    {#if cursor && tool === "paint"}
      <div class="brush" style:left="{cursor.x}px" style:top="{cursor.y}px" style:width="{cursorSize}px" style:height="{cursorSize}px"></div>
    {/if}
  </section>

  <aside bind:this={listPanel} class:reordering={dragged !== null}>
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
              {#each fonts as font}<option value={font}>{font}</option>{/each}
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
        <label>Name <input class="text" value={selected.name} onchange={(e) => setProp({ name: e.currentTarget.value || selected!.name }, "Rename")} /></label>
        <div class="checks">
          <label title="Show this layer only where the layer beneath it has ink"><input type="checkbox" checked={selected.clip} onchange={(e) => setProp({ clip: e.currentTarget.checked }, "Clip to the layer below")} /> Clip</label>
          <label title="Keep it from being moved"><input type="checkbox" checked={!!selected.locked?.position} onchange={(e) => setProp({ locked: { ...selected!.locked, position: e.currentTarget.checked } }, "Lock position")} /> Lock</label>
          {#if selected.mask}
            <label title="The layer's pixel mask"><input type="checkbox" checked={selected.mask.enabled !== false} onchange={(e) => setProp({ mask: { ...selected!.mask, enabled: e.currentTarget.checked } }, "Mask")} /> Mask</label>
          {/if}
          {#if selected.vectorMask}
            <label title="The layer's vector mask"><input type="checkbox" checked={selected.vectorMask.enabled !== false} onchange={(e) => setProp({ vectorMask: { ...selected!.vectorMask, enabled: e.currentTarget.checked } }, "Vector mask")} /> Vector mask</label>
          {/if}
        </div>
        {#if selected.effects?.length}
          <label class="fx-head"><input type="checkbox" checked={selected.effectsEnabled !== false} onchange={(e) => setProp({ effectsEnabled: e.currentTarget.checked }, "Layer effects")} /> Effects</label>
          {#each selected.effects as effect, i}
            <div class="fx">
              <label><input type="checkbox" checked={effect.enabled} onchange={(e) => setEffect(i, { enabled: e.currentTarget.checked })} /> {EFFECT_NAMES[effect.type] ?? effect.type}</label>
              {#if "opacity" in effect}
                <label title="Opacity">%<input class="number" type="number" min="0" max="100" value={Math.round(effect.opacity * 100)} onchange={(e) => setEffect(i, { opacity: Math.min(100, Math.max(0, Number(e.currentTarget.value))) / 100 })} /></label>
              {/if}
              {#if "distance" in effect}
                <label title="Distance in pixels">↘<input class="number" type="number" min="0" value={effect.distance} onchange={(e) => setEffect(i, { distance: Math.max(0, Number(e.currentTarget.value)) })} /></label>
              {/if}
              {#if "size" in effect}
                <label title="Size in pixels">⌀<input class="number" type="number" min="0" value={effect.size} onchange={(e) => setEffect(i, { size: Math.max(0, Number(e.currentTarget.value)) })} /></label>
              {/if}
              {#if "color" in effect}
                <input type="color" value={hex(effect.color)} onchange={(e) => setEffect(i, { color: rgb(e.currentTarget.value, effect.color[3]) })} />
              {/if}
            </div>
          {/each}
        {/if}
        <div class="new">
          <button class="small" onclick={() => step(1)} title="One place up in its group (Ctrl+])">Up</button>
          <button class="small" onclick={() => step(-1)} title="One place down in its group (Ctrl+[)">Down</button>
          <button class="small" onclick={duplicateSelected} title="Make a copy of this layer just above it (Ctrl+J)">Duplicate</button>
          <button class="small" onclick={removeSelected} title="Delete this layer (Del)">Delete</button>
        </div>
      </div>
    {/if}
    <ul>
      {#each layers as { layer, depth } (layer.id)}
        <li
          style:padding-left="{depth * 14}px"
          class:hidden={!layer.visible}
          class:selected={layer.id === active}
          data-layer={layer.id}
          data-kind={layer.kind}
          class:lifted={dragged === layer.id}
          class:drop-above={dropAt?.id === layer.id && dropAt.where === "above"}
          class:drop-below={dropAt?.id === layer.id && dropAt.where === "below"}
          class:drop-inside={dropAt?.id === layer.id && dropAt.where === "inside"}
          onpointerdown={(e) => rowDown(e, layer.id)}
        >
          <span class="grip" title="Drag a row to another place in the list: onto the top half of a row to go above it, the bottom half to go below (or into a group)">⠿</span>
          <input type="checkbox" checked={layer.visible} onchange={() => toggle(layer)} title="Visible" />
          <button class="name" onclick={() => !suppressClick && select(layer.id)} ondblclick={() => editText(layer.id)} title={layer.kind === "text" ? "Double-click to edit the text" : "Select this layer"}>
            {layer.name}
            <small>{layer.kind}{layer.clip ? " · clipped" : ""}</small>
          </button>
        </li>
      {/each}
    </ul>
    {#if history.labels.length || history.redoLabels.length}
      <h2>History</h2>
      <ol class="history">
        {#each history.labels as label, i}
          <li class:now={i === history.labels.length - 1}><button class="name" onclick={() => travel(i + 1 - history.labels.length)} title="Go back to just after this step">{label}</button></li>
        {/each}
        {#each history.redoLabels as label, i}
          <li class="undone"><button class="name" onclick={() => travel(i + 1)} title="Do this step again">{label}</button></li>
        {/each}
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
  .lasso { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; }
  .lasso polygon { fill: rgba(91, 157, 255, 0.12); stroke: #fff; stroke-width: 1; stroke-dasharray: 4 3; paint-order: stroke; filter: drop-shadow(0 0 1px #000); }
  .marquee { position: absolute; border: 1px dashed #fff; outline: 1px dashed #000; pointer-events: none; box-sizing: border-box; }
  header select { background: #3a3d43; color: inherit; border: 1px solid #50545c; border-radius: 4px; padding: 3px 6px; font: inherit; }
  .handle.turn { left: 50%; top: -22px; border-radius: 50%; cursor: grab; }
  .typing { position: absolute; background: rgba(255, 255, 255, 0.92); color: #111; border: 1px solid #5b9dff; padding: 2px 4px; resize: both; font-family: inherit; z-index: 2; }
  .grip { cursor: grab; color: #6d7078; padding: 0 2px; user-select: none; }
  aside ul li { touch-action: none; user-select: none; -webkit-user-select: none; }
  aside.reordering, aside.reordering * { cursor: grabbing !important; }
  li.lifted { opacity: 0.45; }
  li.drop-above { box-shadow: 0 -2px 0 #5b9dff; }
  li.drop-below { box-shadow: 0 2px 0 #5b9dff; }
  li.drop-inside { outline: 2px solid #5b9dff; outline-offset: -2px; }
  .checks { display: flex; flex-wrap: wrap; gap: 2px 10px; }
  .fx { display: flex; flex-wrap: wrap; align-items: center; gap: 2px 6px; padding-left: 8px; }
  .fx .number { width: 3.4em; }
  .props .text { background: #2b2d31; color: inherit; border: 1px solid #50545c; border-radius: 4px; padding: 2px 4px; font: inherit; min-width: 0; flex: 1; }
  .history { max-height: 220px; overflow: auto; }
  .history li.now { background: #3d4270; }
  .history li.undone .name { color: #666; font-style: italic; }
  ol.history li .name { color: #c8c9cd; }
  canvas.picking { cursor: crosshair; }
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
