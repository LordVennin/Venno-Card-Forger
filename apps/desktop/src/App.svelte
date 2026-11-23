<script lang="ts">
  import type { Doc, Layer } from "@cardsmith/doc";
  import type { Brush, Core, CoreState, Rect } from "./lib/core";
  import { getCore } from "./lib/renderer";

  // Phase 1R brush spike. The Rust core (WebAssembly) owns the document, its
  // history and the pixels. This component sends input and edits, and copies
  // the part of the view the core says has changed onto the canvas.

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

  /** Run something that changes the document, then show the result. */
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

  function pointerDown(e: PointerEvent) {
    if (!core || busy) return;
    canvas!.setPointerCapture(e.pointerId);
    if (e.button === 1 || e.button === 2 || e.altKey) {
      panning = { x: e.clientX, y: e.clientY };
      return;
    }
    if (e.button !== 0) return;
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
    } else if (painting) {
      // Browsers batch fast pointer movement; take every point so strokes stay smooth.
      const events = e.getCoalescedEvents?.() ?? [];
      strokePoints(events.length ? events : [e]);
    }
  }

  function pointerUp() {
    panning = null;
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
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === "z") {
      e.preventDefault();
      if (e.shiftKey) run("Redo", () => core!.redo());
      else run("Undo", () => core!.undo());
    } else if (mod && e.key.toLowerCase() === "y") {
      e.preventDefault();
      run("Redo", () => core!.redo());
    } else if (e.key === "[") size = Math.max(1, Math.round(size / 1.25));
    else if (e.key === "]") size = Math.min(1000, Math.round(size * 1.25) + 1);
    else if (e.key.toLowerCase() === "e") erase = true;
    else if (e.key.toLowerCase() === "b") erase = false;
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

<main>
  <header>
    <strong>Cardsmith</strong>
    {#each samples as sample, i}
      <button class:active={i === current} disabled={busy} onclick={() => open(i)}>{sample}</button>
    {/each}
    <label class="file" title="Load font files (TTF, OTF) so text layers can be drawn. Fonts are never stored in documents.">
      Fonts… <input type="file" accept=".ttf,.otf,.ttc" multiple onchange={loadFonts} />
    </label>
    <span class="divider"></span>
    <button class:active={!erase} onclick={() => (erase = false)} title="Brush (B)">Brush</button>
    <button class:active={erase} onclick={() => (erase = true)} title="Eraser (E)">Eraser</button>
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

  <section class="stage" bind:this={stage}>
    <canvas
      bind:this={canvas}
      onpointerdown={pointerDown}
      onpointermove={pointerMove}
      onpointerup={pointerUp}
      onpointercancel={pointerUp}
      onpointerleave={() => (cursor = null)}
      onwheel={wheel}
      oncontextmenu={(e) => e.preventDefault()}
    ></canvas>
    {#if cursor && active}
      <div class="brush" style:left="{cursor.x}px" style:top="{cursor.y}px" style:width="{cursorSize}px" style:height="{cursorSize}px"></div>
    {/if}
  </section>

  <aside>
    <h2>Layers <button class="small" onclick={newPaintLayer} disabled={busy}>+ Paint layer</button></h2>
    <ul>
      {#each layers as { layer, depth } (layer.id)}
        <li style:padding-left="{depth * 14}px" class:hidden={!layer.visible} class:selected={layer.id === active}>
          <input type="checkbox" checked={layer.visible} onchange={() => toggle(layer)} title="Visible" />
          <button class="name" onclick={() => (active = layer.id)} title={layer.kind === "pixel" ? "Paint on this layer" : "Only pixel layers can be painted on"}>
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
    <span class="hint">Wheel: zoom · Alt-drag or middle-drag: pan · [ ]: brush size · B/E: brush/eraser · Ctrl+Z: undo</span>
  </footer>
</main>

<style>
  main {
    display: grid;
    grid-template: "header header" auto "stage aside" 1fr "footer footer" auto / 1fr 270px;
    height: 100vh;
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
