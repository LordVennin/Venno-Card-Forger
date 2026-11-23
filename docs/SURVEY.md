# PSD feature survey

**This list, and nothing beyond it, is the feature set the renderer and importer must support** (SPEC Phase 0, task 4). If a later template needs something that is not here, re-run the survey with that template included and update this file first.

- Source: the three first-set Proxyshop templates, `normal.psd`, `normal-extended.psd` and `borderless-vector.psd`, as distributed by Proxyshop's template manifest on 2026-11-22.
- Regenerate: `pnpm survey fixtures/local/templates`, then copy `fixtures/local/survey/survey.md` under "Generated report" below.
- The PSDs themselves are never committed. They live in the gitignored `fixtures/local/templates/`.

## What this means for the renderer

Read this section as the scope. The generated tables below it are the evidence.

**Document.** All three files are 3264×4440 px, 1200 DPI, RGB, 8 bits per channel. Two carry an ICC profile, one does not. Groups nest up to 6 deep.

**Layer kinds: four.** Pixel, solid-color shape (a color fill clipped by a vector mask), text, and group. There are **no smart objects and no adjustment layers** in the first set, so Phase 1 task 3's "adjustments" list is empty and Phase 2 task 3 (smart objects) has nothing to do yet.

**Blend modes: four on layers.** `normal`, `soft light`, `overlay`, `linear light`. Groups are `pass through` (80) or `normal` (4, isolated). Effects add `multiply`, `screen` and `luminosity` internally.

**Layer effects: seven.** Stroke (outside, inside and center; color fill only; up to two on one layer), drop shadow (with spread, and a Gaussian contour on three), bevel (inner and outer, chisel hard, with soften), gradient overlay (linear and radial, solid multi-stop gradients, luminosity or normal blend), outer glow, inner shadow, and color overlay (`solidFill`).

- Color overlay and most drop shadows are defined on layers whose whole layer style is switched off. Proxyshop turns some of these on at render time, so they are in scope even though the default state does not show them. The engine needs a "layer style enabled" flag separate from each effect's own flag.
- Not present, so not built: inner glow, satin, pattern overlay, gradient or pattern strokes, noise gradients.
- **Effect scale.** Every layer with effects records an effects scale of 16.667 (1200 DPI ÷ 72). It is metadata, not a multiplier: the sizes in the file are already document pixels (confirmed in Phase 2 by rendering against the composites).
- Some effects use the document's global light (90° in one file, 102° in the other two, with altitudes of 30° and 21°). The importer resolves it to a plain angle on each effect.

**Masks and clipping.**
- Vector masks on layers (73) and on groups (8), some with several subpaths, some disabled.
- Raster masks on layers (25) and on groups (8). Most are unlinked from the layer, most default to white outside their bounds, and 9 are disabled. No mask uses density or feather.
- Clipping: 38 clipped layers, almost all solid-color shapes clipped to the layer below.
- Fill opacity below 100% on 18 layers. Fill opacity fades the layer's pixels but not its effects, so it cannot be folded into opacity.
- 443 of 595 layers are hidden by default. Templates are mostly switched-off alternatives, which is what the engine's toggles select between.
- Not present: blend-if ranges, knockout, "blend clipped layers as group" off, "mask hides effects".

**Shapes.** 105 solid-color shape layers. 7 also have a shape stroke (solid color, aligned center or inside). No gradient or pattern fills.

**Text.** 62 text layers, no warp, no text on path, all horizontal.
- Fonts the user must supply: `PlantinMTPro-Regular`, `PlantinMTPro-SemiboldIt`, `BelerenProxy-Bold`, `BelerenSmallCaps-Bold`, `Gotham-Medium`, `mana`, `Proxyglyph`. None can be committed (rule 7). The app needs a missing-font warning that names each one.
- Both point text (42) and box text (20).
- Per-run styling: multiple style runs (20 layers) and multiple fonts in one layer (6).
- Fixed leading and auto leading, tracking, horizontal scale, one baseline shift, auto kerning switched off on 14.
- Paragraph spacing (space before/after) on 20 layers, left/center/right justification, and auto-hyphenate flagged on 33.
- 41 text layers carry a scale in their transform. The importer has to fold that into the font size.
- No faux bold or faux italic, no small caps or all caps flag, no underline, no text stroke.

**Layer names.** 527 of 595 layers (89%) use a name from Proxyshop's `layers.py`, so automatic bindings should resolve most of each template. The 68 others are mostly decorative sub-layers and per-DFC icon layers (`sunmoondfc`, `convertdfc`, ...), listed below.

## Spike findings worth keeping

From the CanvasKit text spike (`packages/render/src/spike`):

- CanvasKit's Paragraph API runs headless in Node and does everything rules text needs: inline placeholders with measured rects, style runs matched to regular and italic faces of one family, fixed-width layout, and height measurement. Shrink-to-fit is a plain loop of layout passes and takes a few milliseconds.
- If a family has no italic face, SkParagraph silently synthesizes a slant. The text engine must check for missing faces itself and warn.
- ag-psd reads structure and the composite image in Node with no canvas package, given `useImageData: true` and a stub `createImageData` (`packages/importer/src/headless.ts`).

## Golden references

`pnpm composite fixtures/local/templates` writes each PSD's embedded composite to `fixtures/local/golden/<name>.default.png`. These show the default visibility state only, where most layers are hidden, so they check compositing of the base frame and placeholder text but not the per-color toggles. Full-card references still need Proxyshop on Windows or Photopea exports (SPEC open question 4).

---

## Generated report

Surveyed **3** file(s), **595** layers in total.


### Files

| File | Size (px) | DPI | Mode | Bits | Layers | Max depth |
|---|---|---:|---|---:|---:|---:|
| borderless-vector.psd | 3264×4440 | 1200 | RGB | 8 | 239 | 6 |
| normal-extended.psd | 3264×4440 | 1200 | RGB | 8 | 166 | 3 |
| normal.psd | 3264×4440 | 1200 | RGB | 8 | 190 | 3 |

### Layer kinds

| Kind | Count |
|---|---:|
| pixel | 318 |
| shape (color fill) | 105 |
| group | 84 |
| text | 62 |
| pixel (empty) | 26 |


### Blend modes

| Blend mode | Count |
|---|---:|
| normal | 507 |
| pass through (group) | 80 |
| normal (group) | 4 |
| soft light | 2 |
| linear light | 1 |
| overlay | 1 |


### Layer effects

#### Enabled

| Effect | Count |
|---|---:|
| stroke | 29 |
| dropShadow | 7 |
| bevel | 6 |
| gradientOverlay | 6 |
| outerGlow | 2 |
| innerShadow | 1 |

#### Defined but switched off

| Effect | Count |
|---|---:|
| dropShadow (layer style off) | 28 |
| solidFill (layer style off) | 4 |
| dropShadow (effect off) | 1 |

#### Blend modes used inside effects

| Effect: mode | Count |
|---|---:|
| stroke: normal | 29 |
| dropShadow: multiply | 7 |
| bevel: multiply | 6 |
| bevel: screen | 6 |
| gradientOverlay: luminosity | 5 |
| outerGlow: multiply | 2 |
| gradientOverlay: normal | 1 |
| innerShadow: multiply | 1 |

#### Effect options

| Option | Count |
|---|---:|
| effects scale != 100% | 80 |
| stroke fill: color | 29 |
| stroke position: outside | 20 |
| stroke position: inside | 7 |
| bevel technique: chisel hard | 6 |
| gradientOverlay gradient: solid | 6 |
| bevel style: inner bevel | 5 |
| gradientOverlay type: linear | 5 |
| dropShadow contour: Gaussian | 3 |
| multiple stroke on one layer | 2 |
| stroke position: center | 2 |
| bevel style: outer bevel | 1 |
| gradientOverlay type: radial | 1 |


### Adjustment layers

_None found._


### Smart objects

_None found._


### Text

Text layers: **62**

#### Fonts (layers using each)

| PostScript name | Count |
|---|---:|
| PlantinMTPro-Regular | 25 |
| BelerenProxy-Bold | 16 |
| Gotham-Medium | 9 |
| mana | 8 |
| BelerenSmallCaps-Bold | 6 |
| Proxyglyph | 6 |
| PlantinMTPro-SemiboldIt | 1 |

#### Features

| Feature | Count |
|---|---:|
| point text | 42 |
| scaled transform | 41 |
| auto hyphenate | 33 |
| auto leading | 27 |
| fixed leading | 25 |
| box text | 20 |
| multiple style runs | 20 |
| paragraph spacing | 20 |
| tracking != 0 | 18 |
| auto kerning off | 14 |
| horizontal scale != 100% | 10 |
| multiple fonts in one layer | 6 |
| baseline shift | 1 |

#### Justification

| Justification | Count |
|---|---:|
| left | 44 |
| center | 12 |
| right | 6 |

#### Anti-aliasing

| Mode | Count |
|---|---:|
| sharp | 60 |
| strong | 2 |

#### Warp

| Style | Count |
|---|---:|
| none | 62 |


### Masks and clipping

| Usage | Count |
|---|---:|
| vector mask on layer | 73 |
| clipping layer | 38 |
| clipping layer: shape (color fill) | 34 |
| raster mask unlinked from layer | 32 |
| raster mask default white (reveal outside) | 28 |
| raster mask on layer | 25 |
| raster mask disabled | 9 |
| raster mask on group | 8 |
| vector mask on group | 8 |
| vector mask with multiple subpaths | 7 |
| vector mask disabled | 6 |
| clipping layer: pixel | 4 |
| vector mask raster cache | 2 |


### Other layer properties

| Property | Count |
|---|---:|
| hidden | 443 |
| fill opacity < 100% | 18 |
| opacity < 100% | 8 |
| shape stroke | 7 |


### Layer names vs. Proxyshop `src/enums/layers.py`

- Layers whose name is a known constant: **527** (85 distinct names)
- Layers with other names: **68** (52 distinct names)
- Known constants never seen: **105**

#### Matched names

| Layer name | Count |
|---|---:|
| B | 21 |
| G | 21 |
| R | 21 |
| U | 21 |
| W | 21 |
| Gold | 19 |
| Colorless | 16 |
| Artifact | 15 |
| BG | 14 |
| BR | 14 |
| GU | 14 |
| GW | 14 |
| Land | 14 |
| RG | 14 |
| RW | 14 |
| UB | 14 |
| UR | 14 |
| WB | 14 |
| WU | 14 |
| Normal | 11 |
| Shape | 10 |
| Medium | 7 |
| Nickname | 7 |
| Short | 7 |
| Tall | 7 |
| Rules Text - Creature | 6 |
| Rules Text - Noncreature | 6 |
| Typeline | 6 |
| Bottom | 5 |
| Card Name | 5 |
| PT Box | 5 |
| Top | 5 |
| Vehicle | 5 |
| Border | 4 |
| Color Indicator | 4 |
| Legendary Crown | 4 |
| Rules Text - Creature Flip | 4 |
| Rules Text - Noncreature Flip | 4 |
| Textbox Reference | 4 |
| Textless | 4 |
| Transform | 4 |
| Artist | 3 |
| Collector | 3 |
| Companion | 3 |
| Divider | 3 |
| Expansion Reference | 3 |
| Full Art Frame | 3 |
| Layer 1 | 3 |
| Legal | 3 |
| Mana Cost | 3 |
| Name & Title Boxes | 3 |
| Power / Toughness | 3 |
| PT Reference | 3 |
| Set | 3 |
| Text and Icons | 3 |
| Textbox | 3 |
| TF Front | 3 |
| Typeline Shift | 3 |
| 1/2 | 2 |
| Art Frame | 2 |
| Background | 2 |
| Hollow Crown Shadow | 2 |
| Land Pinlines & Textbox | 2 |
| Left | 2 |
| Legendary | 2 |
| Legendary Border | 2 |
| MDFC | 2 |
| Normal Border | 2 |
| Nyx | 2 |
| Pinlines | 2 |
| Pinlines & Textbox | 2 |
| Right | 2 |
| Shadows | 2 |
| UBR | 2 |
| 1/3 | 1 |
| 2/3 | 1 |
| Back | 1 |
| Card Name Shift | 1 |
| Effects | 1 |
| Flipside Power / Toughness | 1 |
| Front | 1 |
| Masks | 1 |
| MDFC Back | 1 |
| MDFC Front | 1 |
| Token | 1 |

#### Unmatched names (top 60)

| Layer name | Count |
|---|---:|
| Border Crop | 3 |
| Arrow | 2 |
| Circle | 2 |
| compasslanddfc | 2 |
| convertdfc | 2 |
| Full Art Border (leave on) | 2 |
| meld | 2 |
| mooneldrazidfc | 2 |
| originpwdfc | 2 |
| Overlay | 2 |
| pinline | 2 |
| sunmoondfc | 2 |
| Textbox & Art Frame | 2 |
| Twins | 2 |
| upsidedowndfc | 2 |
| 2 | 1 |
| 3 | 1 |
| Art Frame Medium | 1 |
| Art Frame Normal | 1 |
| Art Frame Short | 1 |
| Art Frame Tall | 1 |
| Art Frame Textless | 1 |
| artifactoverlay | 1 |
| backing | 1 |
| Backing Circle | 1 |
| backup | 1 |
| Color Fill 1 | 1 |
| Dark | 1 |
| Divider TF | 1 |
| fandfc | 1 |
| Full Art Border | 1 |
| Glass | 1 |
| Highlight | 1 |
| Medium TF Back | 1 |
| Medium TF Front | 1 |
| Name | 1 |
| Name Box newww | 1 |
| Normal TF Back | 1 |
| Normal TF Front | 1 |
| Nyxcrown | 1 |
| Overlays | 1 |
| PT Box Dark | 1 |
| PT Box Textless | 1 |
| Shading | 1 |
| Short TF Back | 1 |
| Short TF Front | 1 |
| Tall TF Back | 1 |
| Tall TF Front | 1 |
| Textless Transform | 1 |
| Transform Curve | 1 |
| triangle | 1 |
| triangles | 1 |

#### Known constants not used by any surveyed file

`1/4`, `1/5`, `2/5`, `3/4`, `3/5`, `4/5`, `Ability Text`, `Adventure`, `Adventure Name`, `Adventure Typeline`, `Adventure Typeline Accent`, `BGU`, `BRG`, `BRGW`, `Banner`, `Basic Art Frame`, `Borderless`, `Borderless Frame`, `Card Name - Adventure`, `Chaos Ability`, `Chaos Symbol`, `Class`, `Classic`, `Collector Reference`, `Colon`, `Contrast`, `Cost`, `Creator`, `Creature`, `Darken`, `Defense`, `Defense Reference`, `Divider - Adventure`, `Expansion Symbol`, `Extended`, `First Ability`, `Fourth Ability`, `Frame`, `Full`, `Fullart`, `GUR`, `GWU`, `GWUB`, `Hybrid`, `Lighten`, `Loyalty Graphics`, `Loyalty Reference`, `Mana Cost - Adventure`, `Mini`, `Mutate`, `Mutate Reference`, `NFS`, `NFS - Creature`, `Name Reference`, `Name Reference Legendary`, `Name Reference Non-Legendary`, `Non-Creature`, `Non-Legendary`, `None`, `Nonland`, `One Line`, `Outline`, `PT and Level Boxes`, `Phenomenon`, `Pinlines & Saga Stripe`, `Planeswalker`, `Planeswalker Art Frame`, `Prototype Mana Cost`, `Prototype Manabox`, `Prototype PT Box`, `Prototype Power / Toughness`, `Prototype Rules Text`, `Prototype Textbox`, `RGW`, `RGWU`, `RWB`, `Rules Text`, `Rules Text - Adventure`, `Rules Text - Flip`, `Saga`, `Scryfall Scan Frame`, `Second Ability`, `Snow`, `Stage`, `Starting Loyalty`, `Static Ability`, `Static Text`, `Storybook`, `Stripe`, `TF Back`, `Text`, `Textbox Reference - Adventure`, `Textbox Reference Land`, `Third Ability`, `Typeline - Adventure`, `Typeline Reference`, `UBRG`, `URW`, `WBG`, `WUB`, `WUBR`, `WUBRG`, `Wings`, `mdfc-back`, `mdfc-front`
