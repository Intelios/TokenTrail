<script lang="ts">
  /// Usage Wrapped panel: period pills, a live 1080×1350 preview drawn by
  /// $lib/wrapped, and a Save-As export of exactly that canvas. The preview
  /// and the PNG are the same pixels by construction.

  import { onMount } from 'svelte';
  import { save } from '@tauri-apps/plugin-dialog';
  import { api, type WrappedSummary } from '$lib/api';
  import { drawWrapped } from '$lib/wrapped';

  type Period = 'week' | 'month' | 'all';
  const PERIODS: Array<{ key: Period; days: number; label: string }> = [
    { key: 'week', days: 7, label: 'WEEK' },
    { key: 'month', days: 30, label: 'MONTH' },
    { key: 'all', days: 0, label: 'ALL TIME' },
  ];

  let period = $state<Period>('month');
  let summary = $state<WrappedSummary | null>(null);
  let loading = $state(true);
  let exporting = $state(false);
  let savedPath = $state('');
  let error = $state('');
  let canvas = $state<HTMLCanvasElement | null>(null);
  // The card only looks right in Anton/Inter Tight/Plex Mono; the draw is
  // held back until the fontsource faces have actually loaded.
  let fontsReady = $state(false);

  function periodLabel(): string {
    if (period === 'week') return 'LAST 7 DAYS';
    if (period === 'month') return 'LAST 30 DAYS';
    return 'ALL TIME';
  }

  async function load() {
    loading = true;
    try {
      const days = PERIODS.find((p) => p.key === period)?.days ?? 30;
      summary = await api.wrappedSummary(days);
      error = '';
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void period;
    load();
  });

  $effect(() => {
    if (summary && canvas && fontsReady) drawWrapped(canvas, summary, periodLabel());
  });

  onMount(() => {
    Promise.all([
      document.fonts.load('400 64px Anton'),
      document.fonts.load('700 24px "Inter Tight"'),
      document.fonts.load('500 16px "IBM Plex Mono"'),
    ])
      .then(() => (fontsReady = true))
      .catch(() => (fontsReady = true)); // draw anyway rather than never
    const h = () => load();
    window.addEventListener('tt-sync', h);
    return () => window.removeEventListener('tt-sync', h);
  });

  async function exportPng() {
    const c = canvas;
    if (!c || !summary || summary.total_tokens <= 0) return;
    exporting = true;
    savedPath = '';
    try {
      const stamp = new Date().toISOString().slice(0, 10);
      const target = await save({
        defaultPath: `tokentrail-wrapped-${period}-${stamp}.png`,
        filters: [{ name: 'PNG Image', extensions: ['png'] }],
      });
      if (!target) return; // cancelled the dialog — not an error
      const blob = await new Promise<Blob | null>((res) => c.toBlob(res, 'image/png'));
      if (!blob) throw new Error('PNG encode failed');
      savedPath = await api.exportWrappedPng(target, await blobToBase64(blob));
      error = '';
    } catch (e) {
      error = String(e);
    } finally {
      exporting = false;
    }
  }

  function blobToBase64(b: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const r = new FileReader();
      r.onload = () => resolve(String(r.result).split(',')[1] ?? '');
      r.onerror = () => reject(r.error ?? new Error('read failed'));
      r.readAsDataURL(b);
    });
  }

  const hasData = $derived(summary != null && summary.total_tokens > 0);
</script>

<div class="wrapped">
  <div class="wrow">
    <div class="wpills">
      {#each PERIODS as p (p.key)}
        <button class="wpill" class:on={period === p.key} onclick={() => (period = p.key)}>
          {p.label}
        </button>
      {/each}
    </div>
    <button class="wbtn" onclick={exportPng} disabled={exporting || !hasData}>
      {exporting ? 'EXPORTING…' : 'EXPORT PNG…'}
    </button>
  </div>

  <div class="stage">
    {#if summary}
      <canvas bind:this={canvas} width="1080" height="1350" title="Usage Wrapped preview"></canvas>
    {:else if loading}
      <div class="skel"></div>
    {:else}
      <div class="wempty">
        <b>NO PREVIEW</b>
        <span>{error || 'No data available.'}</span>
      </div>
    {/if}
  </div>

  {#if error && summary}
    <p class="wrefresh-err">{error}</p>
  {/if}

  {#if savedPath}
    <div class="wsaved">
      <span class="sbadge">SAVED</span>
      <code>{savedPath}</code>
    </div>
  {/if}
</div>

<style>
  .wrapped {
    display: flex;
    flex-direction: column;
    gap: 14px;
    margin-top: 8px;
  }

  .wrow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }

  .wpills {
    display: flex;
  }

  .wpill {
    font: 600 11px/1 var(--font-ui);
    letter-spacing: 1px;
    padding: 9px 14px;
    background: var(--bone);
    color: var(--ink);
    border: 2px solid var(--ink);
    border-radius: 0;
    margin-left: -2px;
    cursor: pointer;
  }
  .wpill:first-child {
    margin-left: 0;
  }
  .wpill:hover {
    background: var(--hair);
  }
  .wpill.on {
    background: var(--ink);
    color: #fff;
  }
  .wpill.on:hover {
    background: var(--ink);
  }

  .wbtn {
    font: 600 11px/1 var(--font-ui);
    letter-spacing: 1px;
    text-transform: uppercase;
    padding: 10px 18px;
    background: var(--bone);
    color: var(--ink);
    border: 2px solid var(--ink);
    border-radius: 0;
    cursor: pointer;
  }
  .wbtn:hover:not(:disabled) {
    background: var(--ink);
    color: #fff;
  }
  .wbtn:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .stage {
    display: flex;
  }

  canvas {
    width: 300px;
    height: 375px;
    border: 2px solid var(--ink);
    display: block;
  }

  .skel {
    width: 300px;
    height: 375px;
    border: 2px solid var(--hair);
    background: repeating-linear-gradient(
      135deg,
      var(--bone) 0 12px,
      rgba(13, 13, 11, 0.06) 12px 24px
    );
    animation: skel-pulse 1.2s ease-in-out infinite;
  }
  @keyframes skel-pulse {
    50% {
      opacity: 0.6;
    }
  }

  .wempty {
    width: 300px;
    height: 375px;
    border: 2px solid var(--ink);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    font: 600 12px/1 var(--font-ui);
    letter-spacing: 1px;
  }
  .wempty span {
    font: 400 11px/1.5 var(--font-mono);
    color: var(--dim);
    padding: 0 20px;
    text-align: center;
  }

  .wrefresh-err {
    font: 400 11px/1.5 var(--font-mono);
    color: var(--dim);
    margin: 0;
  }

  .wsaved {
    display: flex;
    align-items: center;
    gap: 10px;
    border: 2px solid var(--ink);
    padding: 8px 12px;
  }
  .sbadge {
    background: var(--cyn);
    color: var(--ink);
    font: 700 10px/1 var(--font-mono);
    letter-spacing: 1px;
    padding: 3px 6px;
    flex: none;
  }
  .wsaved code {
    font: 400 11px/1.4 var(--font-mono);
    word-break: break-all;
  }
</style>
