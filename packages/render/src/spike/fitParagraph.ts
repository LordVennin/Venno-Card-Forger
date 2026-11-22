/**
 * Phase 0 spike: prove that CanvasKit's Paragraph API can lay out rules text
 * headless, with inline placeholders standing in for mana symbols, mixed
 * regular/italic runs, a fixed width, and shrink-to-fit-height.
 *
 * This is throwaway-shaped code. The real text engine lands in packages/text
 * (Phase 3); what must survive is the approach, not this file.
 */
import type { Canvas, CanvasKit, Paragraph, TypefaceFontProvider } from "canvaskit-wasm";

export type Run =
  | { kind: "text"; text: string; italic?: boolean }
  /** Inline symbol stand-in. `label` is drawn inside the placeholder disc. */
  | { kind: "symbol"; label: string; color?: [number, number, number] };

export type FontSpec = {
  family: string;
  /** Raw font file bytes, one entry per face (regular, italic, ...). */
  faces: ArrayBuffer[];
};

export type FitOptions = {
  /** Box the text must fit, in pixels. */
  width: number;
  height: number;
  /** Starting (maximum) font size in pixels. */
  fontSize: number;
  /** Smallest size we are willing to shrink to. */
  minFontSize?: number;
  /** Shrink step in pixels. */
  step?: number;
  /** Line height as a multiple of font size. Kept constant, so leading scales with the font. */
  leading?: number;
  /** Symbol diameter as a multiple of font size. */
  symbolScale?: number;
};

export type Rect = { x: number; y: number; w: number; h: number };

export type FitResult = {
  fontSize: number;
  /** Laid-out height at `fontSize`. */
  height: number;
  /** Height at the starting size, before shrinking. */
  initialHeight: number;
  lineCount: number;
  /** True when the text fits the box (false if we hit `minFontSize` first). */
  fits: boolean;
  /** Number of layout passes it took. */
  iterations: number;
  /** One rect per symbol run, in run order, relative to the paragraph origin. */
  symbolRects: Rect[];
};

export function makeFontProvider(ck: CanvasKit, fonts: FontSpec[]): TypefaceFontProvider {
  const provider = ck.TypefaceFontProvider.Make();
  for (const font of fonts) {
    for (const face of font.faces) provider.registerFont(face, font.family);
  }
  return provider;
}

function buildParagraph(
  ck: CanvasKit,
  fonts: TypefaceFontProvider,
  family: string,
  runs: Run[],
  fontSize: number,
  opts: FitOptions,
): Paragraph {
  const leading = opts.leading ?? 1.2;
  const symbol = fontSize * (opts.symbolScale ?? 0.85);
  const base = {
    color: ck.BLACK,
    fontFamilies: [family],
    fontSize,
    heightMultiplier: leading,
  };
  const builder = ck.ParagraphBuilder.MakeFromFontProvider(
    new ck.ParagraphStyle({ textStyle: base, textAlign: ck.TextAlign.Left }),
    fonts,
  );
  for (const run of runs) {
    if (run.kind === "symbol") {
      builder.addPlaceholder(
        symbol,
        symbol,
        ck.PlaceholderAlignment.Middle,
        ck.TextBaseline.Alphabetic,
        0,
      );
      continue;
    }
    builder.pushStyle(
      new ck.TextStyle({
        ...base,
        fontStyle: { slant: run.italic ? ck.FontSlant.Italic : ck.FontSlant.Upright },
      }),
    );
    builder.addText(run.text);
    builder.pop();
  }
  const paragraph = builder.build();
  builder.delete();
  paragraph.layout(opts.width);
  return paragraph;
}

function placeholderRects(paragraph: Paragraph): Rect[] {
  return paragraph.getRectsForPlaceholders().map(({ rect }) => ({
    x: rect[0]!,
    y: rect[1]!,
    w: rect[2]! - rect[0]!,
    h: rect[3]! - rect[1]!,
  }));
}

/**
 * Lay the runs out at `opts.fontSize`, then shrink until the paragraph's height
 * fits `opts.height`. Returns the paragraph (caller must `.delete()` it) plus
 * measurements.
 */
export function fitParagraph(
  ck: CanvasKit,
  fonts: TypefaceFontProvider,
  family: string,
  runs: Run[],
  opts: FitOptions,
): { paragraph: Paragraph; result: FitResult } {
  const min = opts.minFontSize ?? 6;
  const step = opts.step ?? 0.5;
  let fontSize = opts.fontSize;
  let paragraph = buildParagraph(ck, fonts, family, runs, fontSize, opts);
  const initialHeight = paragraph.getHeight();
  let iterations = 1;
  while (paragraph.getHeight() > opts.height && fontSize - step >= min) {
    paragraph.delete();
    fontSize -= step;
    paragraph = buildParagraph(ck, fonts, family, runs, fontSize, opts);
    iterations++;
  }
  const height = paragraph.getHeight();
  return {
    paragraph,
    result: {
      fontSize,
      height,
      initialHeight,
      lineCount: paragraph.getLineMetrics().length,
      fits: height <= opts.height,
      iterations,
      symbolRects: placeholderRects(paragraph),
    },
  };
}

/** Draw the fitted paragraph and its symbol stand-ins at (x, y). */
export function drawFitted(
  ck: CanvasKit,
  canvas: Canvas,
  paragraph: Paragraph,
  runs: Run[],
  result: FitResult,
  fonts: TypefaceFontProvider,
  family: string,
  x: number,
  y: number,
): void {
  canvas.drawParagraph(paragraph, x, y);
  const symbols = runs.filter((r) => r.kind === "symbol");
  const paint = new ck.Paint();
  paint.setAntiAlias(true);
  result.symbolRects.forEach((rect, i) => {
    const run = symbols[i]!;
    const [r, g, b] = run.color ?? [202, 195, 192];
    const cx = x + rect.x + rect.w / 2;
    const cy = y + rect.y + rect.h / 2;
    paint.setColor(ck.Color(r, g, b, 1));
    canvas.drawCircle(cx, cy, rect.w / 2, paint);

    // Label, centred in the disc, via a one-line paragraph.
    const builder = ck.ParagraphBuilder.MakeFromFontProvider(
      new ck.ParagraphStyle({
        textStyle: { color: ck.BLACK, fontFamilies: [family], fontSize: rect.h * 0.7 },
        textAlign: ck.TextAlign.Center,
      }),
      fonts,
    );
    builder.addText(run.label);
    const label = builder.build();
    builder.delete();
    label.layout(rect.w);
    canvas.drawParagraph(label, x + rect.x, cy - label.getHeight() / 2);
    label.delete();
  });
  paint.delete();
}

export type SpikeRender = { png: Uint8Array; result: FitResult; box: Rect };

/** Render the fitted text inside an outlined box on a white canvas, as PNG bytes. */
export function renderSpike(
  ck: CanvasKit,
  font: FontSpec,
  runs: Run[],
  opts: FitOptions,
  padding = 24,
): SpikeRender {
  const fonts = makeFontProvider(ck, [font]);
  const { paragraph, result } = fitParagraph(ck, fonts, font.family, runs, opts);
  const surface = ck.MakeSurface(opts.width + padding * 2, opts.height + padding * 2);
  if (!surface) throw new Error("CanvasKit could not create a raster surface");
  const canvas = surface.getCanvas();
  canvas.clear(ck.WHITE);

  const outline = new ck.Paint();
  outline.setStyle(ck.PaintStyle.Stroke);
  outline.setColor(ck.Color(200, 60, 60, 1));
  canvas.drawRect(ck.XYWHRect(padding, padding, opts.width, opts.height), outline);
  outline.delete();

  drawFitted(ck, canvas, paragraph, runs, result, fonts, font.family, padding, padding);

  const image = surface.makeImageSnapshot();
  const png = image.encodeToBytes();
  image.delete();
  paragraph.delete();
  surface.delete();
  fonts.delete();
  if (!png) throw new Error("PNG encode failed");
  return { png, result, box: { x: padding, y: padding, w: opts.width, h: opts.height } };
}
