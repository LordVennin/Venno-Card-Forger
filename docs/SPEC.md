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
