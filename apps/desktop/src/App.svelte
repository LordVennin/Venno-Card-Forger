<script lang="ts">
  import type { Doc, Layer } from "@cardsmith/doc";
  import type { Core, CoreState } from "./lib/core";
  import { getCore } from "./lib/renderer";

  // Phase 1R shell. The Rust core (WebAssembly) owns the document, its history
  // and the pixels. This component sends edits and shows what the core reports.
  let canvas = $state<HTMLCanvasElement>();
  let core = $state.raw<Core>();
  let samples = $state<string[]>([]);
  let current = $state(0);
  let doc = $state.raw<Doc>();
  let history = $state.raw<CoreState>({ canUndo: false, canRedo: false, labels: [], warnings: [] });
  let status = $state("Loading core…");

  getCore()
    .then((loaded) => {
      core = loaded;
      samples = loaded.sampleNames();
      open(0);
    })
    .catch((err) => (status = `The core failed to load: ${err.message ?? err}`));

  /** Pull the core's state and redraw. Called after anything that changes the document. */
  function refresh() {
    if (!core || !canvas) return;
    doc = core.doc();
    const started = performance.now();
    const frame = core.render();
    const rendered = performance.now() - started;
    canvas.width = frame.width;
    canvas.height = frame.height;
    // The frame is a view over the core's memory: it goes to the canvas in one copy.
    canvas.getContext("2d")!.putImageData(new ImageData(frame.pixels, frame.width, frame.height), 0, 0);
    history = core.state();
    status = `${frame.width}×${frame.height} · render ${rendered.toFixed(1)} ms · to screen ${(performance.now() - started - rendered).toFixed(1)} ms`;
  }

  function open(index: number) {
    current = index;
    core?.openSample(index);
    refresh();
  }

  function run(action: () => void) {
    try {
      action();
    } catch (err) {
      status = `Edit refused: ${err instanceof Error ? err.message : err}`;
      return;
    }
    refresh();
  }

  const toggle = (layer: Layer) =>
    run(() => core?.execute({ type: "setLayerProps", id: layer.id, props: { visible: !layer.visible } }, `${layer.visible ? "Hide" : "Show"} ${layer.name}`));

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

  // The canvas mounts after the core may already be ready.
  $effect(() => {
    if (canvas && core && !doc) refresh();
  });
</script>

<main>
  <header>
    <strong>Cardsmith</strong>
    {#each samples as sample, i}
      <button class:active={i === current} onclick={() => open(i)}>{sample}</button>
    {/each}
    <span class="spacer"></span>
    <button disabled={!history.canUndo} onclick={() => run(() => core?.undo())}>Undo</button>
    <button disabled={!history.canRedo} onclick={() => run(() => core?.redo())}>Redo</button>
  </header>

  <section class="stage">
    <canvas bind:this={canvas}></canvas>
  </section>

  <aside>
    <h2>Layers</h2>
    <ul>
      {#each layers as { layer, depth } (layer.id)}
        <li style:padding-left="{depth * 14}px" class:hidden={!layer.visible}>
          <label>
            <input type="checkbox" checked={layer.visible} onchange={() => toggle(layer)} />
            {layer.name}
            <small>{layer.kind}{layer.clip ? " · clipped" : ""}</small>
          </label>
        </li>
      {/each}
    </ul>
    {#if history.labels.length}
      <h2>History</h2>
      <ol>
        {#each history.labels as label}<li>{label}</li>{/each}
      </ol>
    {/if}
  </aside>

  <footer>
    {status}
    {#each history.warnings as warning}<span class="warning">{warning}</span>{/each}
  </footer>
</main>

<style>
  main {
    display: grid;
    grid-template: "header header" auto "stage aside" 1fr "footer footer" auto / 1fr 260px;
    height: 100vh;
  }
  header {
    grid-area: header;
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 8px 12px;
    background: #2b2d31;
    border-bottom: 1px solid #111;
  }
  .spacer { flex: 1; }
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
    display: grid;
    place-items: center;
    overflow: auto;
    /* Checkerboard, so transparency is visible. */
    background: repeating-conic-gradient(#3a3a3a 0% 25%, #2e2e2e 0% 50%) 0 0 / 20px 20px;
  }
  canvas { box-shadow: 0 2px 16px #000a; max-width: 100%; max-height: 100%; }
  aside {
    grid-area: aside;
    overflow: auto;
    background: #26282c;
    border-left: 1px solid #111;
    padding: 8px 10px;
  }
  h2 { font-size: 12px; text-transform: uppercase; letter-spacing: 0.08em; color: #9a9ca3; margin: 4px 0 8px; }
  ul, ol { list-style: none; margin: 0 0 12px; padding: 0; font-size: 13px; }
  li { padding-block: 2px; }
  li.hidden, ol li { color: #777; }
  small { color: #777; margin-left: 4px; }
  footer {
    grid-area: footer;
    padding: 4px 12px;
    font-size: 12px;
    color: #9a9ca3;
    background: #2b2d31;
    border-top: 1px solid #111;
  }
  .warning { color: #f0b232; margin-left: 12px; }
</style>
