<script lang="ts">
  import type { BuildReport, Core } from "./lib/core";
  import { native, type Extras, type Printing, type SetEntry, type Settings, type TemplateEntry, type TierState } from "./lib/native";
  import { SYMBOL_GROUPS, symbolColor } from "./lib/symbols";

  // The card panel: choose a template, find a card (or write one), give it
  // art, and the card is built on the canvas and rebuilt as its fields change.
  // The fill around the art is made on the native side and arrives when ready.

  let {
    core,
    busy = $bindable(false),
    focusField = $bindable(null),
    say,
    changed,
    opened,
    demoFill,
    demoReorder,
  }: {
    core: Core;
    busy?: boolean;
    /** A field of the card form to put the cursor in: set from outside, cleared here. */
    focusField?: string | null;
    /** Show a line in the status bar. */
    say: (text: string) => void;
    /** The document changed: copy what was redrawn to the screen and re-read the layers. */
    changed: () => void;
    /** A different document is open: fit it to the window. */
    opened: () => void;
    /** For the app's own check (see `demo`): select the art, mark out this rectangle and fill it. */
    demoFill?: (rect: [number, number, number, number]) => Promise<void>;
    /** For the app's own check: drag rows of the layers list with made-up pointer events and say what happened. */
    demoReorder?: () => Promise<string>;
  } = $props();

  let settings = $state<Settings>({ templatesDir: "", fontsDir: "" });
  let templates = $state<TemplateEntry[]>([]);
  let fontCount = $state(0);
  let folder = $state("");
  // The template folder the open document's images come from: the chosen template's, or
  // for a reopened card, the template it was built on, when that is in the templates folder.
  let assetFolder = $state("");
  let filePath = $state("");
  let tiers = $state<TierState[]>([]);

  // The card: Scryfall's JSON, edited in place by the form.
  let raw = $state<Record<string, any> | null>(null);
  let face = $state("");
  let extras = $state<Extras>({ setSize: null, symbolSvg: null, watermarkSvg: null });
  let search = $state("");
  let names = $state<string[]>([]);
  let prints = $state<Printing[]>([]);
  let printing = $state("");

  let artPath = $state("");
  let tier = $state("auto");
  let seed = $state(1);
  let prompt = $state("");
  let filling = $state("");
  let warnings = $state<string[]>([]);
  // Whether the card's text casts a shadow: the template's own choice, or the user's.
  let shadows = $state<"template" | "on" | "off">("template");

  // Sets to choose from by name, and a symbol of the user's own in place of the set's.
  let sets = $state.raw<SetEntry[]>([]);
  let setName = $state("");
  let ownSymbol = $state(false);

  // The symbol palette: open under the field it types into, narrowed by what follows a "{".
  let palette = $state<{ field: "mana_cost" | "oracle_text"; filter: string } | null>(null);
  let paletteTarget: HTMLInputElement | HTMLTextAreaElement | null = null;

  const RARITIES = ["common", "uncommon", "rare", "mythic", "special"];
  const TIER_NAMES: Record<string, string> = { sd: "Stable Diffusion", lama: "LaMa model", patchmatch: "PatchMatch" };
  const fail = (what: string) => (err: unknown) => say(`${what}: ${err instanceof Error ? err.message : err}`);
  const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

  native
    .settings()
    .then(async (saved) => {
      settings = saved;
      tiers = await native.fillTiers();
      native.sets().then((all) => (sets = all)).catch(() => {});
      await Promise.all([loadFonts(), listTemplates()]);
      await demo();
    })
    .catch(fail("The app's settings could not be read"));

  /**
   * For trying the whole path without clicking, as the app's own check does:
   * VITE_CARDSMITH_DEMO='{"template":"…","card":"…","art":"/path.png","tier":"patchmatch"}' pnpm dev
   */
  async function demo() {
    const text = import.meta.env.VITE_CARDSMITH_DEMO as string | undefined;
    if (!text) return;
    const want = JSON.parse(text) as { open?: string; rename?: string; template: string; card: string; art?: string; tier?: string; symbols?: boolean; export?: string; save?: string; fillSelection?: [number, number, number, number]; reorder?: boolean };
    tier = want.tier ?? tier;
    if (want.open) {
      // A saved card, reopened and changed through its form.
      await openDocument(want.open);
      if (want.rename) {
        edit("name", want.rename);
        await build();
      }
      if (want.export) await native.writeFile(want.export, core.exportPng());
      say(`Reopened ${want.open}: form ${raw ? "restored" : "not restored"}`);
      return;
    }
    folder = want.template;
    await openTemplate();
    if (want.art) {
      core.setArt(await native.artFile(want.art));
      artPath = want.art;
    }
    search = want.card;
    await find();
    if (want.symbols) palette = { field: "oracle_text", filter: "" };
    if (want.reorder) say(await demoReorder?.() ?? "no reorder check");
    if (want.fillSelection) await demoFill?.(want.fillSelection);
    if (want.save) await native.writeFile(want.save, core.saveAvailable());
    if (want.export) {
      await native.writeFile(want.export, core.exportPng());
      say(`Exported to ${want.export}`);
    }
  }

  let panel = $state<HTMLElement>();
  $effect(() => {
    if (!focusField) return;
    const field = panel?.querySelector<HTMLInputElement | HTMLTextAreaElement>(`[data-field="${focusField}"]`);
    focusField = null;
    field?.focus();
    field?.select();
  });

  // ---- folders ---------------------------------------------------------------------

  async function choose(which: "templatesDir" | "fontsDir") {
    const picked = await native.chooseFolder(which === "fontsDir" ? "Choose the folder your fonts are in" : "Choose the folder of imported templates");
    if (!picked) return;
    settings = { ...settings, [which]: picked };
    await native.saveSettings(settings);
    await (which === "fontsDir" ? loadFonts() : listTemplates());
  }

  async function loadFonts() {
    const files = await native.fonts();
    let loaded = 0;
    for (const name of files) {
      try {
        core.addFont(await native.font(name));
        loaded++;
      } catch {
        // Not a font this app can read; the status line says how many were.
      }
    }
    fontCount = loaded;
    if (files.length) {
      changed();
      say(`Loaded ${loaded} of ${files.length} font files`);
      if (raw) rebuild();
    }
  }

  async function listTemplates() {
    templates = await native.templates();
    if (settings.templatesDir && !templates.length) say("That folder has no imported templates in it. Import a PSD with: pnpm import:psd <psd> -o <folder>/<name>");
  }

  // ---- template --------------------------------------------------------------------

  async function openTemplate() {
    if (!folder) return;
    busy = true;
    say(`Opening ${folder}…`);
    try {
      const pack = await native.template(folder);
      core.openTemplate(pack.doc, pack.manifest);
      assetFolder = folder;
      filePath = "";
      opened();
      await loadImages();
      if (raw) await build();
      else say("Template open. Find a card, or write your own, to build it.");
    } catch (err) {
      fail("The template could not be opened")(err);
    } finally {
      busy = false;
    }
  }

  /**
   * Fetch the images that layers now showing need. A template's other images are never
   * loaded. Called after anything that can show a layer: a build, an undo, a tick in the list.
   */
  export async function loadImages() {
    if (!assetFolder) return;
    const missing = core.missingAssets();
    if (!missing.length) return;
    const files = await Promise.all(missing.map((m) => native.templateAsset(assetFolder, m.path).catch(() => null)));
    const got = missing.map((m, i) => ({ id: m.id, data: files[i] })).filter((f): f is { id: string; data: Uint8Array<ArrayBuffer> } => f.data !== null);
    got.forEach((f, i) => core.addAsset(f.id, f.data, i === got.length - 1));
    if (got.length) changed();
  }

  // ---- files -----------------------------------------------------------------------

  async function saveDocument() {
    try {
      const name = String(side?.name ?? raw?.name ?? "card").replace(/[\\/:*?"<>|]/g, "") || "card";
      const path = await native.saveDocument(filePath || `${name}.cardsmith`, () => core.saveAvailable());
      if (!path) return;
      filePath = path;
      say(`Saved ${fileName(path)}`);
    } catch (err) {
      fail("The card could not be saved")(err);
    }
  }

  async function openDocument(path?: string) {
    try {
      const file = path ? { path, data: await native.documentFile(path) } : await native.openDocument();
      if (!file) return;
      busy = true;
      core.openCardsmith(file.data);
      filePath = file.path;
      // Its other images are in the template it was built on, if that is still in the templates folder.
      const template = core.doc().templateId;
      assetFolder = templates.find((t) => t.id === template)?.folder ?? "";
      folder = "";
      raw = null;
      prints = [];
      warnings = [];
      artPath = "";
      opened();
      await loadImages();
      // With its template at hand, the card's form works again.
      if (assetFolder) {
        try {
          const pack = await native.template(assetFolder);
          const request = core.resumeCard(pack.doc, pack.manifest);
          if (request.scryfall) {
            raw = request.scryfall;
            face = request.name ?? "";
            extras = { setSize: request.setSize ?? null, symbolSvg: request.symbolSvg ?? null, watermarkSvg: request.watermarkSvg ?? null };
            shadows = request.textShadows == null ? "template" : request.textShadows ? "on" : "off";
            artPath = request.artPath ?? "";
            search = String(raw.name ?? "");
            folder = assetFolder;
            symbolFor = `${raw.set}/${raw.rarity}/${side?.type_line}`;
          }
        } catch {
          // Not a card built here, or saved before cards recorded how they were built: it opens as plain layers.
        }
      }
      say(`Opened ${fileName(file.path)}` + (template && !assetFolder ? `. Its template (${template}) is not in the templates folder, so layers that were hidden when it was saved have no picture.` : ""));
    } catch (err) {
      fail("That file could not be opened")(err);
    } finally {
      busy = false;
    }
  }

  // ---- finding a card ----------------------------------------------------------------

  let typing: ReturnType<typeof setTimeout> | undefined;
  function typed() {
    clearTimeout(typing);
    const text = search;
    // Not while typing every letter: Scryfall asks apps to be sparing.
    typing = setTimeout(async () => {
      names = await native.autocomplete(text).catch(() => []);
      if (names.some((n) => n.toLowerCase() === text.trim().toLowerCase())) find();
    }, 250);
  }

  async function find(set?: string, number?: string) {
    const name = search.trim();
    if (!name) return;
    try {
      say(`Looking up ${name}…`);
      const found = await native.card(name, set, number);
      raw = found.scryfall;
      face = found.name;
      extras = found.extras;
      printing = `${raw.set}/${raw.collector_number}`;
      if (!set) prints = await native.printings(String(raw.name).split(" // ")[0]!).catch(() => []);
      say(`${raw.name} · ${String(raw.set).toUpperCase()} ${raw.collector_number}`);
      await build();
    } catch (err) {
      fail("Not found")(err);
    }
  }

  function pickPrinting() {
    const [set, number] = printing.split("/");
    find(set, number);
  }

  function blank() {
    raw = { object: "card", layout: "normal", lang: "en", name: "New Card", mana_cost: "", colors: [], color_identity: [], type_line: "Creature", oracle_text: "", flavor_text: "", power: "", toughness: "", artist: "", set: "", collector_number: "", rarity: "common" };
    face = "";
    prints = [];
    extras = { setSize: null, symbolSvg: null, watermarkSvg: null };
    build();
  }

  // ---- the form --------------------------------------------------------------------

  /** The part of Scryfall's JSON the form edits: the card, or the wanted face of a two-faced one. */
  const side = $derived.by(() => {
    if (!raw) return null;
    const faces: Record<string, any>[] = Array.isArray(raw.card_faces) ? raw.card_faces : [];
    return faces.find((f) => String(f.name).toLowerCase() === face.toLowerCase()) ?? raw;
  });

  let pending: ReturnType<typeof setTimeout> | undefined;
  function rebuild() {
    clearTimeout(pending);
    pending = setTimeout(() => build(), 300);
  }

  function edit(field: string, value: string, on: "side" | "card" = "side") {
    const target = on === "side" ? side : raw;
    if (!target || !raw) return;
    target[field] = value;
    if (field === "name" && target !== raw) face = value;
    if (field === "mana_cost") {
      // The frame follows the card's colors, which for a card being written are those of its cost.
      const colors = [..."WUBRG"].filter((c) => value.toUpperCase().includes(c));
      target.colors = colors;
      raw.colors = colors;
      raw.color_identity = colors;
    }
    rebuild();
  }

  let symbolFor = "";
  async function setChanged() {
    if (!raw) return;
    const key = `${raw.set}/${raw.rarity}/${side?.type_line}`;
    if (key === symbolFor) return;
    symbolFor = key;
    const found = await native.setSymbol(String(raw.set ?? ""), String(raw.rarity ?? ""), String(side?.type_line ?? "")).catch(() => extras);
    extras = ownSymbol ? { ...found, symbolSvg: extras.symbolSvg } : found;
    rebuild();
  }

  /** The sets whose name or code has what was typed, newest first; the real sets before their token and promo sets. */
  const setChoices = $derived.by(() => {
    const typed = setName.trim().toLowerCase();
    if (typed.length < 2) return [];
    const main = (s: SetEntry) => ["core", "expansion", "masters", "draft_innovation", "commander", "funny"].includes(s.kind);
    const hits = sets.filter((s) => s.name.toLowerCase().includes(typed) || s.code === typed);
    return [...hits.filter(main), ...hits.filter((s) => !main(s))].slice(0, 30);
  });

  function setPicked() {
    const typed = setName.trim().toLowerCase();
    const set = sets.find((s) => s.name.toLowerCase() === typed) ?? sets.find((s) => s.code === typed);
    if (!set || !raw) return;
    raw.set = set.code;
    setChanged();
  }

  const currentSet = $derived(sets.find((s) => s.code === String(raw?.set ?? "").toLowerCase()));

  async function ownSymbolChosen(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    extras = { ...extras, symbolSvg: await file.text() };
    ownSymbol = true;
    rebuild();
  }

  function setsSymbolAgain() {
    ownSymbol = false;
    symbolFor = "";
    setChanged();
  }

  // ---- the symbol palette --------------------------------------------------------------

  /** After typing in a field that takes symbols: an unfinished "{…" before the cursor opens the palette. */
  function symbolsTyped(field: "mana_cost" | "oracle_text", el: HTMLInputElement | HTMLTextAreaElement) {
    const before = el.value.slice(0, el.selectionStart ?? el.value.length);
    const open = /\{([^{}\s]*)$/.exec(before);
    paletteTarget = el;
    palette = open ? { field, filter: open[1]!.toUpperCase() } : palette?.field === field && !palette.filter ? palette : null;
  }

  function togglePalette(field: "mana_cost" | "oracle_text", el: HTMLInputElement | HTMLTextAreaElement | null) {
    paletteTarget = el;
    palette = palette?.field === field ? null : { field, filter: "" };
  }

  const offered = $derived(
    SYMBOL_GROUPS.map((g) => ({ name: g.name, symbols: g.symbols.filter((s) => !palette?.filter || s.slice(1).startsWith(palette.filter)) })).filter((g) => g.symbols.length),
  );

  /** Put a symbol in at the cursor, in place of an unfinished "{…" if one was being typed. */
  function insertSymbol(symbol: string) {
    const el = paletteTarget;
    if (!el || !palette) return;
    const at = el.selectionStart ?? el.value.length;
    const before = el.value.slice(0, at).replace(/\{[^{}\s]*$/, "");
    const text = before + symbol + el.value.slice(el.selectionEnd ?? at);
    edit(palette.field, text);
    palette = { field: palette.field, filter: "" };
    el.value = text;
    el.focus();
    el.setSelectionRange(before.length + symbol.length, before.length + symbol.length);
  }

  // ---- art and building ------------------------------------------------------------------

  async function chooseArt() {
    const picked = await native.chooseArt();
    if (!picked) return;
    try {
      const size = core.setArt(await native.artFile(picked));
      artPath = picked;
      say(`Art: ${fileName(picked)}, ${size.width}×${size.height}`);
      if (raw && folder) await build();
    } catch (err) {
      fail("That picture could not be used")(err);
    }
  }

  /** Build the card. `refill` asks for a new fill even when the one there still suits the art. */
  async function build(refill = false) {
    clearTimeout(pending);
    if (!raw || !folder) {
      if (raw && !folder) say("Choose a template to build the card on.");
      return;
    }
    let report: BuildReport;
    try {
      const started = performance.now();
      report = core.buildCard({ scryfall: $state.snapshot(raw), name: face, ...extras, artPath: artPath || null, textShadows: shadows === "template" ? null : shadows === "on" });
      warnings = report.warnings;
      changed();
      await loadImages();
      say(`Built ${side?.name ?? raw.name} in ${Math.round(performance.now() - started)} ms`);
    } catch (err) {
      fail("The card could not be built")(err);
      return;
    }
    // Editing the card's text leaves the art, and so its fill, alone.
    if (report.fill && tier !== "none" && artPath && (refill || !report.fillKept)) await fill(report);
  }

  // One fill is made at a time. If the art has moved again before it is done, one more is
  // made afterwards for where the art is now; requests in between are dropped.
  let fillRunning = false;
  let fillWanted: BuildReport | null = null;

  async function fill(report: BuildReport) {
    if (!report.fill) return;
    if (fillRunning) {
      fillWanted = report;
      return;
    }
    fillRunning = true;
    filling = `Filling around the art…`;
    try {
      const made = await native.fill(artPath, report.fill, tier, seed, prompt);
      if (made.ticket !== null) {
        // Still right if the card was rebuilt meanwhile with its art in the same place; turned away otherwise.
        const added = core.addFill(report.build, await native.fillImage(made.ticket), made.provider, { seed, workWidth: made.workWidth, ...(prompt ? { prompt } : {}) });
        if (added) {
          changed();
          await loadImages();
          const fell = (tier === "sd" || tier === "lama") && made.notices.length ? ` (${made.notices.join("; ")})` : "";
          say(`Filled around the art with ${TIER_NAMES[made.provider === "sdcpp" ? "sd" : made.provider] ?? made.provider} in ${(made.ms / 1000).toFixed(1)} s${fell}`);
        }
      }
    } catch (err) {
      fail("The fill could not be made")(err);
    } finally {
      fillRunning = false;
      filling = "";
    }
    const next = fillWanted;
    fillWanted = null;
    if (next) await fill(next);
  }

  /** A different way of filling was chosen: make the fill again that way, or take it away. */
  function tierChanged() {
    if (tier !== "none") return build(true);
    try {
      core.execute({ type: "removeLayer", id: "card-fill" }, "Remove the fill");
      changed();
    } catch {
      // There was no fill to remove.
    }
  }

  function variation() {
    seed += 1;
    build(true);
  }
</script>

{#snippet symbols()}
  <div class="palette">
    {#each offered as group}
      <div class="row">
        {#each group.symbols as symbol}
          <button class="sym" style:background={symbolColor(symbol)} onclick={() => insertSymbol(symbol)} title={symbol}>{symbol.slice(1, -1)}</button>
        {/each}
      </div>
    {:else}
      <span class="path">No symbol starts with that.</span>
    {/each}
  </div>
{/snippet}

<aside class="card" bind:this={panel}>
  <h2>File</h2>
  <div class="row">
    <button onclick={() => openDocument()} disabled={busy} title="Open a saved card (.cardsmith)">Open…</button>
    <button onclick={saveDocument} disabled={busy} title="Save the card with its layers, to carry on with later">Save…</button>
    <span class="path" title={filePath}>{filePath ? fileName(filePath) : ""}</span>
  </div>
  {#if filePath && !raw && !folder}
    <p class="note">This file opened as plain layers: move, restyle, paint and export as before. Its card form is not available, because its template is not in the templates folder or it was not built as a card here.</p>
  {/if}

  <h2>Folders</h2>
  <div class="row">
    <button onclick={() => choose("templatesDir")} title={settings.templatesDir || "The folder your imported templates are in"}>Templates…</button>
    <span class="path">{settings.templatesDir ? `${templates.length} in ${fileName(settings.templatesDir)}` : "not chosen"}</span>
  </div>
  <div class="row">
    <button onclick={() => choose("fontsDir")} title={settings.fontsDir || "The folder your card fonts are in"}>Fonts…</button>
    <span class="path">{settings.fontsDir ? `${fontCount} loaded from ${fileName(settings.fontsDir)}` : "not chosen"}</span>
  </div>

  <h2>Template</h2>
  <select bind:value={folder} onchange={openTemplate} disabled={busy || !templates.length}>
    <option value="" disabled>{templates.length ? "Choose a template" : "No templates yet"}</option>
    {#each templates as t}
      <option value={t.folder} disabled={!t.buildable}>{t.name}{t.buildable ? "" : " (no card layers found)"}</option>
    {/each}
  </select>

  <h2>Card</h2>
  <div class="row">
    <input class="grow" list="card-names" placeholder="Card name" bind:value={search} oninput={typed} onkeydown={(e) => e.key === "Enter" && find()} />
    <datalist id="card-names">{#each names as name}<option value={name}></option>{/each}</datalist>
    <button onclick={() => find()} disabled={!search.trim()}>Find</button>
  </div>
  {#if prints.length > 1}
    <select bind:value={printing} onchange={pickPrinting} title="Which printing">
      {#each prints as p}
        <option value="{p.set}/{p.number}">{p.setName} · {p.set.toUpperCase()} {p.number} · {p.year} · {p.artist}</option>
      {/each}
    </select>
  {/if}
  {#if prints.some((p) => p.thumbnail)}
    <div class="prints" title="Printings, newest first">
      {#each prints as p}
        {#if p.thumbnail}
          <button class:chosen={printing === `${p.set}/${p.number}`} onclick={() => ((printing = `${p.set}/${p.number}`), pickPrinting())} title="{p.setName} · {p.set.toUpperCase()} {p.number} · {p.year} · {p.artist}">
            <img src={p.thumbnail} alt="{p.setName} {p.number}" loading="lazy" />
          </button>
        {/if}
      {/each}
    </div>
  {/if}
  <button class="quiet" onclick={blank}>…or write a card from nothing</button>

  {#if raw && side}
    <label>Name <input data-field="name" value={side.name ?? ""} oninput={(e) => edit("name", e.currentTarget.value)} /></label>
    <label title={"Type { for the symbols"}
      >Mana cost
      <input data-field="mana_cost" value={side.mana_cost ?? ""} placeholder={"{1}{G}"} oninput={(e) => (edit("mana_cost", e.currentTarget.value), symbolsTyped("mana_cost", e.currentTarget))} />
      <button class="chip" onclick={(e) => togglePalette("mana_cost", e.currentTarget.previousElementSibling as HTMLInputElement)} title="Symbols">{"{ }"}</button>
    </label>
    {#if palette?.field === "mana_cost"}{@render symbols()}{/if}
    <label>Type <input data-field="type_line" value={side.type_line ?? ""} oninput={(e) => edit("type_line", e.currentTarget.value)} onchange={setChanged} /></label>
    <label class="tall" title={"Type { for the symbols"}
      ><span class="row">Rules <button class="chip" onclick={(e) => togglePalette("oracle_text", e.currentTarget.closest("label")!.querySelector("textarea"))} title="Symbols">{"{ }"}</button></span>
      <textarea rows="5" data-field="oracle_text" value={side.oracle_text ?? ""} oninput={(e) => (edit("oracle_text", e.currentTarget.value), symbolsTyped("oracle_text", e.currentTarget))}></textarea>
    </label>
    {#if palette?.field === "oracle_text"}{@render symbols()}{/if}
    {#if /\{[^{}\s]+\}/.test(side.oracle_text ?? "")}
      <div class="chips" title="The rules text with its symbols">
        {#each String(side.oracle_text).split(/(\{[^{}\s]+\})/) as part}
          {#if /^\{[^{}\s]+\}$/.test(part)}<span class="sym" style:background={symbolColor(part)}>{part.slice(1, -1)}</span>{:else}{part}{/if}
        {/each}
      </div>
    {/if}
    <label class="tall">Flavor <textarea rows="3" value={side.flavor_text ?? ""} oninput={(e) => edit("flavor_text", e.currentTarget.value)}></textarea></label>
    <div class="row">
      <label>Power <input class="short" data-field="power" value={side.power ?? ""} oninput={(e) => edit("power", e.currentTarget.value)} /></label>
      <label>Toughness <input class="short" value={side.toughness ?? ""} oninput={(e) => edit("toughness", e.currentTarget.value)} /></label>
    </div>
    <label>Artist <input value={side.artist ?? raw.artist ?? ""} oninput={(e) => edit("artist", e.currentTarget.value)} /></label>
    <label title="Type part of a set's name"
      >Set
      <input list="set-names" placeholder={currentSet?.name ?? "Set name"} bind:value={setName} onchange={setPicked} />
      <datalist id="set-names">{#each setChoices as s}<option value={s.name}>{s.code.toUpperCase()} · {s.released.slice(0, 4)}</option>{/each}</datalist>
    </label>
    <div class="row">
      <label>Code <input class="short" value={raw.set ?? ""} oninput={(e) => edit("set", e.currentTarget.value.toLowerCase(), "card")} onchange={setChanged} /></label>
      <label>No. <input class="short" data-field="collector_number" value={raw.collector_number ?? ""} oninput={(e) => edit("collector_number", e.currentTarget.value, "card")} /></label>
      <label
        >Rarity
        <select value={raw.rarity ?? "common"} onchange={(e) => (edit("rarity", e.currentTarget.value, "card"), setChanged())}>
          {#each RARITIES as r}<option value={r}>{r}</option>{/each}
        </select>
      </label>
    </div>
    <div class="row">
      {#if extras.symbolSvg}
        <img class="symbol" alt="The set symbol" src={"data:image/svg+xml;utf8," + encodeURIComponent(extras.symbolSvg)} />
      {/if}
      <span class="path">{ownSymbol ? "Your own symbol" : currentSet ? `${currentSet.name}, ${currentSet.released.slice(0, 4)}` : extras.symbolSvg ? "Set symbol" : "No set symbol"}</span>
      <label class="file quiet">Use an SVG… <input type="file" accept=".svg,image/svg+xml" onchange={ownSymbolChosen} /></label>
      {#if ownSymbol}<button class="quiet" onclick={setsSymbolAgain}>Use the set's</button>{/if}
    </div>
  {/if}

  {#if raw}
    <label title="The normal frames leave text shadows off and the borderless one turns them on, as Proxyshop does. Choose here to have them your way.">
      Text shadows
      <select bind:value={shadows} onchange={() => build()}>
        <option value="template">As the template has them</option>
        <option value="on">On</option>
        <option value="off">Off</option>
      </select>
    </label>
  {/if}

  <h2>Art</h2>
  <div class="row">
    <button onclick={chooseArt} disabled={!folder}>Choose art…</button>
    <span class="path" title={artPath}>{artPath ? fileName(artPath) : "none"}</span>
  </div>
  <label
    >Fill around it
    <select bind:value={tier} onchange={tierChanged}>
      <option value="auto">Automatic</option>
      {#each tiers as t}
        <option value={t.id} title={t.reason ?? ""}>{TIER_NAMES[t.id]}{t.ok ? "" : " (not installed)"}</option>
      {/each}
      <option value="none">No fill</option>
    </select>
  </label>
  {#if tier === "sd"}
    <label class="tall">Prompt <input placeholder="what should be there (optional)" bind:value={prompt} onchange={() => build(true)} /></label>
  {/if}
  <div class="row">
    <button onclick={variation} disabled={!raw || !artPath || tier === "none" || !!filling} title="Make the fill again, differently">Another variation</button>
    <span class="path">{filling}</span>
  </div>

  {#each warnings as warning}<p class="warning">{warning}</p>{/each}
</aside>

<style>
  .card {
    grid-area: card;
    overflow: auto;
    background: #26282c;
    border-right: 1px solid #111;
    padding: 8px 10px;
    font-size: 13px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  h2 { font-size: 12px; text-transform: uppercase; letter-spacing: 0.08em; color: #9a9ca3; margin: 8px 0 0; }
  .row { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
  select { appearance: none; -webkit-appearance: none; max-width: 100%; text-overflow: ellipsis; }
  .grow { flex: 1; min-width: 0; }
  .path { color: #9a9ca3; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  label { display: flex; align-items: center; gap: 6px; color: #b5b7bd; }
  label.tall { flex-direction: column; align-items: stretch; gap: 2px; }
  label > input, label > textarea, label > select { flex: 1; min-width: 0; }
  input, textarea, select {
    background: #1e1f22;
    color: #e6e6e6;
    border: 1px solid #50545c;
    border-radius: 4px;
    padding: 4px 6px;
    font: inherit;
  }
  textarea { resize: vertical; }
  .short { width: 3.2em; flex: none; }
  button { background: #3a3d43; color: inherit; border: 1px solid #50545c; border-radius: 4px; padding: 4px 10px; font: inherit; white-space: nowrap; }
  button:disabled { opacity: 0.4; }
  .quiet { background: none; border: none; color: #8ea1e1; text-align: left; padding: 0; }
  .warning { color: #f0b232; margin: 0; }
  .note { color: #9a9ca3; margin: 0; font-size: 12px; }
  .prints { display: flex; gap: 4px; overflow-x: auto; padding-bottom: 4px; }
  .prints button { padding: 0; border: 2px solid transparent; border-radius: 5px; background: none; flex: none; }
  .prints button.chosen { border-color: #5865f2; }
  .prints img { display: block; height: 96px; border-radius: 4px; }
  .chips { white-space: pre-wrap; color: #c8c9cd; font-size: 12px; line-height: 1.7; }
  .chips .sym { display: inline-flex; align-items: center; justify-content: center; vertical-align: middle; }
  .chip { padding: 0 6px; font-size: 11px; color: #b5b7bd; }
  .palette { display: flex; flex-direction: column; gap: 4px; padding: 6px; background: #1e1f22; border: 1px solid #50545c; border-radius: 4px; }
  .palette .row { gap: 3px; }
  .sym { min-width: 24px; height: 24px; padding: 0 4px; border-radius: 12px; border: 1px solid #111; color: #111; font-size: 11px; font-weight: 600; }
  .symbol { width: 26px; height: 26px; object-fit: contain; background: #d8d8d8; border-radius: 4px; padding: 2px; }
  label.file { cursor: pointer; }
  label.file input { display: none; }
</style>
