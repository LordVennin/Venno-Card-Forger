# Known differences from the references

What `pnpm test:golden` does not yet match, and why. Remove an entry when it is fixed.

Measured with text included (fonts loaded). Default states are compared with the composites embedded in the PSDs; named states with exports the owner made in Photopea. With text left out the figures were 107, 83, 83, 85, 72 and 41.3 dB: the drop is text edge pixels, see "Text edges".

| Template and state | Reference | PSNR | Status |
|---|---|---:|---|
| `normal` default | PSD composite | 44.7 dB | Matches within limits |
| `normal` white legendary creature | Photopea export | 42.0 dB | Matches within limits |
| `normal-extended` default | PSD composite | 43.1 dB | Matches within limits |
| `normal-extended` white legendary creature | Photopea export | 41.8 dB | Matches within limits |
| `borderless-vector` default | PSD composite | 41.7 dB | Matches within limits; see "Anti-aliasing" |
| `borderless-vector` frame | Photopea export | 38.5 dB | Matches within its limits; see "Fine detail of bevels" and "Text edges" |

## Open

### Text edges
Text is drawn from the fonts in `fixtures/local/fonts` (the golden runner leaves text areas out only when that folder is empty). Across the three templates all 62 text layers have their ink within 1.4 px of where Photoshop recorded it, and baselines sit on whole pixels as Photoshop's do. What differs is the edge pixels: Photoshop's "sharp" anti-aliasing and its text blending gamma (1.45) make white text about 3% heavier than ours. Matching that was tried (applying the gamma) and removed only a tenth of the difference, so it is not done.

One layer disagrees: `borderless-vector`, the first `Rules Text - Creature Flip`, whose stored bounds are 103 px lower than its own text box and transform put it. Its box has a negative top (`boxBounds` starts at -46.9) unlike every other rules box. It is hidden by default and repositioned by the card engine anyway.

Not supported in text, each with an importer warning: faux bold and faux italic, baseline shift, all caps and small caps set as a style (small-caps fonts are fine), warps, vertical text, justified paragraphs, rotation.

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
