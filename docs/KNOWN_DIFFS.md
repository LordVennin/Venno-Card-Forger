# Known differences from the references

What `pnpm test:golden` does not yet match, and why. Remove an entry when it is fixed.

Measured on 2026-11-24. Default states are compared with the composites embedded in the PSDs; named states with exports the owner made in Photopea.

| Template and state | Reference | PSNR | Status |
|---|---|---:|---|
| `normal` default | PSD composite | 107 dB | Matches |
| `normal` white legendary creature | Photopea export | 83 dB | Matches |
| `normal-extended` default | PSD composite | 83 dB | Matches |
| `normal-extended` white legendary creature | Photopea export | 85 dB | Matches |
| `borderless-vector` default | PSD composite | 72 dB | Matches; see "Anti-aliasing" |
| `borderless-vector` frame | Photopea export | 41.3 dB | Matches within its limits; see "Fine detail of bevels" |

## Open

### Text is not drawn
Text layers are imported (box, font, size, leading, tracking, alignment, color, runs) but not rendered until Phase 3. The golden runner leaves the area of every visible text layer out of the comparison. That exclusion comes out when text rendering lands.

### Effects are checked against Photopea, not Photoshop
The frame state exercises strokes, bevels, gradient overlays, a drop shadow, fill opacity, clipped soft-light and overlay layers and vector masks, and matches Photopea at 41.3 dB. The owner also compared the P/T box against cards they made in Proxyshop (real Photoshop) and confirmed Photopea's rendering of it. Photopea is still not Photoshop, so this is good evidence, not proof. What the comparison settled:

- gradient overlays honour Photoshop's "smoothness" (each segment eases in and out),
- a gradient's length is the line through the bounds' center at its angle, from edge to opposite edge,
- bevels honour Photoshop's "Contour" sub-option (a drawn cross-section in place of a straight ramp). The importer used to drop it silently, which made the P/T box look lit from the wrong side.

Still estimates, because nothing visible in the references pins them down: blur width for a shadow or glow "size" beyond the one drop shadow here, the Gaussian contour's curve, the radius of radial gradients, and outer glow and inner shadow (not in any reference state).

### Fine detail of bevels on curved ends
Both renderers draw thin streaks where a chiselled bevel follows a curve, and the streaks differ. This is the worst block (7.9/255 mean error) in the frame state, and the reason its block limit is 9 instead of 6.

### Anti-aliasing on vector edges
Shape and vector-mask edges are rasterized by tiny-skia, not Photoshop, so partially covered edge pixels can differ. This is the 51/255 difference in `borderless-vector`: a one-pixel line along curved outlines. Not planned to change.

### One imported shape is approximate
`borderless-vector`: `/Pinlines/Shape/Card Name/MDFC/Textbox & Art Frame/Textbox & Art Frame` is an inverted shape made of overlapping subpaths. It is hidden by default and only used for modal double-faced cards. The importer warns about it.

### Vector templates are bound but not yet colored
`borderless-vector` recolors its shapes at render time instead of switching between pre-drawn color layers. Its layers are bound, but applying colors is the card engine's job (Phase 5).

## Deferred

### Binding review screen
SPEC Phase 2 task 4 asks for a screen to correct bindings the importer could not resolve. All three templates resolve completely, and the app cannot open template folders yet, so the screen is deferred to the editor UI (Phase 7). Until then, bindings can be corrected by editing `bindings` in the template's `manifest.json`; the importer prints any it could not resolve.
