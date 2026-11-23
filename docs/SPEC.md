# Cardsmith — Specification and Phased Plan

## 1. Goals

1. Produce card renders at Proxyshop quality on Linux, with no Photoshop.
2. Be a real editor. Everything the card engine produces is an ordinary layered document that the user can keep editing: paint, mask, add layers, move things, re-fill.
3. Be easier to use than Proxyshop. That means a visual set-symbol picker, card search with a printing picker, a rich rules-text editor with inline symbols and a live preview, and a batch queue the user can correct before rendering.
4. Run on modest hardware: old GPUs with 8 GB of VRAM, or no GPU at all.

### Non-goals (v1)

- General-purpose photo editing beyond what card work needs.
- Exporting PSD files.
- Shipping templates, fonts or card art.
- Windows/macOS packaging. Tauri makes it possible later, but it isn't a v1 target.

## 2. Core data model (`packages/doc`)

```ts
type Doc = {
  version: 1;
  width: number; height: number;        // pixels; templates define print size
  dpi: number;
  root: GroupLayer;
  assets: Record<AssetId, AssetRef>;    // images, SVGs, fonts referenced by id
  card?: CardData;                      // present when produced by the card engine
  templateId?: string;
};

type LayerBase = {
  id: LayerId; name: string;
  visible: boolean; opacity: number;    // 0..1
  blend: BlendMode;                     // full Photoshop set we encounter in the survey
  clip: boolean;                        // clipped to the layer below
  mask?: { asset: AssetId; offset: Vec2; inverted?: boolean; density?: number; feather?: number };
  vectorMask?: Path;
  effects?: Effect[];                   // only types found by the survey (Phase 0)
  locked?: { pixels?: boolean; position?: boolean; all?: boolean };
  role?: string;                        // semantic tag used by the card engine, e.g. "pinlines.W"
};

type Layer =
  | GroupLayer      // { kind:"group", children: Layer[], passThrough: boolean }
  | PixelLayer      // { kind:"pixel", asset: AssetId, transform: Mat3 }
  | TextLayer       // { kind:"text", box: Rect, content: RichText, style: TextStyle, fit: FitMode }
  | ShapeLayer      // { kind:"shape", path: Path, fill: Fill, stroke?: Stroke }
  | ArtFrameLayer   // { kind:"art", frameRef: Rect, asset?: AssetId, transform: Mat3, fit: "cover"|"contain"|"manual" }
  | FillLayer       // { kind:"fill", source: FillProviderId, params, asset: AssetId } -- generated content
  | AdjustmentLayer;// only types found by the survey
```

- Commands follow the `{ do(doc), undo(doc), label }` pattern. Everything is serializable to JSON.
- Save files are `.cardsmith`: a zip containing `doc.json` and an `assets/` folder.

## 3. Template package (output of the importer)

```
<template>/
  manifest.json       // id, name, author, source psd hash, supported layouts, print size, bindings
  doc.json            // converted layer tree (Doc without card)
  assets/             // per-layer PNGs, masks, SVGs
  preview.png
```

**Bindings** connect what the card engine needs to the template's layers, so the engine never searches by layer name at runtime:

```jsonc
{
  "layouts": ["normal"],
  "artRefs": { "normal": "<layerId>", "fullArt": "<layerId>" },
  "text": { "name": "<id>", "mana": "<id>", "type": "<id>", "rules": "<id>", "pt": "<id>", ... },
  "textRefs": { "rules": "<textbox reference layerId>", "rulesLand": "<id>" },
  "toggles": {
    "pinlines":   { "W": "<id>", "U": "<id>", "WU": "<id>", "Land": "<id>", ... },
    "twins":      { ... }, "background": { ... }, "crown": { ... }, "pt": { ... }
  },
  "symbols": { "expansion": "<id>", "watermark": "<id>" },
  "flags": { "isFullArt": true, "fillOutsideArt": "whenNotFullArtRef" }
}
```

The importer generates bindings automatically from Proxyshop's layer-name conventions (`src/enums/layers.py`). A small UI lets the user fix any binding that couldn't be resolved.

## 4. Card data (`packages/cards`)

- `CardData` is Scryfall-shaped JSON. A single schema covers fetched cards, custom cards and edited cards.
- The Scryfall client caches every response on disk, sends the required headers, throttles requests, and offers optional bulk data download for offline search and autocomplete.
- Set data comes from Scryfall `/sets` (name, code, icon SVG, release date). Keyrune is the fallback. Don't depend on `api.hexproof.io` (Proxyshop's own service), because it may disappear.
- `frameLogic(card) -> { background, pinlines, twins, identity, isColorless, isHybrid }` is ported from `frame_logic.py`.
- Layout classes are ported from `layouts.py`, starting with Normal.

## 5. Phases

Each phase lists its tasks and the acceptance criteria that must pass before the next phase starts.

### Phase 0: Scaffold and de-risk (spikes)

**Tasks**
1. Set up a pnpm monorepo, the Tauri 2 app shell, Vite and Vitest. Pick the UI framework and log the decision.
2. **CanvasKit spike, headless.** In Node, render a paragraph that has inline placeholders (stand-ins for mana symbols), mixed regular and italic runs, and a fixed width. Measure its height, then shrink the font until it fits a box. This is the riskiest assumption in the project, so prove it first.
3. **PSD feature survey** (`pnpm survey <dir>`): parse every PSD with ag-psd and output a report (JSON plus Markdown) with:
   - every layer kind,
   - every blend mode, effect type and adjustment type in use, with counts,
   - smart objects and whether they're embedded or linked,
   - text features in use (fonts, tracking, leading, paragraph styles, warp),
   - clipping and mask usage,
   - the layer names matched against `src/enums/layers.py`.
4. Write `docs/SURVEY.md` from the report. **That list, and nothing beyond it, is the feature set the renderer must support.**
5. Collect references for the golden tests. For each template in the first set (Normal, Extended, a borderless/full-art template), get reference renders of the `template_renders.toml` cards. Possible sources:
   - Proxyshop itself, on a Windows/Photoshop machine if one is available.
   - Photopea exports (free, in a browser).
   - At minimum, the PSD's embedded composite image, which shows the default visibility state.

   Store references in `fixtures/local/golden/`, which is gitignored.

**Acceptance criteria**
- `pnpm dev` opens a window.
- The spike test produces a PNG of fitted rules text with inline placeholder glyphs.
- `docs/SURVEY.md` exists.

### Phase 1: Document model and renderer

**Tasks**
1. Implement the types and command system from §2, including save/load of `.cardsmith` files.
2. In `packages/render`, implement compositing of groups (including pass-through), pixel layers, opacity, blend modes, clipping, raster and vector masks, and shape layers.
3. Implement every effect and adjustment listed in `SURVEY.md`. Each effect gets its own unit test with a tiny hand-built document.
4. Add a `pnpm render` CLI.

**Acceptance criteria**
- Hand-written test documents render correctly in both Node and the app.
- Rendering the same input in Node and in the app gives a pixel diff of at most 1/255 per channel.

### Phase 1R: Rust core (added 2026-11-23)

The owner expects heavy brush and eraser work, so edit-to-screen latency is now a first-class requirement. The core moves to Rust. See the Decisions log for the measurements behind this.

**Architecture**
- `crates/core` (`cardsmith-core`): document model, commands and history, serialization, compositor, effects, renderer, and later the brush engine and text. No platform dependencies, so it builds both natively and for `wasm32-unknown-unknown`.
- **In the app, the core runs as WebAssembly inside the webview.** It owns the pixel buffers and writes the visible region straight to the canvas. Pixels never cross Tauri's IPC.
- **Natively, the same crate** backs the headless CLI (`cardsmith render`, batch export) and the test suite, with threads.
- The Tauri (Rust) side keeps file IO, model downloads, fills and other heavy or privileged work. It exchanges files and commands with the webview, not frames.
- The web UI (Svelte) keeps panels, pickers and editors. It holds no document state of its own: it sends commands to the core and renders what the core reports.

**Tasks**
1. Port `packages/doc` to Rust with the same JSON format, so existing documents and samples load unchanged. Port its tests.
2. Port `raster`, `field`, `effects` and `render`, replacing CanvasKit with pure-Rust path rasterization and image decoding. Port their tests. The sample renders must match the TypeScript renderer's within 1/255, apart from path anti-aliasing at shape edges.
3. `cardsmith render` and `cardsmith samples` CLI, replacing `pnpm render`.
4. WebAssembly bindings and app integration: the app renders the samples through the WASM core.
5. **Incremental rendering:** tiles or dirty rectangles, cached composites below and above the active layer, and a viewport-resolution preview path.
6. **Brush spike:** a paintable pixel layer, brush and eraser as undoable commands, and an on-screen frame-time readout.
7. Remove the TypeScript renderer and document commands once the Rust ones pass. Keep TypeScript only for UI and the PSD importer (an offline tool that reads PSDs with ag-psd and writes our JSON).

**Acceptance criteria**
- All ported tests pass under `cargo test`.
- Native and WASM builds render the samples to identical bytes.
- On the owner's machine, brush strokes on a 3264×4440 document with a realistic layer stack stay under 16 ms per frame for brushes up to 256 px, with the readout to prove it. If this fails, revisit the display path before Phase 2.

### Phase 2: PSD importer and golden tests

**Tasks**
1. Write `pnpm import <psd>`. It converts the PSD into a template package (§3) and extracts each layer's pixels, masks and vector data.
2. For text layers, convert the PSD's text data into our TextLayer (box, font, size, leading, tracking, alignment, color).
3. Smart objects: rasterize their embedded content into a pixel layer. Keep a reference to the original data, in case later phases need it.
4. Auto-generate the bindings, then build a binding-review screen (a simple dev UI is fine at this stage).
5. Build the golden runner. It renders the imported template in its default state, compares it to the reference, writes a diff heatmap, and applies per-template thresholds (PSNR plus a maximum-error-region check).

**Acceptance criteria**
- The first three templates import without warnings.
- Their default-state renders match the reference within the threshold.
- Anything that doesn't match is listed in `docs/KNOWN_DIFFS.md`.

### Phase 3: Text engine (`packages/text`)

**Tasks**
1. Write a rules-text parser that produces RichText runs: plain text, `{X}` symbols (the full mana set, including hybrid, Phyrexian, snow, tap/untap, energy and others), italic reminder text, the flavor text separator, bold/ability words, and line/paragraph breaks.
2. Port `locate_symbols`, `locate_italics`, `generate_italics` and `strip_reminder_text` from `cards.py`, with tests built from real card text.
3. Render symbols as inline placeholders drawn from the Mana font, using the color treatment from Proxyshop's symbol maps (`src/enums/mtg.py`, `src/schema/colors.py`).
4. Write the fitters, porting their behavior from `text_layers.py` and `helpers/text.py`:
   - shrink-to-fit-height for rules text, keeping leading proportional,
   - shrink-to-width for the name and type line, while avoiding overlap with the mana cost and set symbol,
   - vertical centering inside the textbox reference,
   - the flavor divider, which shows only when flavor text is present.

**Acceptance criteria**
- A text test suite of about 40 real cards (long text, many symbols, planeswalker-style lines, flavor text, reminder text) matches reference crops within threshold.

### Phase 4: Card data and frame logic

**Tasks**
1. Write the Scryfall client and cache, plus bulk data download.
2. Port `frame_logic.py` test-first, converting `frame_data.toml` into Vitest cases.
3. Port `NormalLayout` from `layouts.py`: which fields come from Scryfall, and how.
4. Parse the art filename tag format (`Name (Artist) [SET] {num} $creator`) for compatibility.

**Acceptance criteria**
- 100% of the `frame_data.toml` cases pass.
- Fetching a card by name, or by set and number, returns `CardData` from the cache when offline.

### Phase 5: Card engine (Normal layout)

**Tasks**
1. `buildCard(template, card, art?, options) -> Doc`, which:
   - applies the toggles from frame logic,
   - fills the text layers through the text engine,
   - places the set symbol and rarity,
   - places the art (see Phase 6),
   - sets `role` tags so the UI can offer card-aware editing later.
2. The output is a normal Doc. The engine never touches rendering.
3. Run golden tests over the `template_renders.toml` cards for the first three templates.

**Acceptance criteria**
- Every golden test card for the first three templates is within threshold.

### Phase 6: Art framing and fill

**Port from Proxyshop**

1. **Framing** (`frame_layer`): scale = `max(refW/artW, refH/artH)` (cover), then center on the reference box.
2. **Reference choice** (`art_reference`): use the full-art reference when the art is vertical (`height > 1.1 × width`) or when the vertical-full-art option is on and the template has one. Otherwise use the normal art frame.
3. **Fill decision** (`is_content_aware_enabled`): fill when the template is full-art but the chosen reference isn't a full/borderless frame. Extended templates always fill.
4. **Mask** (`content_aware_fill_edges`): the art's alpha, contracted by 10px and smoothed by 5px, then inverted. With the "feathered" option, contract by 22px and feather by 8px instead. Scale these pixel values relative to Proxyshop's template resolution.

**FillProvider interface (`packages/fill`)**

```ts
interface FillProvider {
  id: "patchmatch" | "lama" | "migan" | "sdcpp" | "comfyui";
  available(): Promise<{ ok: boolean; reason?: string; vramMB?: number }>;
  fill(req: { image: ImageData; mask: ImageData; prompt?: string; seed?: number;
              variations?: number; signal: AbortSignal; onProgress?(p: number): void })
    : Promise<ImageData[]>;
}
```

**Tiers**

| Tier | Backend | Runs on | Notes |
|---|---|---|---|
| 1 | PatchMatch (Rust) | CPU | Always available. The direct equivalent of Content-Aware Fill. Seeded, so it can produce variations |
| 2 | LaMa or MI-GAN (ONNX via `ort`) | GPU or CPU | Tiny model, downloaded on demand. Default when any GPU is present |
| 3 | SD 1.5 / SDXL inpaint via stable-diffusion.cpp sidecar (Vulkan) | GPU, 8 GB OK | Opt-in. The Generative Fill equivalent. Optional prompt. **Phase 6 spike: confirm sd.cpp mask/inpaint mode** |
| opt | ComfyUI HTTP endpoint | User's server | For power users: Flux Fill, LoRAs, any workflow JSON |

**Pipeline rules**
- Work at reduced resolution. Crop to the mask's bounding box plus some context, downscale to the tier's working size (Tier 1: whatever is fast; Tier 2: about 1024px; Tier 3: 512 for SD 1.5, 1024 for SDXL), fill, upscale, then composite only the masked region with a feathered edge.
- The result is a new `FillLayer` beneath the art, clipped as the template requires, carrying its own mask. The original art is never modified.
- On error or out-of-memory, fall back one tier and show a notice (Proxyshop falls back from Generative Fill to Content-Aware Fill the same way).
- The variation picker shows N thumbnails, and clicking one commits it. "Re-fill with…" on any FillLayer re-runs it with a different tier, seed or prompt.
- On first launch, probe the hardware and choose the default tier (no usable GPU → Tier 1; otherwise Tier 2). Models download only when their tier is selected, and the download size is shown first.

**Acceptance criteria**
- A horizontal image on a full-art template produces a filled document with each of Tier 1 and Tier 2.
- Tier 3 works on an 8 GB card, or is logged as blocked with the reason.
- All tiers produce the same layer structure.

### Phase 7: Editor UI

**Tasks**
1. **Canvas:** zoom/pan, selection, move/scale/rotate transforms, snapping to reference boxes.
2. **Layers panel:** visibility, opacity, blend, clip, masks, groups, drag to reorder, layer effects editor (the surveyed types only).
3. **Tools:** brush and eraser (including on masks), rectangle and lasso selections, fill selection with any fill tier, eyedropper.
4. **Card panel:** a form generated from the layout definition. Normal cards get name, cost, type, rules, flavor, P/T, artist, set, rarity. Planeswalkers get ability lines, sagas get chapters, and so on in later phases. Edits re-run only the affected part of the card engine, as commands.
5. **Rules text editor:**
   - Typing `{` opens a searchable symbol palette.
   - Symbols appear as inline icon chips.
   - Reminder text is italicized automatically, with a manual override.
   - The flavor section is its own field.
   - The canvas preview updates as you type (debounced).
6. **Card search:** Scryfall autocomplete by name, then a printing picker with thumbnails (set, number, art). Selecting a printing fills the card data.
7. **Set symbol picker:** searchable by set name or code, shows the icon and year, has a rarity toggle with live preview, and accepts a custom SVG upload.
8. **Template picker:** grouped by layout, with preview thumbnails and per-template options (port the concept of Proxyshop's `src/data/config/*.toml`).
9. **Undo/redo** for everything, plus a history panel.

**Acceptance criteria**
- A new user can make a custom card with a set symbol, mana symbols in the rules text, and a filled full-art background, without reading docs or typing a set code.

### Phase 8: Batch and export

**Tasks**
1. **Batch queue:** drop art files or paste a card list. Each row shows the detected card, printing, template and art match, all editable before rendering. Filename tags are honored but optional.
2. Run the queue headless through Tauri/Node workers, with progress, cancel and per-row errors.
3. **Export:** PNG and JPG at the template's print resolution, with optional bleed and crop marks, and optional print-sheet PDF.

**Acceptance criteria**
- 50 cards render unattended, and any failure is reported per row without stopping the batch.

### Phase 9: Layout expansion

Port the layouts in this order, each with golden tests:
1. Transform/MDFC
2. Saga
3. Planeswalker
4. Class
5. Adventure
6. Battle
7. Split
8. Leveler
9. Token
10. Planar
11. Mutate/Prototype

## 6. Open questions (record answers in the Decisions log)

1. Project name and license. Ported files stay MPL-2.0. MPL for the whole project is the simplest choice.
2. UI framework: Svelte or React.
3. Which three templates come first?
4. Golden reference source: is a Windows machine with Proxyshop available?
5. Minimum supported GPU or driver for Tier 2/3 (Vulkan version).
6. Font handling: detect system fonts by name, or have the user point to a fonts folder? Show a clear warning listing any font a template needs that's missing.

## 7. Decisions log

_(append: date — decision — reason)_

- 2026-11-22 — UI framework is **Svelte 5** (open question 2). — The UI is mostly panels bound to one document store. Svelte's compiled reactivity fits that with less runtime and boilerplate than React, and the render core is framework-free CanvasKit either way, so the choice is cheap to revisit.
- 2026-11-22 — First three templates are Proxyshop's **`normal.psd`, `normal-extended.psd` and `borderless-vector.psd`** (open question 3). — They are the Normal, Extended and borderless/full-art templates the spec asks for, and `normal.psd` also carries the Fullart treatment.
- 2026-11-22 — Whole project is **MPL-2.0**, confirmed by the owner (open question 1). `LICENSE` holds the standard text. — Simplest option the spec names; ported files must be MPL-2.0 anyway. The name stays "Cardsmith" as a working title.
- 2026-11-22 — Golden references start as the **PSDs' embedded composites**, extracted with `pnpm composite` (open question 4 stays open). — No Windows/Photoshop machine is available to this session. Composites only show the default visibility state, so per-card references from Proxyshop or Photopea are still needed before Phase 5's acceptance criteria can be judged.
- 2026-11-22 — Test fonts come from the **`dejavu-fonts-ttf` dev dependency**, not from committed files or system fonts. — Keeps rule 7 (no committed fonts) while making text tests identical on every machine.
- 2026-11-22 — The survey reads Proxyshop's layer names by **parsing `reference/Proxyshop/src/enums/layers.py` at run time** instead of porting the list. — Nothing to keep in sync and nothing to license for a dev tool. The importer's binding generator (Phase 2) will need a real port.
- 2026-11-22 — `pnpm test:golden`, `pnpm render` and `pnpm import` do not exist yet. — They belong to Phases 1 and 2. CLAUDE.md lists only commands that work.
- 2026-11-22 — Layer-style "master switch" is modelled separately from each effect's own enabled flag. — The survey found drop shadows and color overlays defined on layers whose whole style is off; Proxyshop enables them at render time.
- 2026-11-22 — No Windows machine is available (open question 4 answered). Per-card golden references will be **Photopea exports** made on the owner's Linux machine, from a checklist written at the start of Phase 2. — Photopea approximates Photoshop, so thresholds for those references are set looser than for embedded composites.
- 2026-11-23 — **Compositing runs in our own typed-array code, not in CanvasKit.** CanvasKit rasterizes vector paths to coverage, decodes images and (from Phase 3) lays out text; blending, masks, clipping and effects are loops over `Uint8ClampedArray` in `packages/render/src/raster.ts`. — Measured on a 3264×4440 canvas, CanvasKit's CPU backend blends at about 13 megapixels per second (1.2 s to draw one full-size layer), which is minutes per card. The same blit in plain JavaScript takes about 0.1 s. Path coverage through CanvasKit into an alpha-only buffer takes 4 ms, so that part stays. This also makes Node and the app identical by construction, and leaves a small, pure module to port to Rust/WASM if more speed is needed.
- 2026-11-23 — **The app renders on the CPU with the same code as Node; no WebGL surface.** — Follows from the entry above. `pnpm test:parity` shows 0/255 difference in headless Chromium and in the real Tauri window. GPU compositing can come later as an optional preview path, but it must not become the reference.
- 2026-11-23 — Buffers are **straight (non-premultiplied) 8-bit RGBA**. — It is what Photoshop stores, it keeps color exact at soft edges, and masks and opacity only touch the alpha byte.
- 2026-11-23 — `Mat3` is six numbers `[a, c, tx, b, d, ty]` (the first two rows of Skia's 3×3). `Path` is an SVG path string with an optional fill rule. `Color` is straight RGBA in 0..1. A group's `children[0]` is the **bottom** layer. — The spec left these open. All are plain JSON, and the child order matches ag-psd, so the importer does not have to reverse anything.
- 2026-11-23 — Additions to SPEC §2's `LayerBase`, all driven by the survey: `fillOpacity`, `effectsEnabled` (the layer-style master switch), `mask.outside` ("hide"/"reveal", Photoshop's default mask color), `mask.enabled`, and `vectorMask` as `{ path, enabled, inverted }` instead of a bare path. — Proxyshop switches masks and layer styles on and off at render time, so "disabled" has to be representable.
- 2026-11-23 — There is no `AdjustmentLayer` type and `Fill` is solid color only. — The survey found no adjustment layers and no gradient or pattern shape fills. Add them when a surveyed template needs them.
- 2026-11-23 — Effects are computed from an exact **signed distance field** of the layer's alpha (`field.ts`), not with image-filter morphology. — Photoshop's strokes and spreads are round (distance based); box-shaped dilation gives square corners. The same field drives stroke, spread, choke and the chisel-hard bevel.
- 2026-11-23 — Effects follow Photoshop's non-isolated model: drop shadow and outer glow blend with the backdrop under the layer; gradient overlay, color overlay, inner shadow, bevel and stroke blend over it, in that order; fill opacity fades only the layer's pixels; opacity fades everything. A clipped layer is rendered on its own first and then blended into its base without changing the base's alpha. A group with effects is treated as isolated.
- 2026-11-23 — **Uncalibrated constants, to be settled by Phase 2's golden tests:** blur sigma = size ÷ 2 for shadows, glows and bevel soften; the bevel's lighting normalization; the Gaussian contour as a smoothstep; radial gradient radius = half the bounds' projection; mask feather sigma = feather ÷ 2. — These reproduce Photoshop's behaviour in kind. The exact numbers need reference images, which Phase 1 does not have.
- 2026-11-23 — Text layers are part of the document model but are **not drawn** until Phase 3; the renderer reports a warning for each. Pixel layers with a scale or rotation use plain bilinear sampling. — Text is Phase 3's job. Bilinear is correct for upscaling and mild downscaling; heavy downscaling of high-resolution art needs area filtering, which belongs with art framing in Phase 6.
- 2026-11-23 — Render speed at card size (3264×4440, one CPU core, Node): about 0.1–0.25 s per full-canvas layer, 0.5 s for an overlay effect, 1.5–3 s for a stroke, shadow or bevel on a full-canvas shape. — Good enough to proceed. The obvious next steps, when needed: cache each layer's rasterized content between renders, render previews at reduced scale, limit the distance field to the band an effect needs, and move the hot loops to Rust/WASM.
- 2026-11-23 — **The core moves to Rust; the UI stays web (Tauri + Svelte).** Chosen by the owner. — Heavy brush and eraser work makes edit latency a hard requirement, and one Rust core is easier to audit than logic split across TypeScript and Rust. Phases 0 and 1 are the cheapest point to do this: the TypeScript core is about 1,500 lines with 134 tests that port directly.
- 2026-11-23 — **Pixels never cross Tauri's IPC. In the app the Rust core runs as WebAssembly inside the webview.** Required by the owner and confirmed by measurement in the real Tauri window (WebKitGTK, release build): raw binary IPC and a custom URI scheme both move about 100–125 MB/s, so a 1920×1080 frame costs 66 ms and a full 3264×4440 frame 700 ms. A view over WASM memory passed to `putImageData` costs 1.5 ms and 12 ms for the same sizes. An empty IPC round trip is about 1 ms, which is fine for commands and files.
- 2026-11-23 — Rust by itself is not the speed-up; structure is. Measured for one full-canvas layer (3264×4440): the Phase 1 JavaScript loop about 100 ms, the same math in scalar Rust 106 ms native and 163–191 ms as WASM, an integer fast path for normal blending over opaque pixels 50 ms native and 57 ms as WASM. WebKitGTK runs WASM as fast as Node does. — So the plan relies on dirty rectangles, cached stacks and viewport-resolution previews first, branch-free SIMD kernels second, and native threads for export and batch.
- 2026-11-23 — Rule 2 ("the render core must run headless") now means **headless as a native Rust binary**, not in Node. CanvasKit leaves the renderer. Text layout (Phase 3) needs a pure-Rust engine that also builds for WASM; the Phase 0 CanvasKit spike proved the approach (inline placeholders, measure, shrink to fit), not the library. — A dependency that cannot build for WASM would force pixels back over IPC.
- 2026-11-23 — The PSD importer stays in TypeScript with ag-psd. — It is an offline, one-time conversion that writes our JSON and PNGs, it is not on any hot path, and Rust has no PSD library that reads layer effects and text data as completely.
- 2026-11-23 — The WASM module exposes a **plain C ABI with no binding generator** (`crates/wasm`): JSON and file bytes go in through `(pointer, length)` pairs, and the frame is read as a view over the module's memory. — The build is one `cargo build` with no extra tools to install or keep in step, and the whole boundary is about 250 lines that can be read in one sitting.
- 2026-11-23 — Pure-Rust replacements for CanvasKit in the core: **tiny-skia** for anti-aliased path coverage, **kurbo** for SVG path parsing, the **image** crate for PNG/JPEG. — All build for `wasm32-unknown-unknown`. tiny-skia's anti-aliasing differs slightly from Skia's on curved edges, which is the only difference between the Rust and TypeScript sample renders.
- 2026-11-23 — Edits are **data** (`Edit`, JSON-serializable), and applying one returns its inverse. — The UI can send edits as JSON without Rust-side glue per command, and undo cannot drift out of step with redo because both run the same `apply`.
- 2026-11-23 — Parity is checked by **frame hash** across three builds: native CLI, WASM in Node, WASM in the frontend (`pnpm test:parity`, `--app` for the real Tauri window). All three agree on both samples.
- 2026-11-23 — **Everything expensive is computed in fixed 256 px tiles on the document grid and cached**: path coverage, and each layer effect's coverage (keyed by a fingerprint of the layer's content and effects). — A tile is computed the same way whatever region asked for it, so a partial redraw is bit-identical to the same part of a full render; tests compare the two pixel for pixel. Uniform tiles cost no memory. Redrawing after a change is then mostly compositing.
- 2026-11-23 — An open document is a **`Session`** in the core: structural edits and brush strokes share one undo stack, each change redraws only the area it can affect (the layer's bounds plus the reach of effects on it and on the groups around it), and a viewport-sized preview is kept in step. The app only ever copies the dirty rectangle of that preview to the canvas.
- 2026-11-23 — While painting, the **flattened layers beneath the painted layer are cached per tile**, when the paint order can be cut at that layer (not clipped, and every group around it a plain pass-through group). Otherwise the stack is redrawn for the dirty area. — Both paths are tested against a fresh full render.
- 2026-11-23 — Brush model: round brush and eraser, hardness, **opacity per stroke** (re-crossing a spot within one stroke does not build up, as in Photoshop), pressure scales size. Strokes store undo data per touched 256 px block. Painting is refused on scaled or rotated layers and on images shared by several layers, with a message, until those cases are handled properly.
- 2026-11-23 — **Measured in the real Tauri window (WebKitGTK, WASM, software display under Xvfb), card-sized sample of 3264×4440 with 45 layers.** Per pointer move, core time: 32 px brush median 1 ms; 128 px 1 ms; 256 px 3 ms; 600 px 12–15 ms. Copying the dirty rectangle to the canvas: under 1 ms. The first time a stroke enters a tile it costs 10–30 ms to build that tile's cache; going over the same area again stays at 3–8 ms worst case. Starting a stroke costs about 11–16 ms. — This meets the 16 ms target for brushes up to 256 px except for the first-touch spikes. The owner's machine is the real test.
- 2026-11-23 — Known costs still to address, in likely order of value: opening the card-sized sample takes about 4 s natively and 5 s in WASM; an edit to a large layer redraws its whole area (0.6–1.8 s for most of the card); undoing a long stroke takes 0.3–0.7 s; a stroke start copies the whole layer image. Options: build first-touch tiles ahead of the pointer, render previews at screen resolution, integer and SIMD fast paths in the compositor, native threads for export.
- 2026-11-23 — Gradient overlays are laid out across the layer's **geometric bounds** (path bounds, or the image rectangle), not the bounds of its visible pixels. — It needs no rendering, so it is the same for a partial and a full draw. Photoshop uses pixel bounds; for imported template layers the two agree, because PSD layer images are already cropped to their pixels.
- 2026-11-24 — The owner ran the brush spike on their machine (Linux Mint, with the GPU fully loaded by another job) and reported it holds up. **Phase 1R's acceptance gate is passed.** — The editor is CPU-only, so GPU load does not compete with it; heavy CPU load remains untested.
- 2026-11-24 — Phase 1R task 7 done: the TypeScript renderer, document commands and the Phase 0 CanvasKit spike are removed. `packages/doc` keeps only the type definitions. CanvasKit is no longer a dependency. — The spike's findings are recorded in docs/SURVEY.md; the text engine will be built in Rust in Phase 3.
