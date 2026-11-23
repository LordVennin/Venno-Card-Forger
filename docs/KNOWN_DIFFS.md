# Known differences from the references

What `pnpm test:golden` does not yet match, and why. Remove an entry when it is fixed.

Measured on 2026-11-24 against the composites embedded in the three first-set PSDs (default layer state).

| Template | PSNR | Largest difference | Status |
|---|---:|---:|---|
| `normal` | 107 dB | 1/255 | Matches |
| `normal-extended` | 83 dB | 1/255 | Matches |
| `borderless-vector` | 72 dB | 51/255 on 0.02% of pixels | Matches within threshold; see "Anti-aliasing" |

## Open

### Text is not drawn
Text layers are imported (box, font, size, leading, tracking, alignment, color, runs) but not rendered until Phase 3. The golden runner leaves the area of every visible text layer out of the comparison. That exclusion comes out when text rendering lands.

### Layer effects are not yet checked against Photoshop
In the default state of all three templates, the shapes that carry strokes, bevels, gradient overlays and shadows are hidden, so the comparison above does not exercise them. `tests/golden/states.json` defines states that do (`borderless-vector.frame`, and a white legendary creature for the two raster templates). They render, and look right by eye, but have no reference image yet. Until they do, these are estimates:

- blur width for a given shadow or glow "size",
- the lighting of the chisel-hard bevel,
- the Gaussian contour's curve,
- the radius of radial gradients,
- the exact stacking order of bevel and stroke.

### Anti-aliasing on vector edges
Shape and vector-mask edges are rasterized by tiny-skia, not Photoshop, so partially covered edge pixels can differ. This is the 51/255 difference in `borderless-vector`: a one-pixel line along curved outlines. Not planned to change.

### One imported shape is approximate
`borderless-vector`: `/Pinlines/Shape/Card Name/MDFC/Textbox & Art Frame/Textbox & Art Frame` is an inverted shape made of overlapping subpaths. It is hidden by default and only used for modal double-faced cards. The importer warns about it.

### Vector templates are bound but not yet colored
`borderless-vector` recolors its shapes at render time instead of switching between pre-drawn color layers. Its layers are bound, but applying colors is the card engine's job (Phase 5).

## Deferred

### Binding review screen
SPEC Phase 2 task 4 asks for a screen to correct bindings the importer could not resolve. All three templates resolve completely, and the app cannot open template folders yet, so the screen is deferred to the editor UI (Phase 7). Until then, bindings can be corrected by editing `bindings` in the template's `manifest.json`; the importer prints any it could not resolve.
