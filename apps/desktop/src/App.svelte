<script lang="ts">
  import { History, setLayerProps, type Layer } from "@cardsmith/doc";
  import { buildSamples, type Sample } from "@cardsmith/render";
  import { getRenderer } from "./lib/renderer";

  // Phase 1 shell: pick a sample document, toggle layers through commands,
  // undo and redo. The canvas only ever shows what the renderer returns.
  let canvas = $state<HTMLCanvasElement>();
  let samples = $state.raw<Sample[]>([]);
  let current = $state(0);
  let history = $state.raw<History>();
  let revision = $state(0);
  let status = $state("Loading renderer…");
  let warnings = $state<string[]>([]);

  getRenderer()
    .then(({ ck }) => {
      samples = buildSamples(ck);
      open(0);
    })
    .catch((err) => (status = `Renderer failed to load: ${err.message ?? err}`));

  function open(index: number) {
    current = index;
    history = new History(structuredClone(samples[index]!.doc));
    history.subscribe(() => revision++);
    revision++;
  }

  const layers = $derived.by(() => {
    void revision;
    const list: { layer: Layer; depth: number }[] = [];
    if (!history) return list;
    // Top layer first, like every layer panel, with children under their group.
    const add = (children: Layer[], depth: number) => {
      for (let i = children.length - 1; i >= 0; i--) {
        const layer = children[i]!;
        list.push({ layer, depth });
        if (layer.kind === "group") add(layer.children, depth + 1);
      }
    };
    add(history.doc.root.children, 0);
    return list;
  });

  $effect(() => {
    void revision;
    if (!canvas || !history) return;
    const doc = history.doc;
    const assets = samples[current]!.assets;
    getRenderer().then(({ renderer }) => {
      const started = performance.now();
      const result = renderer.render(doc, assets);
      canvas!.width = result.width;
      canvas!.height = result.height;
      const image = new ImageData(new Uint8ClampedArray(result.pixels), result.width, result.height);
      canvas!.getContext("2d")!.putImageData(image, 0, 0);
      warnings = result.warnings;
      status = `${result.width}×${result.height} rendered in ${Math.round(performance.now() - started)} ms`;
    });
  });

  const toggle = (layer: Layer) =>
    history?.execute(setLayerProps(layer.id, { visible: !layer.visible }, `${layer.visible ? "Hide" : "Show"} ${layer.name}`));
</script>

<main>
  <header>
    <strong>Cardsmith</strong>
    {#each samples as sample, i}
      <button class:active={i === current} onclick={() => open(i)}>{sample.name}</button>
    {/each}
    <span class="spacer"></span>
    <button disabled={!history?.canUndo || revision < 0} onclick={() => history?.undo()}>Undo</button>
    <button disabled={!history?.canRedo || revision < 0} onclick={() => history?.redo()}>Redo</button>
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
  </aside>

  <footer>
    {status}
    {#each warnings as warning}<span class="warning">{warning}</span>{/each}
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
  ul { list-style: none; margin: 0; padding: 0; font-size: 13px; }
  li { padding-block: 2px; }
  li.hidden { color: #777; }
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
