import type { Counts, Survey } from "./survey.js";

function table(counts: Counts, header: string, limit?: number): string {
  const rows = Object.entries(counts).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  if (rows.length === 0) return "_None found._\n";
  const shown = limit ? rows.slice(0, limit) : rows;
  const lines = [`| ${header} | Count |`, "|---|---:|", ...shown.map(([k, v]) => `| ${escape(k)} | ${v} |`)];
  if (limit && rows.length > limit) lines.push(`| _…and ${rows.length - limit} more (see survey.json)_ | |`);
  return lines.join("\n") + "\n";
}

function escape(text: string): string {
  return (text === "" ? "(empty name)" : text).replace(/\|/g, "\\|").replace(/\n/g, " ");
}

/** Render the survey as Markdown. This is the generated body of docs/SURVEY.md. */
export function renderMarkdown(survey: Survey): string {
  const out: string[] = [];
  const h = (title: string) => out.push(`\n## ${title}\n`);

  out.push(`Surveyed **${survey.files.length}** file(s), **${survey.totalLayers}** layers in total.\n`);

  h("Files");
  out.push("| File | Size (px) | DPI | Mode | Bits | Layers | Max depth |", "|---|---|---:|---|---:|---:|---:|");
  for (const f of survey.files) {
    out.push(
      `| ${escape(f.file)} | ${f.width}×${f.height} | ${f.dpi ?? "?"} | ${f.colorMode} | ${f.bitsPerChannel ?? "?"} | ${f.layers} | ${f.maxDepth} |`,
    );
  }
  if (survey.failed.length) {
    out.push("\n**Failed to parse:**\n");
    for (const f of survey.failed) out.push(`- ${escape(f.file)}: ${f.error}`);
  }

  h("Layer kinds");
  out.push(table(survey.layerKinds, "Kind"));
  h("Blend modes");
  out.push(table(survey.blendModes, "Blend mode"));

  h("Layer effects");
  out.push("### Enabled\n", table(survey.effects, "Effect"));
  out.push("### Defined but switched off\n", table(survey.effectsDisabled, "Effect"));
  out.push("### Blend modes used inside effects\n", table(survey.effectBlendModes, "Effect: mode"));
  out.push("### Effect options\n", table(survey.effectOptions, "Option"));

  h("Adjustment layers");
  out.push(table(survey.adjustments, "Type"));
  h("Smart objects");
  out.push(table(survey.smartObjects, "Kind"));

  h("Text");
  out.push(`Text layers: **${survey.text.layers}**\n`);
  out.push("### Fonts (layers using each)\n", table(survey.text.fonts, "PostScript name"));
  out.push("### Features\n", table(survey.text.features, "Feature"));
  out.push("### Justification\n", table(survey.text.justification, "Justification"));
  out.push("### Anti-aliasing\n", table(survey.text.antiAlias, "Mode"));
  out.push("### Warp\n", table(survey.text.warp, "Style"));

  h("Masks and clipping");
  out.push(table(survey.masks, "Usage"));
  h("Other layer properties");
  out.push(table(survey.layerProps, "Property"));

  h("Layer names vs. Proxyshop `src/enums/layers.py`");
  if (!survey.layerNames.checked) {
    out.push("_Not checked: no `layers.py` was available._\n");
  } else {
    const sum = (c: Counts) => Object.values(c).reduce((a, b) => a + b, 0);
    const m = survey.layerNames;
    out.push(
      `- Layers whose name is a known constant: **${sum(m.matched)}** (${Object.keys(m.matched).length} distinct names)`,
      `- Layers with other names: **${sum(m.unmatched)}** (${Object.keys(m.unmatched).length} distinct names)`,
      `- Known constants never seen: **${m.unused.length}**\n`,
    );
    out.push("### Matched names\n", table(m.matched, "Layer name"));
    out.push("### Unmatched names (top 60)\n", table(m.unmatched, "Layer name", 60));
    out.push("### Known constants not used by any surveyed file\n");
    out.push(m.unused.length ? m.unused.map((n) => `\`${n}\``).join(", ") + "\n" : "_None._\n");
  }
  return out.join("\n");
}
