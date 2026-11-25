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
| Symbols | Proxyglyph (user-supplied font) for mana symbols. Set symbols and set data from hexproof.io, as Proxyshop uses |

`packages/doc` is only the TypeScript description of the document JSON, for the UI and the importer. The Rust core is the implementation and rejects anything that does not match `crates/core/src/doc.rs`, so change the two together.

## Repository layout (target)

```
crates/core/           Rust core: doc model, commands, compositor, effects, renderer, brush, text
crates/cards/          Card knowledge in Rust (pure, builds for WASM): rules text, symbols, frame logic, layouts, art file names
crates/scryfall/       Scryfall client (native only): polite requests, on-disk cache that works offline, bulk data
crates/cli/            Native binary: render, samples, batch
crates/wasm/           WebAssembly bindings used by the app
packages/doc/          TypeScript types for the document JSON (no logic)
apps/desktop/          Tauri app (src-tauri/ = Rust shell, src/ = Svelte UI)
packages/importer/     PSD -> template package converter + feature survey (TypeScript, offline)
crates/engine/         Card engine (pure, builds for WASM): template + card data -> one undoable batch edit
crates/fill/           Filling the space around art: the FillProvider trait, fallback between tiers, Tier 1 (PatchMatch, pure Rust), Tier 2 (LaMa through ONNX Runtime, `onnx` feature) and Tier 3 (stable-diffusion.cpp as a separate program, `sdcpp` feature)
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
- Visual debugging: `cargo run --release -p cardsmith-cli -- render <doc.json> -o out.png` renders headless. `pnpm test:golden --update <name>` updates one reference, and only after a human has confirmed the new output is correct.

## Commands

```
pnpm install
pnpm dev              # run the Tauri app
cargo test --workspace   # Rust core tests (also: pnpm test:rust)
cargo run --release -p cardsmith-cli -- render <doc.json | file.cardsmith> -o <png> [--fonts <dir>] [--text-bounds <json>] [--preview <1-3>]   # headless render (Rust); text needs the fonts
cargo run --release -p cardsmith-cli -- samples <dir>                                # sample documents and renders
cargo run --release -p cardsmith-cli -- text <suite.json> -o <dir> --fonts <dir>     # render a suite of rules text boxes
cargo run --release -p cardsmith-cli -- card "<name or art file name>" [--set <code>] [--number <n>] [--raw]   # a card from Scryfall (cached); --raw for Scryfall's own JSON
cargo run --release -p cardsmith-cli -- bulk <oracle | default>                      # download Scryfall bulk data for offline lookups
cargo run --release -p cardsmith-cli -- build <template-dir> "<name>" | <card.json> -o <png | .cardsmith> [--fonts <dir>] [--art <image>] [--no-symbol] [--fill auto|sd|lama|patchmatch|none] [--seed <n>] [--prompt <text>] [--sd-cli <path>]   # build a card from an imported template, filling around the art where the frame shows past it
cargo run --release -p cardsmith-cli -- symbols [--cache <dir>]                      # download set symbols and set data from hexproof.io for offline use
cargo run --release -p cardsmith-cli -- models [--cache <dir>]                       # download the LaMa fill model and ONNX Runtime (about 240 MB) for the model-based fill tier
cargo run --release -p cardsmith-cli -- models sd [--cache <dir>]                    # download the Stable Diffusion 1.5 inpainting model (4.3 GB) for --fill sd; see docs/TIER3.md
tools/install-tier3.sh [--backend vulkan|cuda|cpu] [--uninstall]                      # build sd-cli from source and download the model, into the cache where --fill sd looks
cargo run --release -p cardsmith-cli -- sheet -o <png> [--across N] [--shrink N] <images...>   # many images on one sheet
cargo run --release -p cardsmith-cli -- bench                                        # time opening, painting and undo on the card-sized sample
pnpm test             # TypeScript unit tests (Vitest): the importer and survey
pnpm typecheck        # tsc -b for packages, svelte-check for the app
pnpm build:wasm             # build crates/wasm and copy it into the app (pnpm dev does this for you)
pnpm test:parity            # native vs WASM-in-Node vs WASM-in-Chromium frame hashes, plus brush timings in the frontend (frontend part skips without Playwright)
pnpm test:parity --app      # the same, with the frontend running in the real Tauri window (needs a display; xvfb-run -a works)
pnpm survey <dir-of-psds> [-o <out-dir>]    # importer feature survey -> survey.json + survey.md
pnpm composite <psd-or-dir> [-o <out-dir>]  # extract embedded composites as golden references

pnpm import:psd <psd> -o <template-dir>     # convert a PSD into a template package (doc.json, assets/, manifest.json)
```

`pnpm test:golden [template...]` renders each imported template (default state, plus the states in `tests/golden/states.json`) and compares with the references in `fixtures/local/golden/`. It skips cleanly when there are none, and leaves text areas out when `fixtures/local/fonts` is empty. What does not match yet is listed in `docs/KNOWN_DIFFS.md`.

`pnpm test:text` renders the rules text of the cards in `tests/text/cards.json` (symbols, italics, bullets, flavor text, shrink to fit), checks each fits, and compares with `fixtures/local/text-golden/` where references exist. It skips cleanly without fonts. `--update <card>` or `--update all` accepts the current renders, only after a human has confirmed them.

`pnpm test:cards [template...]` builds the cards in `tests/cards/cards.json` from every imported template the engine has bindings for, fails on any engine warning, and compares with `fixtures/local/card-golden/<template>/` where references exist. Set symbols and set sizes come from `fixtures/local/cache` (fill it with `cardsmith symbols --cache fixtures/local/cache`; without it cards are built with no symbol). Fills are made with PatchMatch so the pixels are the same everywhere; `--fill lama` builds the suite with the model into `out/cards-lama/` without comparing. Sheets go to `out/cards/<template>/sheet.png`. `--update <template>/<card>` or `--update all` accepts renders, only after a human has confirmed them.

`pnpm dev` needs the Tauri Linux libraries (`libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev librsvg2-dev`) and the Rust WASM target (`rustup target add wasm32-unknown-unknown`).

Local fixtures (all gitignored, under `fixtures/local/`): `fonts/` holds the fonts text is drawn with, `templates/` holds the PSDs, `imported/` the template packages made from them, `survey/` the survey output, `golden/` the reference images, `text-golden/` and `card-golden/` the accepted text and card renders, `cache/` Scryfall and hexproof.io data for the card suite.

## Where things are

- `crates/core`: `doc.rs` (types), `commands.rs` (`Edit`, `apply`, `History`), `serialize.rs` (JSON and `.cardsmith`), `raster.rs` (compositor), `field.rs` (distance field and blur), `effects.rs` (effects as coverage stamps), `render.rs` (walks the layer tree; tile caches for path coverage and effects), `text.rs` (fonts registry and text layout: shaping by rustybuzz, our own line breaking and fitting), `paint.rs` (brush engine), `session.rs` (an open document: edits, strokes, undo, incremental redraw, the on-screen view, drawn from a smaller copy when zoomed out and only where visible when zoomed in; `settle` brings the full-size frame up to date), `proxy.rs` (the smaller copy of a document and its images), `arrange.rs` (the edit that moves or scales any kind of layer), `samples.rs` (sample documents, including the card-sized one). Tests are in `crates/core/tests` and beside the modules.
- **Two invariants the tests enforce, and that every renderer change must keep:** drawing a region gives exactly the pixels a full render has there, and a session's incrementally updated frame always equals a fresh full render once settled (`frame()` and `frame_hash()` settle it).
- `crates/cards`: `rules_text.rs` (ported from Proxyshop, MPL header): `italic_ranges`/`generate_italics`, `strip_reminder_text`, the symbol map and colors, and `format_rules_text`, which turns oracle and flavor text into the `RichText` of a rules box. Proxyshop's italics cases are in `tests/data/text_italic.toml`.
- `crates/cards` also has `frame_logic.rs` (background, pinlines and name boxes from a card's cost, types and text), `layout.rs` (`process_card_data` settles the layout, `card_data` reads every field into `CardData`) and `file_name.rs` (`Name (Artist) [SET] {num} $creator`), all ported with MPL headers. Proxyshop's cases are in `tests/data/`: `frame_data.toml` runs against `frame_cards.json`, which `tools/fetch-frame-cards.py` refreshes from Scryfall.
- `crates/engine`: `lib.rs` (`Template`, `Bindings`, `Options`, `build_card`, the shared text and art steps, raster frames) and `vector.rs` (the borderless vector template's frame: shapes by textbox height, clipped color layers, masks), ported from Proxyshop's template classes with an MPL header. Its tests build cards on a small made-up template with the rectangle font.
- `crates/fill`: `patchmatch.rs` (the inpainter, optionally started from a guide picture), `lama.rs` (the LaMa model as a guide for PatchMatch; needs the `onnx` feature and two downloaded files), `sdcpp.rs` (Stable Diffusion inpainting by running `sd-cli` with a generated script; `sdcpp` feature; the tests use a stand-in program, the real one has been run once by the owner, see `docs/TIER3.md`), `lib.rs` (`fill_edges`, `FillProvider`, `PatchMatch`, `fill_with_fallback`). `crates/scryfall/src/models.rs` downloads the model and ONNX Runtime. The engine's `FillPlan` says what to fill; `FillPlan::edit` adds the result as a `fill` layer beneath the art.
- `crates/engine/src/svg.rs`: the SVG reader that turns a set symbol into shape layers. `cargo run --release -p cardsmith-engine --example svg_check -- <dir>` runs it over a folder of SVGs and lists what it could not read.
- `crates/scryfall`: `lib.rs` (`Client`, the `Transport` trait tests replace, the cache, `card_data`), `bulk.rs` (bulk download, index and lookups), `hexproof.rs` (set symbols package and set data from hexproof.io). Its tests use a pretend network; nothing in `cargo test` touches the real one.
- Text tests use `crates/core/tests/fixtures/blocks.ttf`, a font of rectangles made by `tools/make-test-font.py`. Real fonts are only ever read from `fixtures/local/fonts`.
- `crates/wasm`: the C ABI the webview calls, including the card workbench (`cs_open_template`, `cs_build_card`, `cs_add_fill`, ...). `apps/desktop/src/lib/core.ts` is its TypeScript wrapper.
- `crates/engine/src/workbench.rs`: a card being built and rebuilt in an open session (one undo step per build, late fills). `crates/fill/src/tiers.rs`: the installed fill tiers and the fallback order, shared by the CLI and the app.
- `apps/desktop/scripts/card-timing.ts` times opening a template, building, rebuilding, hiding and dragging in the WebAssembly core outside the app; run it before and after any change to how the session redraws.
- `apps/desktop`: `src/App.svelte` (canvas, the move tool with its handles and snapping, on-canvas text editing, brush, new layers, the layers list and the selected layer's properties), `src/CardPanel.svelte` (folders, template, card search and form, art, fill), `src/lib/native.ts` (the bridge to `src-tauri/src/lib.rs`, whose commands read the chosen folders, ask Scryfall and hexproof.io, and make fills). `VITE_CARDSMITH_DEMO='{"template":"…","card":"…","art":"…","tier":"…"}'` with `CARDSMITH_TEMPLATES`, `CARDSMITH_FONTS` and `CARDSMITH_CACHE` starts the app straight on a built card, for checks on a virtual screen (`xvfb-run -a pnpm dev`).
- `crates/cli`: `cardsmith render | samples | hashes | bench | compare | text | card | bulk | symbols | models | build | sheet`.
- `packages/importer/src/import`: `convert.ts` (PSD layer tree to document), `paths.ts` (vector paths), `effects.ts` (layer effects), `bindings.ts` (Proxyshop layer names to bindings), `cli.ts`.
- `tests/golden`: `run.ts` (the runner), `thresholds.json`, `states.json`. `tests/text`: `run.ts`, `cards.json`. `tests/cards`: `run.ts`, `cards.json` (refreshed by `tools/fetch-suite-cards.py`), `art.png` (a made-up scene from `tools/make-test-art.py`).
- The core takes no dependency that cannot build for `wasm32-unknown-unknown`, and uses no threads, files or clocks.
