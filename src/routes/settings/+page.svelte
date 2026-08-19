<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type IngestStats, type ModelAlias, type SourceStatus } from '$lib/api';
  import { normalizeModelName, sourceColor } from '$lib/format';

  type Tab = 'sources' | 'merges' | 'hidden' | 'export';
  let activeTab = $state<Tab>('sources');

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

<div class="sframe">
  <!-- Header with Category Navigation Pills -->
  <div class="thd">
    <div class="up">
      <h1>Settings</h1>
      <div class="sub">Sources, model alias merging, hidden models & data export</div>
    </div>
    <div class="pillsrow up">
      <div class="pills">
        <button class="pill" class:on={activeTab === 'sources'} onclick={() => (activeTab = 'sources')}>
          Sources & Sync
        </button>
        <button class="pill" class:on={activeTab === 'merges'} onclick={() => (activeTab = 'merges')}>
          Model Merges {#if suggestions.length}<span class="badge">{suggestions.length}</span>{/if}
        </button>
        <button class="pill" class:on={activeTab === 'hidden'} onclick={() => (activeTab = 'hidden')}>
          Hidden Models {#if hidden.length}<span class="badge">{hidden.length}</span>{/if}
        </button>
        <button class="pill" class:on={activeTab === 'export'} onclick={() => (activeTab = 'export')}>
          Export & Info
        </button>
      </div>
    </div>
  </div>

  {#if error}
    <div class="errpad"><p class="error">{error}</p></div>
  {/if}

  <!-- TAB 1: Sources & Sync -->
  {#if activeTab === 'sources'}
    <div class="grid2 up">
      <div class="col">
        <div class="col-hd">
          <h2>Harness Sources</h2>
          <span class="cnt">{sources.filter((s) => s.found).length} of {sources.length} installed</span>
        </div>
        <p class="note">
          TokenTrail only ever <b>reads</b> these locations. Everything it learns is copied into its own database, so history survives even if a harness wipes or rotates its files.
        </p>
        <div class="srclist">
          {#each sources as s}
            <div class="src-item">
              <span class="dot" style="background:{sourceColor(s.source)}"></span>
              <div class="meta">
                <div class="nm">{s.display}</div>
                <div class="p">{s.path}</div>
              </div>
              {#if s.found}
                <span class="tag ok">installed</span>
              {:else}
                <span class="tag miss">not found</span>
              {/if}
            </div>
          {/each}
        </div>
      </div>

      <div class="col">
        <div class="col-hd">
          <h2>Sync Engine</h2>
        </div>
        <p class="note">
          A background sync runs automatically every 30 seconds while the app is open.
        </p>
        <div class="action-card">
          <button class="primary sync-btn" onclick={sync} disabled={syncing}>
            {syncing ? 'Syncing…' : 'Force Sync Now'}
          </button>
          {#if stats}
            <div class="sync-results">
              <div class="res-title">Last Sync Stats:</div>
              {#each stats as s}
                <div class="res-row">
                  <span class="stag"><i style="background:{sourceColor(s.source)}"></i>{s.source}</span>
                  <span class="res-cnt">{s.processed} events</span>
                  {#if s.error}
                    <span class="error">{s.error}</span>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
        </div>

        <div class="col-hd" style="margin-top:28px">
          <h2>About The Numbers</h2>
        </div>
        <p class="note">
          Token totals are input + output + cached tokens as reported by each harness. Cost figures are
          <b>API-equivalent estimates</b> from a bundled price snapshot — they approximate what the same
          usage would cost at list API prices, not your actual subscription spend. Active days and streaks are computed in UTC.
        </p>
      </div>
    </div>

  <!-- TAB 2: Model Merges -->
  {:else if activeTab === 'merges'}
    <div class="merges-view up">
      {#if suggestions.length}
        <div class="suggest-band">
          <div class="sug-hd">
            <h3>Suggested Merges ({suggestions.length})</h3>
            <span class="sug-sub">Auto-detected variations that look like the same model</span>
          </div>
          <div class="sug-grid">
            {#each suggestions as s}
              <div class="sug-card">
                <div class="sug-names">{s.names.join(' · ')}</div>
                <div class="sug-act">
                  <span class="sug-target">Canonical: <b>{s.canonical}</b></span>
                  <button class="sug-btn" onclick={() => doMerge(s.names, s.canonical)}>Merge</button>
                </div>
              </div>
            {/each}
          </div>
        </div>
      {/if}

      <div class="grid2">
        <div class="col">
          <div class="col-hd">
            <h2>Merge Models</h2>
            <span class="cnt">{selected.length} selected</span>
          </div>
          <p class="note">
            Pick 2 or more names that represent the same model, then choose which canonical name to display.
          </p>
          <div class="search-box">
            <input type="text" placeholder="Search models…" bind:value={mergeFilter} />
          </div>
          <div class="checklist">
            {#each filteredNames as name}
              <label class="check-row" class:checked={selected.includes(name)}>
                <input type="checkbox" checked={selected.includes(name)} onchange={() => toggleSelect(name)} />
                <span class="name">{name}</span>
              </label>
            {/each}
            {#if !filteredNames.length}
              <div class="empty-state">No models match "{mergeFilter}"</div>
            {/if}
          </div>

          {#if selected.length >= 2}
            <div class="canon-box">
              <div class="canon-title">Display all as:</div>
              <div class="canon-radios">
                {#each selected as name}
                  <label class="radio-row">
                    <input type="radio" name="canonical" value={name} checked={canonical === name} onchange={() => (canonical = name)} />
                    <span>{name}</span>
                  </label>
                {/each}
              </div>
            </div>
          {/if}

          <div class="btn-row">
            <button class="primary" disabled={selected.length < 2} onclick={() => doMerge(selected, canonical)}>
              Merge {selected.length >= 2 ? `${selected.length} models` : '…'}
            </button>
            {#if selected.length >= 2}
              <span class="note-inline">into “<b>{canonical}</b>”</span>
            {/if}
          </div>
        </div>

        <div class="col">
          <div class="col-hd">
            <h2>Active Merges ({aliasGroups.length})</h2>
          </div>
          <p class="note">
            Models currently folded into canonical display names. Unmerging is instant — raw usage events are never rewritten.
          </p>
          <div class="active-merges-list">
            {#each aliasGroups as g}
              <div class="merge-group-card">
                <div class="grp-header">
                  <span class="grp-canon">{g.canonical}</span>
                  <button class="unmerge-btn" onclick={() => doUnmerge(g.canonical)}>Unmerge</button>
                </div>
                <div class="grp-aliases">
                  {#each g.aliases as alias}
                    <span class="alias-chip">
                      {alias}
                      <button class="x-btn" aria-label="Stop merging {alias}" onclick={() => removeAlias(alias)}>×</button>
                    </span>
                  {/each}
                </div>
              </div>
            {/each}
            {#if !aliasGroups.length}
              <div class="empty-state">No model aliases merged yet</div>
            {/if}
          </div>
        </div>
      </div>
    </div>

  <!-- TAB 3: Hidden Models -->
  {:else if activeTab === 'hidden'}
    <div class="grid2 up">
      <div class="col">
        <div class="col-hd">
          <h2>Hide Models</h2>
          <span class="cnt">{hideSelected.length} selected</span>
        </div>
        <p class="note">
          Some harnesses record background jobs as pseudo-models (e.g. Codex's <code>codex-auto-review</code>). Hiding removes them from <b>every</b> metric and chart. Raw data is preserved and can be unhidden any time.
        </p>
        <div class="search-box">
          <input type="text" placeholder="Search models…" bind:value={hideFilter} />
        </div>
        <div class="checklist">
          {#each hideFiltered as name}
            <label class="check-row" class:checked={hideSelected.includes(name)}>
              <input type="checkbox" checked={hideSelected.includes(name)} onchange={() => toggleHide(name)} />
              <span class="name">{name}</span>
            </label>
          {/each}
          {#if !hideFiltered.length}
            <div class="empty-state">No models match "{hideFilter}"</div>
          {/if}
        </div>
        <div class="btn-row">
          <button class="primary" disabled={!hideSelected.length} onclick={() => doHide(hideSelected)}>
            Hide {hideSelected.length ? `${hideSelected.length} models` : '…'}
          </button>
        </div>
      </div>

      <div class="col">
        <div class="col-hd">
          <h2>Currently Hidden ({hidden.length})</h2>
        </div>
        <p class="note">
          These models are currently excluded from all aggregate stats across TokenTrail.
        </p>
        <div class="hidden-list">
          {#each hidden as name}
            <div class="hidden-item">
              <div class="meta">
                <div class="nm">{name}</div>
                <div class="p">Hidden from all dashboard metrics</div>
              </div>
              <button class="unhide-btn" onclick={() => doUnhide(name)}>Unhide</button>
            </div>
          {/each}
          {#if !hidden.length}
            <div class="empty-state">No models currently hidden</div>
          {/if}
        </div>
      </div>
    </div>

  <!-- TAB 4: Export & Info -->
  {:else if activeTab === 'export'}
    <div class="grid2 up">
      <div class="col">
        <div class="col-hd">
          <h2>Export Raw Data</h2>
        </div>
        <p class="note">
          Dump every stored usage event across all harnesses. Export files are written to the app data folder.
        </p>
        <div class="export-actions">
          <button class="export-btn" onclick={() => doExport('csv')} disabled={exporting !== ''}>
            {exporting === 'csv' ? 'Exporting CSV…' : 'Export as CSV'}
          </button>
          <button class="export-btn" onclick={() => doExport('json')} disabled={exporting !== ''}>
            {exporting === 'json' ? 'Exporting JSON…' : 'Export as JSON'}
          </button>
        </div>
        {#if exportPath}
          <div class="export-success">
            <span class="succ-badge">SAVED</span>
            <code>{exportPath}</code>
          </div>
        {/if}
      </div>

      <div class="col">
        <div class="col-hd">
          <h2>Storage & Architecture</h2>
        </div>
        <p class="note">
          TokenTrail stores all normalized metrics locally in an embedded SQLite database (<code>usage.db</code>). External harness databases are always opened <b>read-only</b>.
        </p>
        <div class="arch-card">
          <div class="arch-row">
            <span class="k">Storage Mode:</span>
            <span class="v">100% Local & Offline</span>
          </div>
          <div class="arch-row">
            <span class="k">Idempotence:</span>
            <span class="v">Stable <code>(source, source_event_id)</code> upserts</span>
          </div>
          <div class="arch-row">
            <span class="k">Pricing Table:</span>
            <span class="v">Bundled list prices (auto-repriced on table changes)</span>
          </div>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .sframe {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .thd {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 16px;
    padding: 15px clamp(22px, 1.8vw, 40px) 14px;
    border-bottom: 2px solid var(--ink);
    flex-wrap: wrap;
  }
  .thd .sub { font: 400 11px/1 var(--font-mono); opacity: 0.55; margin-top: 6px; letter-spacing: 0.6px; margin-bottom: 0; }
  .pillsrow { display: flex; gap: 12px; flex-wrap: wrap; }

  .badge {
    background: var(--org);
    color: #fff;
    padding: 1px 5px;
    font-size: 9px;
    margin-left: 4px;
  }

  .errpad { padding: 20px 22px; }

  .grid2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    flex: 1;
    min-height: 0;
  }

  .col {
    padding: 20px clamp(22px, 1.8vw, 40px);
    border-right: 2px solid var(--ink);
    display: flex;
    flex-direction: column;
    overflow-y: auto;
  }
  .col:last-child {
    border-right: none;
  }

  .col-hd {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 8px;
  }
  .col-hd h2 {
    font: 600 11px/1 var(--font-ui);
    letter-spacing: 1.6px;
    text-transform: uppercase;
    margin: 0;
  }
  .col-hd .cnt {
    font: 500 10px/1 var(--font-mono);
    opacity: 0.55;
    letter-spacing: 0.6px;
    text-transform: uppercase;
  }

  .note {
    color: var(--dim);
    font-size: 12px;
    line-height: 1.55;
    margin-top: 0;
    margin-bottom: 14px;
  }
  .note b { color: var(--ink); font-weight: 600; }
  .note code { background: rgba(13, 13, 11, 0.06); padding: 2px 4px; font-family: var(--font-mono); font-size: 11px; }

  .srclist {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .src-item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 11px 12px;
    border: 1px solid var(--hair);
    background: var(--bone);
  }
  .src-item .dot {
    width: 8px;
    height: 8px;
    flex: none;
  }
  .src-item .meta { flex: 1; min-width: 0; }
  .src-item .nm { font: 600 12px/1.2 var(--font-ui); color: var(--ink); }
  .src-item .p { font: 400 10px/1.3 var(--font-mono); color: var(--dim); margin-top: 3px; word-break: break-all; }

  .action-card {
    border: 1px solid var(--hair);
    padding: 14px;
    background: var(--bone);
  }
  .sync-btn {
    width: 100%;
    padding: 10px 16px;
    font: 600 11px/1 var(--font-ui);
    letter-spacing: 1px;
    text-transform: uppercase;
  }
  .sync-results {
    margin-top: 12px;
    padding-top: 10px;
    border-top: 1px solid var(--hair);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .res-title {
    font: 600 10px/1 var(--font-mono);
    text-transform: uppercase;
    letter-spacing: 0.6px;
    opacity: 0.6;
    margin-bottom: 2px;
  }
  .res-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font: 400 11px/1 var(--font-mono);
  }
  .res-cnt {
    font-weight: 600;
  }

  /* Merges tab */
  .merges-view {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .suggest-band {
    padding: 14px clamp(22px, 1.8vw, 40px);
    border-bottom: 2px solid var(--ink);
    background: rgba(255, 77, 0, 0.06);
  }
  .sug-hd {
    display: flex;
    align-items: baseline;
    gap: 12px;
    margin-bottom: 10px;
  }
  .sug-hd h3 {
    font: 600 11px/1 var(--font-ui);
    letter-spacing: 1.4px;
    text-transform: uppercase;
    color: var(--org);
    margin: 0;
  }
  .sug-sub {
    font: 400 11px/1 var(--font-mono);
    opacity: 0.6;
  }
  .sug-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 10px;
  }
  .sug-card {
    padding: 10px 12px;
    border: 1.5px solid var(--ink);
    background: var(--bone);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .sug-names {
    font: 600 12px/1.3 var(--font-mono);
    word-break: break-all;
  }
  .sug-act {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .sug-target {
    font: 400 10.5px/1 var(--font-mono);
    opacity: 0.7;
  }
  .sug-btn {
    padding: 4px 10px;
    font: 600 10px/1 var(--font-ui);
    letter-spacing: 0.8px;
    text-transform: uppercase;
    background: var(--org);
    color: #fff;
    border: 1.5px solid var(--ink);
    cursor: pointer;
  }

  .search-box {
    margin-bottom: 10px;
  }
  .search-box input {
    width: 100%;
    padding: 8px 10px;
    font: 400 11px/1 var(--font-mono);
    border: 1.5px solid var(--ink);
    background: var(--bone);
    color: var(--ink);
    border-radius: 0;
  }
  .search-box input:focus {
    outline: none;
    border-color: var(--org);
  }

  .checklist {
    max-height: 220px;
    overflow-y: auto;
    border: 1.5px solid var(--ink);
    background: var(--bone);
    display: flex;
    flex-direction: column;
  }
  .check-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    border-bottom: 1px solid var(--hair);
    cursor: pointer;
    font: 500 11px/1 var(--font-mono);
  }
  .check-row:last-child {
    border-bottom: none;
  }
  .check-row:hover {
    background: var(--hair);
  }
  .check-row.checked {
    background: rgba(255, 77, 0, 0.08);
  }
  .check-row .name {
    word-break: break-all;
  }

  .canon-box {
    margin-top: 12px;
    padding: 10px 12px;
    border: 1.5px solid var(--ink);
    background: rgba(13, 13, 11, 0.03);
  }
  .canon-title {
    font: 600 10px/1 var(--font-ui);
    letter-spacing: 1px;
    text-transform: uppercase;
    opacity: 0.7;
    margin-bottom: 8px;
  }
  .canon-radios {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .radio-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font: 500 11px/1 var(--font-mono);
    cursor: pointer;
  }

  .btn-row {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 14px;
  }
  .btn-row .primary {
    padding: 8px 16px;
    font: 600 10px/1 var(--font-ui);
    letter-spacing: 1px;
    text-transform: uppercase;
  }
  .note-inline {
    font: 400 11px/1 var(--font-mono);
    opacity: 0.75;
  }

  .active-merges-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    overflow-y: auto;
  }
  .merge-group-card {
    padding: 10px 12px;
    border: 1px solid var(--hair);
    background: var(--bone);
  }
  .grp-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 8px;
  }
  .grp-canon {
    font: 600 12px/1.2 var(--font-mono);
    color: var(--ink);
  }
  .unmerge-btn {
    padding: 3px 8px;
    font: 600 9px/1 var(--font-ui);
    letter-spacing: 0.6px;
    text-transform: uppercase;
    background: transparent;
    border: 1px solid var(--ink);
    color: var(--ink);
    cursor: pointer;
  }
  .unmerge-btn:hover {
    background: var(--ink);
    color: #fff;
  }
  .grp-aliases {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .alias-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 6px;
    font: 400 10px/1 var(--font-mono);
    border: 1px solid var(--hair);
    background: rgba(13, 13, 11, 0.04);
  }
  .x-btn {
    border: none;
    background: transparent;
    font-size: 12px;
    line-height: 1;
    color: var(--dim);
    cursor: pointer;
    padding: 0;
  }
  .x-btn:hover {
    color: var(--org);
  }

  /* Hidden Tab */
  .hidden-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .hidden-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 12px;
    border: 1px solid var(--hair);
    background: var(--bone);
  }
  .hidden-item .meta { flex: 1; min-width: 0; }
  .hidden-item .nm { font: 600 12px/1.2 var(--font-mono); color: var(--ink); }
  .hidden-item .p { font: 400 10px/1 var(--font-mono); color: var(--dim); margin-top: 3px; }
  .unhide-btn {
    padding: 4px 10px;
    font: 600 9.5px/1 var(--font-ui);
    letter-spacing: 0.6px;
    text-transform: uppercase;
    background: transparent;
    border: 1px solid var(--ink);
    cursor: pointer;
  }
  .unhide-btn:hover {
    background: var(--ink);
    color: #fff;
  }

  /* Export Tab */
  .export-actions {
    display: flex;
    gap: 12px;
    margin-top: 10px;
  }
  .export-btn {
    padding: 10px 18px;
    font: 600 11px/1 var(--font-ui);
    letter-spacing: 1px;
    text-transform: uppercase;
    background: var(--bone);
    border: 2px solid var(--ink);
    color: var(--ink);
    cursor: pointer;
  }
  .export-btn:hover {
    background: var(--ink);
    color: #fff;
  }
  .export-success {
    margin-top: 14px;
    padding: 10px 12px;
    border: 1px solid var(--ink);
    background: rgba(0, 194, 194, 0.1);
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .succ-badge {
    font: 600 9px/1 var(--font-mono);
    background: var(--cyn);
    color: #0d0d0b;
    padding: 2px 6px;
  }
  .export-success code {
    font: 400 10.5px/1 var(--font-mono);
    word-break: break-all;
  }

  .arch-card {
    border: 1px solid var(--hair);
    padding: 14px;
    background: var(--bone);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .arch-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font: 400 11px/1.3 var(--font-mono);
  }
  .arch-row .k { opacity: 0.6; font-weight: 500; }
  .arch-row .v { font-weight: 600; }

  .empty-state {
    padding: 18px;
    text-align: center;
    font: 400 11px/1 var(--font-mono);
    opacity: 0.45;
  }

  @media (max-width: 900px) {
    .grid2 { grid-template-columns: 1fr; }
    .col { border-right: none; border-bottom: 2px solid var(--ink); }
  }
</style>
