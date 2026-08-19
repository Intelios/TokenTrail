<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import type { EChartsOption } from 'echarts';
  import Chart from '$lib/Chart.svelte';
  import { api, type ModelStatsRow } from '$lib/api';
  import { fmtCost, fmtDate, fmtTokens, MODEL_PALETTE, sourceColor, sourceLabel } from '$lib/format';

  const RANGES: [number, string][] = [
    [7, '7 days'],
    [30, '30 days'],
    [90, '90 days'],
    [3650, 'All time'],
  ];

  const METRICS: ['tokens' | 'cost' | 'calls', string][] = [
    ['tokens', 'Tokens'],
    ['cost', 'Cost'],
    ['calls', 'Calls'],
  ];

  let days = $state(90);
  let metric = $state<'tokens' | 'cost' | 'calls'>('tokens');
  let rows = $state<ModelStatsRow[]>([]);
  let error = $state('');

  function metricVal(r: ModelStatsRow): number {
    if (metric === 'tokens') return r.tokens;
    if (metric === 'cost') return r.cost_usd ?? 0;
    return r.events;
  }

  function fmtMetric(n: number): string {
    if (metric === 'tokens') return fmtTokens(n);
    if (metric === 'cost') return fmtCost(n);
    return n.toLocaleString();
  }

  async function load() {
    try {
      rows = await api.modelStats(days);
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  $effect(() => {
    days;
    load();
  });

  onMount(() => {
    const h = () => load();
    window.addEventListener('tt-sync', h);
    return () => window.removeEventListener('tt-sync', h);
  });

  const sorted = $derived(
    metric === 'tokens' ? [...rows].sort((a, b) => b.tokens - a.tokens) :
    metric === 'cost' ? [...rows].sort((a, b) => (b.cost_usd ?? 0) - (a.cost_usd ?? 0)) :
    [...rows].sort((a, b) => b.events - a.events)
  );

  const totalMetric = $derived(sorted.reduce((s, r) => s + metricVal(r), 0));
  const totalTokens = $derived(rows.reduce((s, r) => s + r.tokens, 0));
  const totalCost = $derived(rows.reduce((s, r) => s + (r.cost_usd ?? 0), 0));
  const totalEvents = $derived(rows.reduce((s, r) => s + r.events, 0));
  const totalOutput = $derived(rows.reduce((s, r) => s + r.output_tokens, 0));
  const costliest = $derived([...rows].sort((a, b) => (b.cost_usd ?? 0) - (a.cost_usd ?? 0))[0]);

  const donutOption = $derived.by(() => {
    if (!sorted.length) return undefined;
    const top8 = sorted.slice(0, 8);
    const otherVal = sorted.slice(8).reduce((s, r) => s + metricVal(r), 0);
    const data = top8.map((r, i) => ({
      name: r.model,
      value: metricVal(r),
      itemStyle: { color: MODEL_PALETTE[i % MODEL_PALETTE.length] },
    }));
    if (otherVal > 0) {
      data.push({ name: 'Other', value: otherVal, itemStyle: { color: '#94a3b8' } });
    }
    return {
      backgroundColor: 'transparent',
      tooltip: {
        backgroundColor: '#131828',
        borderColor: '#232b41',
        textStyle: { color: '#e2e8f0' },
        formatter: (p: any) => `${p.name}<br/>${fmtMetric(p.value)} (${p.percent}%)`,
      },
      series: [
        {
          type: 'pie',
          radius: ['58%', '82%'],
          label: { show: false },
          itemStyle: { borderColor: '#131828', borderWidth: 2 },
          data,
        },
      ],
    } satisfies EChartsOption;
  });

  const barsOption = $derived.by(() => {
    if (!rows.length) return undefined;
    const top10 = [...rows].sort((a, b) => b.tokens - a.tokens).slice(0, 10);
    const models = top10.map((r) => r.model);
    return {
      backgroundColor: 'transparent',
      tooltip: {
        trigger: 'axis',
        backgroundColor: '#131828',
        borderColor: '#232b41',
        textStyle: { color: '#e2e8f0' },
        valueFormatter: (v: unknown) => fmtTokens(Number(v)),
      },
      grid: { left: 8, right: 12, top: 16, bottom: 0, containLabel: true },
      xAxis: {
        type: 'value',
        axisLabel: { color: '#8b95ab', formatter: (v: number) => fmtTokens(v) },
        splitLine: { lineStyle: { color: 'rgba(35,43,65,0.5)' } },
      },
      yAxis: {
        type: 'category',
        data: models,
        inverse: true,
        axisLine: { lineStyle: { color: '#232b41' } },
        axisLabel: { color: '#8b95ab', width: 120, overflow: 'truncate' },
      },
      series: [
        { name: 'Tokens', type: 'bar', data: top10.map((r) => r.tokens), itemStyle: { color: '#a78bfa', borderRadius: [0, 2, 2, 0] } },
      ],
    } satisfies EChartsOption;
  });

  const metricLabel = $derived(metric === 'tokens' ? 'tokens' : metric === 'cost' ? 'cost' : 'calls');

  function modelUrl(name: string): string {
    return '/models/' + encodeURIComponent(name);
  }
</script>

<h1>Models</h1>
<p class="sub">Your model lineup — ranked by what you actually use</p>

<div class="row" style="margin-bottom:12px">
  {#each RANGES as [d, label]}
    <button class="pill" class:on={days === d} onclick={() => (days = d)}>{label}</button>
  {/each}
</div>
<div class="row" style="margin-bottom:20px">
  {#each METRICS as [m, label]}
    <button class="pill" class:on={metric === m} onclick={() => (metric = m)}>{label}</button>
  {/each}
</div>

{#if error}
  <p class="error">{error}</p>
{:else if !rows.length}
  <div class="loading">no models recorded yet</div>
{:else}
  <div class="cards">
    <div class="card">
      <div class="label">Models used</div>
      <div class="value">{rows.length}</div>
      <div class="hint">{totalEvents.toLocaleString()} calls</div>
    </div>
    <div class="card">
      <div class="label">Top model</div>
      <div class="value ell">{sorted[0]?.model ?? '—'}</div>
      <div class="hint">{totalMetric ? ((metricVal(sorted[0]) / totalMetric) * 100).toFixed(1) : 0}% of {metricLabel}</div>
    </div>
    <div class="card">
      <div class="label">Costliest model</div>
      <div class="value ell">{costliest?.model ?? '—'}</div>
      <div class="hint">{fmtCost(costliest?.cost_usd ?? null)}</div>
    </div>
    <div class="card">
      <div class="label">Est. cost</div>
      <div class="value">{fmtCost(totalCost)}</div>
      <div class="hint">{fmtTokens(totalTokens)} tokens</div>
    </div>
    <div class="card">
      <div class="label">Avg tokens / call</div>
      <div class="value">{totalEvents ? fmtTokens(totalTokens / totalEvents) : '—'}</div>
      <div class="hint">{totalTokens ? ((totalOutput / totalTokens) * 100).toFixed(0) : 0}% output</div>
    </div>
  </div>

  <div class="grid2">
    <div class="panel">
      <h2>Total tokens — top 10</h2>
      {#if barsOption}
        <Chart option={barsOption} height={340} />
      {:else}
        <div class="loading">no data</div>
      {/if}
    </div>
    <div class="panel">
      <h2>Share of {metricLabel} — top 8</h2>
      {#if donutOption}
        <Chart option={donutOption} height={230} />
      {:else}
        <div class="loading">no data</div>
      {/if}
    </div>
  </div>

  <div class="panel" style="padding:6px 12px">
    <table>
      <thead>
        <tr>
          <th>#</th>
          <th>Model</th>
          <th class="num">Calls</th>
          <th class="num">Tokens</th>
          <th class="num">Est. cost</th>
          <th class="num">Share</th>
          <th class="num">Last used</th>
        </tr>
      </thead>
      <tbody>
        {#each sorted as r, i}
          <tr style="cursor:pointer" onclick={() => goto(modelUrl(r.model))}>
            <td class="muted">{i + 1}</td>
            <td>
              <div style="display:flex;align-items:center;gap:8px">
                <span class="mdot" style="background:{MODEL_PALETTE[i % MODEL_PALETTE.length]}"></span>
                <a class="modellink" href={modelUrl(r.model)} onclick={(e) => e.stopPropagation()}>{r.model}</a>
              </div>
              {#if r.sources.length}
                <div class="tagrow">
                  {#each r.sources as s}
                    <span class="tag" style="border-color:{sourceColor(s)};color:{sourceColor(s)}">{sourceLabel(s)}</span>
                  {/each}
                </div>
              {/if}
            </td>
            <td class="num">{r.events.toLocaleString()}</td>
            <td class="num">
              {fmtTokens(r.tokens)}
              <div class="path">{fmtTokens(r.tokens / r.events)} avg</div>
            </td>
            <td class="num">{fmtCost(r.cost_usd)}</td>
            <td class="num">{totalMetric ? ((metricVal(r) / totalMetric) * 100).toFixed(1) : 0}%</td>
            <td class="num muted">{fmtDate(r.last_ts)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .modellink {
    color: var(--text);
    text-decoration: none;
    transition: color 0.15s;
  }
  .modellink:hover {
    color: var(--accent);
    text-decoration: underline;
  }
</style>
