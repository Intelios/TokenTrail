<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type IngestStats, type ModelAlias, type SourceStatus } from '$lib/api';
  import { normalizeModelName, sourceColor } from '$lib/format';

  let sources = $state<SourceStatus[]>([]);
  let syncing = $state(false);
  let stats = $state<IngestStats[] | null>(null);
  let exportPath = $state('');
  let exporting = $state('');
  let error = $state('');

  // model merges
  let modelNames = $state<string[]>([]);
  let aliases = $state<ModelAlias[]>([]);
  let mergeFilter = $state('');
  let selected = $state<string[]>([]);
  let canonical = $state('');

  // hidden models
  let hidden = $state<string[]>([]);
  let hideFilter = $state('');
  let hideSelected = $state<string[]>([]);

  async function load() {
    try {
      sources = await api.sourceStatus();
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  async function loadMerges() {
    try {
      const [byModel, aliasRows, hiddenRows] = await Promise.all([
        api.byModel(3650),
        api.modelAliases(),
        api.hiddenModels(),
      ]);
      modelNames = byModel.map((r) => r.model);
      aliases = aliasRows;
      hidden = hiddenRows;
      selected = selected.filter((n) => modelNames.includes(n));
      hideSelected = hideSelected.filter((n) => modelNames.includes(n));
      if (!selected.includes(canonical)) canonical = selected[0] ?? '';
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  onMount(() => {
    load();
    loadMerges();
    const h = () => {
      load();
      loadMerges();
    };
    window.addEventListener('tt-sync', h);
    return () => window.removeEventListener('tt-sync', h);
  });

  async function sync() {
    syncing = true;
    try {
      stats = await api.syncNow();
      window.dispatchEvent(new CustomEvent('tt-sync'));
    } catch (e) {
      error = String(e);
    } finally {
      syncing = false;
    }
  }

  async function doExport(format: 'csv' | 'json') {
    exporting = format;
    try {
      exportPath = await api.exportData(format);
      error = '';
    } catch (e) {
      error = String(e);
    } finally {
      exporting = '';
    }
  }

  // Names that normalize to the same key are likely the same model spelled
  // differently; the first in list order is the highest-token variant.
  const suggestions = $derived.by(() => {
    const groups = new Map<string, string[]>();
    for (const name of modelNames) {
      const key = normalizeModelName(name);
      if (!key) continue;
      const list = groups.get(key);
      if (list) list.push(name);
      else groups.set(key, [name]);
    }
    return [...groups.values()]
      .filter((list) => new Set(list).size >= 2)
      .map((list) => ({ names: [...new Set(list)], canonical: list[0] }));
  });

  const filteredNames = $derived.by(() => {
    const q = mergeFilter.trim().toLowerCase();
    return q ? modelNames.filter((n) => n.toLowerCase().includes(q)) : modelNames;
  });

  const hideFiltered = $derived.by(() => {
    const q = hideFilter.trim().toLowerCase();
    return q ? modelNames.filter((n) => n.toLowerCase().includes(q)) : modelNames;
  });

  const aliasGroups = $derived.by(() => {
    const groups = new Map<string, string[]>();
    for (const a of aliases) {
      const list = groups.get(a.canonical);
      if (list) list.push(a.alias);
      else groups.set(a.canonical, [a.alias]);
    }
    return [...groups.entries()].map(([name, names]) => ({ canonical: name, aliases: names }));
  });

  function toggleSelect(name: string) {
    selected = selected.includes(name)
      ? selected.filter((n) => n !== name)
      : [...selected, name];
    if (!selected.includes(canonical)) canonical = selected[0] ?? '';
  }

  async function doMerge(names: string[], target: string) {
    try {
      await api.mergeModels(names, target);
      window.dispatchEvent(new CustomEvent('tt-sync'));
      selected = [];
      canonical = '';
      mergeFilter = '';
      await loadMerges();
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  async function doUnmerge(target: string) {
    try {
      await api.unmergeModels(target);
      window.dispatchEvent(new CustomEvent('tt-sync'));
      await loadMerges();
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  async function removeAlias(alias: string) {
    try {
      await api.removeModelAlias(alias);
      window.dispatchEvent(new CustomEvent('tt-sync'));
      await loadMerges();
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  function toggleHide(name: string) {
    hideSelected = hideSelected.includes(name)
      ? hideSelected.filter((n) => n !== name)
      : [...hideSelected, name];
  }

  async function doHide(names: string[]) {
    try {
      await api.hideModels(names);
      window.dispatchEvent(new CustomEvent('tt-sync'));
      hideSelected = [];
      hideFilter = '';
      await loadMerges();
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  async function doUnhide(name: string) {
    try {
      await api.unhideModel(name);
      window.dispatchEvent(new CustomEvent('tt-sync'));
      await loadMerges();
      error = '';
    } catch (e) {
      error = String(e);
    }
  }
</script>

<h1>Settings</h1>
<p class="sub">Sources, sync, and data export</p>

{#if error}
  <p class="error">{error}</p>
{/if}

<div class="panel">
  <h2>Sources</h2>
  <p class="note" style="margin-top:0">
    TokenTrail only ever <b>reads</b> these locations. Everything it learns is copied into its own
    database, so history survives even if a harness wipes or rotates its files.
  </p>
  {#each sources as s}
    <div class="src">
      <span class="dot" style="background:{sourceColor(s.source)}"></span>
      <div class="meta">
        <div>{s.display}</div>
        <div class="p">{s.path}</div>
      </div>
      {#if s.found}
        <span class="tag ok">found</span>
      {:else}
        <span class="tag miss">not installed</span>
      {/if}
    </div>
  {/each}
</div>

<div class="panel">
  <h2>Sync</h2>
  <p class="note" style="margin-top:0">
    A background sync runs every 30 seconds while the app is open. Use this to force one now:
  </p>
  <button class="primary" onclick={sync} disabled={syncing}>{syncing ? 'Syncing…' : 'Sync now'}</button>
  {#if stats}
    <div style="margin-top:12px" class="note">
      {#each stats as s}
        <div>
          {s.source}: {s.processed} processed{#if s.error} <span class="error">— {s.error}</span>{/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if suggestions.length}
  <div class="panel">
    <h2>Suggested merges</h2>
    <p class="note" style="margin-top:0">
      These look like the same model recorded under different names. Merge them and they'll show up
      as one entry everywhere.
    </p>
    {#each suggestions as s}
      <div class="src">
        <div class="meta">
          <div>{s.names.join(' · ')}</div>
          <div class="p">Show all as “{s.canonical}”</div>
        </div>
        <button onclick={() => doMerge(s.names, s.canonical)}>Merge</button>
      </div>
    {/each}
  </div>
{/if}

<div class="panel">
  <h2>Merge models</h2>
  <p class="note" style="margin-top:0">
    Pick two or more names that are the same model, then choose which one they should all display as.
  </p>
  <input type="text" placeholder="Filter models…" bind:value={mergeFilter} />
  <div class="mergelist">
    {#each filteredNames as name}
      <label class="merge-row">
        <input type="checkbox" checked={selected.includes(name)} onchange={() => toggleSelect(name)} />
        <span>{name}</span>
      </label>
    {/each}
    {#if !filteredNames.length}
      <div class="loading" style="padding:16px">no models match</div>
    {/if}
  </div>
  {#if selected.length >= 2}
    <div class="row" style="margin-top:10px">
      <span class="note">Display as:</span>
      {#each selected as name}
        <label class="canon-row">
          <input type="radio" name="canonical" value={name} checked={canonical === name} onchange={() => (canonical = name)} />
          <span>{name}</span>
        </label>
      {/each}
    </div>
  {/if}
  <div class="row" style="margin-top:10px">
    <button class="primary" disabled={selected.length < 2} onclick={() => doMerge(selected, canonical)}>
      Merge {selected.length >= 2 ? `${selected.length} models` : '…'}
    </button>
    {#if selected.length >= 2}
      <span class="note">into “{canonical}”</span>
    {/if}
  </div>
</div>

{#if aliasGroups.length}
  <div class="panel">
    <h2>Current merges</h2>
    <p class="note" style="margin-top:0">
      Model names that are currently shown under a single entry. Removing one is instant — your data
      is never rewritten.
    </p>
    {#each aliasGroups as g}
      <div class="src">
        <div class="meta">
          <div>{g.canonical}</div>
          <div class="p">
            {#each g.aliases as alias}
              <span class="tag">
                {alias}
                <button class="x" aria-label="Stop merging {alias}" onclick={() => removeAlias(alias)}>×</button>
              </span>
            {/each}
          </div>
        </div>
        <button onclick={() => doUnmerge(g.canonical)}>Unmerge</button>
      </div>
    {/each}
  </div>
{/if}

<div class="panel">
  <h2>Hidden models</h2>
  <p class="note" style="margin-top:0">
    Some harnesses record background jobs as pseudo-models — Codex's
    <b>codex-auto-review</b> (auto titling / security review) — rather than models you
    actually chose. Hiding one removes its tokens and cost from <b>every</b> stat in the
    app. Nothing is deleted; unhide any time.
  </p>
  {#if hidden.length}
    {#each hidden as name}
      <div class="src">
        <div class="meta">
          <div>{name}</div>
          <div class="p">hidden from all stats</div>
        </div>
        <button onclick={() => doUnhide(name)}>Unhide</button>
      </div>
    {/each}
  {/if}
  <input type="text" placeholder="Filter models…" bind:value={hideFilter} />
  <div class="mergelist">
    {#each hideFiltered as name}
      <label class="merge-row">
        <input type="checkbox" checked={hideSelected.includes(name)} onchange={() => toggleHide(name)} />
        <span>{name}</span>
      </label>
    {/each}
    {#if !hideFiltered.length}
      <div class="loading" style="padding:16px">no models match</div>
    {/if}
  </div>
  <div class="row" style="margin-top:10px">
    <button class="primary" disabled={!hideSelected.length} onclick={() => doHide(hideSelected)}>
      Hide {hideSelected.length ? `${hideSelected.length} models` : '…'}
    </button>
  </div>
</div>

<div class="panel">
  <h2>Export</h2>
  <p class="note" style="margin-top:0">
    Dump every stored usage event. Files land in the app's data folder.
  </p>
  <div class="row">
    <button onclick={() => doExport('csv')} disabled={exporting !== ''}>
      {exporting === 'csv' ? 'Exporting…' : 'Export CSV'}
    </button>
    <button onclick={() => doExport('json')} disabled={exporting !== ''}>
      {exporting === 'json' ? 'Exporting…' : 'Export JSON'}
    </button>
  </div>
  {#if exportPath}
    <p class="note" style="margin-bottom:0">Saved to <code>{exportPath}</code></p>
  {/if}
</div>

<div class="panel">
  <h2>About the numbers</h2>
  <p class="note" style="margin-top:0">
    Token totals are input + output + cached tokens as reported by each harness. Cost figures are
    <b>API-equivalent estimates</b> from a bundled price snapshot — they approximate what the same
    usage would cost at list API prices, not your actual subscription spend. Days are counted in UTC.
  </p>
</div>
