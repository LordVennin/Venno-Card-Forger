/**
 * The symbols offered by the palette in the card form, in the braces Scryfall
 * and the card engine use. A test in `crates/cards` checks the engine can draw
 * every one of them, so the palette never offers a symbol that comes out as text.
 */
export const SYMBOL_GROUPS: { name: string; symbols: string[] }[] = [
  { name: "Common", symbols: ["{T}", "{Q}", "{W}", "{U}", "{B}", "{R}", "{G}", "{C}", "{X}", "{S}", "{E}"] },
  { name: "Numbers", symbols: ["{0}", "{1}", "{2}", "{3}", "{4}", "{5}", "{6}", "{7}", "{8}", "{9}", "{10}", "{11}", "{12}", "{13}", "{14}", "{15}", "{16}", "{20}"] },
  { name: "Hybrid", symbols: ["{W/U}", "{W/B}", "{U/B}", "{U/R}", "{B/R}", "{B/G}", "{R/G}", "{R/W}", "{G/W}", "{G/U}", "{2/W}", "{2/U}", "{2/B}", "{2/R}", "{2/G}"] },
  { name: "Phyrexian", symbols: ["{W/P}", "{U/P}", "{B/P}", "{R/P}", "{G/P}"] },
];

/** A background for a symbol's chip, by its color. */
export function symbolColor(symbol: string): string {
  const colors: Record<string, string> = { W: "#f3eecb", U: "#a9d8ee", B: "#b8aea8", R: "#eea98f", G: "#9fd0a8" };
  const letters = [...symbol].filter((c) => c in colors);
  if (letters.length === 1) return colors[letters[0]!]!;
  if (letters.length === 2) return `linear-gradient(135deg, ${colors[letters[0]!]} 50%, ${colors[letters[1]!]} 50%)`;
  return "#cfc8c2";
}
