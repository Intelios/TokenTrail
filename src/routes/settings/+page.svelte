<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type IngestStats, type SourceStatus } from '$lib/api';
  import { sourceColor } from '$lib/format';

  let sources = $state<SourceStatus[]>([]);
  let syncing = $state(false);
  let stats = $state<IngestStats[] | null>(null);
  let exportPath = $state('');
  let exporting = $state('');
  let error = $state('');

  async function load() {
    try {
      sources = await api.sourceStatus();
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  onMount(load);

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
