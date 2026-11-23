# CLAUDE.md

Working title: **Cardsmith**. This is a Linux-first desktop editor for Magic: The Gathering proxy cards. Think of it as Proxyshop rebuilt as a focused, layer-based image editor, with no Photoshop dependency.

The full plan lives in `docs/SPEC.md`. Read it before starting any phase. Work one phase at a time, in order, and don't start a phase until the previous phase's acceptance criteria pass.

## What we are building

- A layer-based editor (pixel, text, shape, group and art-frame layers, plus masks, clipping, blend modes and layer effects) specialized for card frames.
- A **card engine**. It takes card data (from Scryfall, or a custom card), picks a template, toggles the template's layers, sets the text, frames the art, fills empty art space, and returns a normal, fully editable document.
- A **one-time importer**. It converts Proxyshop-style PSD templates into our own document format. The PSD is never read at render time.

## Stack

| Concern | Choice |
|---|---|
| Core | **Rust** (`crates/core`): document model, commands, compositor, effects, renderer, brush engine, text. Builds natively and for `wasm32-unknown-unknown` |
| App shell | Tauri 2. The core runs as **WebAssembly inside the webview** and draws to the canvas itself |
| Frontend | TypeScript, Vite, Svelte 5. UI only: panels, pickers, editors |
| Headless | The same core as a native binary: `cardsmith render`, batch export, tests |
| Text | Pure-Rust layout engine, chosen in Phase 3. Mana symbols are inline placeholders |
| PSD parsing | `ag-psd` in TypeScript, used only in the offline importer |
| Heavy compute (Tauri side) | PatchMatch fill, ONNX Runtime (`ort` crate) for LaMa/MI-GAN, stable-diffusion.cpp as a sidecar binary, model downloads, file IO |
| Card data | Scryfall API and Scryfall bulk data |
| Symbols | Mana font (open license) for mana symbols. Scryfall `/sets` SVGs and Keyrune for set symbols |

**Migration in progress (SPEC Phase 1R):** `packages/doc` and `packages/render` are the TypeScript originals being ported to `crates/core`. Until the port lands they remain the working implementation and the behavioural reference; their tests define what the Rust code must do.

## Repository layout (target)

```
crates/core/           Rust core: doc model, commands, compositor, effects, renderer, brush, text
crates/cli/            Native binary: render, samples, batch
crates/wasm/           WebAssembly bindings used by the app
apps/desktop/          Tauri app (src-tauri/ = Rust shell, src/ = Svelte UI)
packages/importer/     PSD -> template package converter + feature survey (TypeScript, offline)
packages/cards/        Card model, Scryfall client, frame logic, layouts
packages/engine/       Card engine: template manifest + card -> document
packages/fill/         FillProvider interface + glue to Rust/HTTP backends
tests/golden/          Reference images and the golden-image runner
reference/Proxyshop/   Clone of Proxyshop for reference only (gitignored)
```

Whether `cards`, `engine` and `fill` end up in Rust or TypeScript is decided when their phase starts. Default to Rust for anything that touches the document.

## Non-negotiable rules

1. **The document model is the source of truth.** The renderer is a pure function: `(document, assets) -> pixels`. Don't add UI-only rendering paths.
2. **The render core must run headless.** `crates/core` has to build and run as a plain native binary with no window, no webview and no GPU. Batch export and every golden test depend on this. It must also build for WebAssembly, so it takes no dependency that cannot.
3. **Every edit is a command.** All mutations go through the command system in the core, so undo/redo works for everything, including brush strokes, card-engine output and fills.
3a. **Pixels never cross Tauri's IPC.** Frames go from the WASM core straight to the canvas. IPC carries commands and files only. Measured: IPC moves about 100 MB/s, which is 66 ms for one 1080p frame.
4. **Fills and generated content go on their own layers.** Never modify the user's art pixels in place.
5. **Golden tests gate visual work.** Any change to compositing, effects, text or templates must keep `pnpm test:golden` passing. If you change a threshold, write the reason in the commit message.
6. **Port logic test-first.** When porting Proxyshop logic, port its test data first (e.g. `src/data/tests/frame_data.toml`), confirm the tests fail, then port the code.
7. **Never commit third-party assets.** No fonts other than the openly licensed ones, no templates, no card art, no Wizards of the Coast assets. Users provide fonts and templates, which the app imports locally. Test fixtures that need them load from a gitignored `fixtures/local/` folder, and those tests skip cleanly when it's missing.
8. **Ported code keeps MPL-2.0.** Proxyshop is MPL-2.0. A file substantially ported from it keeps an MPL-2.0 header and a `Ported from Proxyshop <path>` line.
9. **Follow Scryfall's rules.** Send a descriptive `User-Agent` and `Accept` header. Stay under 10 requests/second. Cache responses. Prefer bulk data for search and offline use.
10. **Keep VRAM use low.** The target user may have an old 8 GB GPU or no GPU at all. Every AI feature needs a CPU fallback and must degrade to the next fill tier without crashing.

## Proxyshop reference map

**Session start:** if `reference/Proxyshop` doesn't exist, clone it before doing anything else. It's gitignored, so every fresh session (including cloud sessions) needs this:
`git clone --depth 1 https://github.com/Investigamer/Proxyshop reference/Proxyshop`

| Our concern | Proxyshop source |
|---|---|
| Frame color logic (pinlines, twins, background, identity, hybrid) | `src/frame_logic.py`, tests in `src/data/tests/frame_data.toml` |
| Card layouts (normal, transform, MDFC, saga, class, PW, battle, split, adventure, leveler, token, ...) | `src/layouts.py` |
| Template behavior, art loading, fill decision | `src/templates/_core.py` (`load_artwork`, `art_reference`, `is_content_aware_enabled`), `src/templates/normal.py` |
| Art framing (cover-scale + center) | `src/helpers/position.py` (`frame_layer`) |
| Fill selection math (contract/smooth/feather/invert) | `src/helpers/design.py` (`content_aware_fill_edges`, `generative_fill_edges`) |
| Rules text: symbols, italics, reminder text | `src/cards.py` (`locate_symbols`, `locate_italics`, `generate_italics`, `strip_reminder_text`) |
| Text fitting and formatting | `src/text_layers.py` (`FormattedTextArea`, `ScaledTextField`, ...), `src/helpers/text.py` |
| Symbol maps and regexes | `src/enums/mtg.py`, `src/schema/colors.py` |
| Art filename tags `Name (Artist) [SET] {num} $creator` | `src/cards.py` (`parse_card_info`), `src/enums/mtg.py` (`CardTextPatterns`) |
| Layer name constants | `src/enums/layers.py` |
| Per-template test cards | `src/data/tests/template_renders.toml` |

Proxyshop drives Photoshop over COM. Port the **decisions** it makes, not the Photoshop calls.

## Workflow

- Read the current phase in `docs/SPEC.md`. Break it into tasks, do them in order, and run the tests after each one.
- Commit small. Every commit leaves `pnpm test` green.
- When you hit an open question from the spec, don't guess silently. Write the decision you made under "Decisions log" at the bottom of `docs/SPEC.md`.
- Visual debugging: `pnpm render <doc.json> -o out.png` renders headless. `pnpm test:golden --update <name>` updates one reference, and only after a human has confirmed the new output is correct.

## Commands

```
pnpm install
pnpm dev              # run the Tauri app
cargo test --workspace   # Rust core tests (also: pnpm test:rust)
cargo run --release -p cardsmith-cli -- render <doc.json | file.cardsmith> -o <png>   # headless render (Rust)
cargo run --release -p cardsmith-cli -- samples <dir>                                # sample documents and renders
pnpm test             # TypeScript unit tests (Vitest)
pnpm typecheck        # tsc -b for packages, svelte-check for the app
pnpm render <doc.json | file.cardsmith> -o <png>   # headless render
pnpm render --samples <dir>                        # write the built-in sample documents and their renders
pnpm test:parity            # Node vs frontend pixel diff, in headless Chromium (skips if Playwright is absent)
pnpm test:parity --app      # the same, inside the real Tauri window (needs a display; xvfb-run -a works)
pnpm survey <dir-of-psds> [-o <out-dir>]    # importer feature survey -> survey.json + survey.md
pnpm composite <psd-or-dir> [-o <out-dir>]  # extract embedded composites as golden references
pnpm spike:text [-o <png>]                  # Phase 0 CanvasKit text-fit spike
```

Not built yet (Phase 2): `pnpm test:golden`, `pnpm import <psd> -o <template-dir>`.

`pnpm dev` needs the Tauri Linux libraries: `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev librsvg2-dev`.

Local fixtures (all gitignored, under `fixtures/local/`): `templates/` holds the PSDs, `survey/` the survey output, `golden/` the reference images.

## Where things are

- `packages/doc`: types (`types.ts`), commands and `History` (`commands.ts`), JSON and `.cardsmith` save/load (`serialize.ts`).
- `packages/render`: `render.ts` walks the layer tree; `raster.ts` is the compositor (no CanvasKit); `field.ts` is the distance field and blur; `effects.ts` turns effects into coverage stamps; `samples.ts` holds the sample documents used by tests, the app and the parity check.
- Do not composite through CanvasKit surfaces. Its CPU backend is about ten times slower than `raster.ts` (see the Decisions log).
