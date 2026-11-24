<script lang="ts">
  import type { BuildReport, Core } from "./lib/core";
  import { native, type Extras, type Printing, type Settings, type TemplateEntry, type TierState } from "./lib/native";

  // The card panel: choose a template, find a card (or write one), give it
  // art, and the card is built on the canvas and rebuilt as its fields change.
  // The fill around the art is made on the native side and arrives when ready.

  let {
    core,
    busy = $bindable(false),
    say,
    changed,
    opened,
  }: {
    core: Core;
    busy?: boolean;
    /** Show a line in the status bar. */
    say: (text: string) => void;
    /** The document changed: copy what was redrawn to the screen and re-read the layers. */
    changed: () => void;
    /** A different document is open: fit it to the window. */
    opened: () => void;
  } = $props();

  let settings = $state<Settings>({ templatesDir: "", fontsDir: "" });
  let templates = $state<TemplateEntry[]>([]);
  let fontCount = $state(0);
  let folder = $state("");
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

  const RARITIES = ["common", "uncommon", "rare", "mythic", "special"];
  const TIER_NAMES: Record<string, string> = { sd: "Stable Diffusion", lama: "LaMa model", patchmatch: "PatchMatch" };
  const fail = (what: string) => (err: unknown) => say(`${what}: ${err instanceof Error ? err.message : err}`);
  const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

  native
    .settings()
    .then(async (saved) => {
      settings = saved;
      tiers = await native.fillTiers();
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
    const want = JSON.parse(text) as { template: string; card: string; art?: string; tier?: string };
    tier = want.tier ?? tier;
    folder = want.template;
    await openTemplate();
    if (want.art) {
      core.setArt(await native.artFile(want.art));
      artPath = want.art;
    }
    search = want.card;
    await find();
  }

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

  /** Fetch the images that layers now showing need. A template's other images are never loaded. */
  async function loadImages() {
    const missing = core.missingAssets();
    if (!missing.length) return;
    const files = await Promise.all(missing.map((m) => native.templateAsset(folder, m.path).catch(() => null)));
    const got = missing.map((m, i) => ({ id: m.id, data: files[i] })).filter((f): f is { id: string; data: Uint8Array<ArrayBuffer> } => f.data !== null);
    got.forEach((f, i) => core.addAsset(f.id, f.data, i === got.length - 1));
    changed();
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
    pending = setTimeout(build, 300);
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
    extras = await native.setSymbol(String(raw.set ?? ""), String(raw.rarity ?? ""), String(side?.type_line ?? "")).catch(() => extras);
    rebuild();
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

  async function build() {
    clearTimeout(pending);
    if (!raw || !folder) {
      if (raw && !folder) say("Choose a template to build the card on.");
      return;
    }
    let report: BuildReport;
    try {
      const started = performance.now();
      report = core.buildCard({ scryfall: $state.snapshot(raw), name: face, ...extras });
      warnings = report.warnings;
      changed();
      await loadImages();
      say(`Built ${side?.name ?? raw.name} in ${Math.round(performance.now() - started)} ms`);
    } catch (err) {
      fail("The card could not be built")(err);
      return;
    }
    if (report.fill && tier !== "none" && artPath) await fill(report);
  }

  async function fill(report: BuildReport) {
    if (!report.fill) return;
    filling = `Filling around the art…`;
    try {
      const made = await native.fill(artPath, report.fill, tier, seed, prompt);
      if (made.ticket === null) return;
      const added = core.addFill(report.build, await native.fillImage(made.ticket), made.provider, { seed, workWidth: made.workWidth, ...(prompt ? { prompt } : {}) });
      if (!added) return; // The card changed while this was being made; its own fill is on the way.
      changed();
      await loadImages();
      const fell = (tier === "sd" || tier === "lama") && made.notices.length ? ` (${made.notices.join("; ")})` : "";
      say(`Filled around the art with ${TIER_NAMES[made.provider === "sdcpp" ? "sd" : made.provider] ?? made.provider} in ${(made.ms / 1000).toFixed(1)} s${fell}`);
    } catch (err) {
      fail("The fill could not be made")(err);
    } finally {
      filling = "";
    }
  }

  function variation() {
    seed += 1;
    build();
  }
</script>

<aside class="card">
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
  <button class="quiet" onclick={blank}>…or write a card from nothing</button>

  {#if raw && side}
    <label>Name <input value={side.name ?? ""} oninput={(e) => edit("name", e.currentTarget.value)} /></label>
    <label title={"Symbols in braces: {2}{G}{G}"}>Mana cost <input value={side.mana_cost ?? ""} placeholder={"{1}{G}"} oninput={(e) => edit("mana_cost", e.currentTarget.value)} /></label>
    <label>Type <input value={side.type_line ?? ""} oninput={(e) => edit("type_line", e.currentTarget.value)} onchange={setChanged} /></label>
    <label class="tall" title={"Symbols in braces: {T}: Add {G}."}>Rules <textarea rows="5" value={side.oracle_text ?? ""} oninput={(e) => edit("oracle_text", e.currentTarget.value)}></textarea></label>
    <label class="tall">Flavor <textarea rows="3" value={side.flavor_text ?? ""} oninput={(e) => edit("flavor_text", e.currentTarget.value)}></textarea></label>
    <div class="row">
      <label>Power <input class="short" value={side.power ?? ""} oninput={(e) => edit("power", e.currentTarget.value)} /></label>
      <label>Toughness <input class="short" value={side.toughness ?? ""} oninput={(e) => edit("toughness", e.currentTarget.value)} /></label>
    </div>
    <label>Artist <input value={side.artist ?? raw.artist ?? ""} oninput={(e) => edit("artist", e.currentTarget.value)} /></label>
    <div class="row">
      <label>Set <input class="short" value={raw.set ?? ""} oninput={(e) => edit("set", e.currentTarget.value.toLowerCase(), "card")} onchange={setChanged} /></label>
      <label>No. <input class="short" value={raw.collector_number ?? ""} oninput={(e) => edit("collector_number", e.currentTarget.value, "card")} /></label>
      <label
        >Rarity
        <select value={raw.rarity ?? "common"} onchange={(e) => (edit("rarity", e.currentTarget.value, "card"), setChanged())}>
          {#each RARITIES as r}<option value={r}>{r}</option>{/each}
        </select>
      </label>
    </div>
  {/if}

  <h2>Art</h2>
  <div class="row">
    <button onclick={chooseArt} disabled={!folder}>Choose art…</button>
    <span class="path" title={artPath}>{artPath ? fileName(artPath) : "none"}</span>
  </div>
  <label
    >Fill around it
    <select bind:value={tier} onchange={() => build()}>
      <option value="auto">Automatic</option>
      {#each tiers as t}
        <option value={t.id} title={t.reason ?? ""}>{TIER_NAMES[t.id]}{t.ok ? "" : " (not installed)"}</option>
      {/each}
      <option value="none">No fill</option>
    </select>
  </label>
  {#if tier === "sd"}
    <label class="tall">Prompt <input placeholder="what should be there (optional)" bind:value={prompt} onchange={() => build()} /></label>
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
</style>
