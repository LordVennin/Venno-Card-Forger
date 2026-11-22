import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import type { FitOptions, FontSpec, Run } from "./fitParagraph.js";

const WHITE: [number, number, number] = [255, 251, 213];
const BLUE: [number, number, number] = [170, 224, 250];

/** Made-up rules text shaped like a wordy real card: symbols, reminder text, flavor. */
export const SAMPLE_RUNS: Run[] = [
  { kind: "text", text: "Flying, vigilance " },
  { kind: "text", italic: true, text: "(Attacking doesn't cause this creature to tap.)" },
  { kind: "text", text: "\n" },
  { kind: "symbol", label: "2", color: [202, 195, 192] },
  { kind: "symbol", label: "W", color: WHITE },
  { kind: "symbol", label: "U", color: BLUE },
  { kind: "text", text: ", " },
  { kind: "symbol", label: "T" },
  { kind: "text", text: ": Exile target nonland permanent you don't control until this creature leaves the battlefield. Activate only as a sorcery.\n" },
  { kind: "text", text: "Whenever you cast a spell, you may pay " },
  { kind: "symbol", label: "1" },
  { kind: "text", text: ". If you do, draw a card, then add " },
  { kind: "symbol", label: "W", color: WHITE },
  { kind: "text", text: " or " },
  { kind: "symbol", label: "U", color: BLUE },
  { kind: "text", text: ".\n" },
  { kind: "text", italic: true, text: "\"The lighthouse keeps no ships from the rocks. It only shows them which rocks are mine.\"" },
];

export const SAMPLE_OPTIONS: FitOptions = {
  width: 560,
  height: 300,
  fontSize: 36,
  minFontSize: 10,
  step: 0.5,
  leading: 1.15,
};

/** DejaVu Serif (open license), loaded from the dejavu-fonts-ttf dev dependency. */
export async function loadSampleFont(): Promise<FontSpec> {
  const require = createRequire(import.meta.url);
  const faces = await Promise.all(
    ["DejaVuSerif.ttf", "DejaVuSerif-Italic.ttf"].map(async (file) => {
      const buf = await readFile(require.resolve(`dejavu-fonts-ttf/ttf/${file}`));
      return buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength) as ArrayBuffer;
    }),
  );
  return { family: "DejaVu Serif", faces };
}
