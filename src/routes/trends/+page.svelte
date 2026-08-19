<script lang="ts">
  import { onMount } from 'svelte';
  import type { EChartsOption } from 'echarts';
  import Chart from '$lib/Chart.svelte';
  import { api, type DailyCacheRow, type DailyModelRow } from '$lib/api';
  import { fmtTokens, MODEL_PALETTE } from '$lib/format';

  let days = $state(365);
  let byModel = $state<DailyModelRow[]>([]);
  let cache = $state<DailyCacheRow[]>([]);
  let error = $state('');

  async function load() {
    try {
      const [m, c] = await Promise.all([api.dailyByModel(days), api.dailyCache(days)]);
      byModel = m;
      cache = c;
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  $effect(() => {
    // reload when the range changes
    days;
    load();
  });

  onMount(() => {
    const h = () => load();
    window.addEventListener('tt-sync', h);
    return () => window.removeEventListener('tt-sync', h);
  });

  const modelOption = $derived.by(() => {
    if (!byModel.length) return undefined;
    const dates = [...new Set(byModel.map((r) => r.date))].sort();
    const models = [...new Set(byModel.map((r) => r.model))];
    const map = new Map(byModel.map((r) => [`${r.date}|${r.model}`, r.tokens]));
    return {
      backgroundColor: 'transparent',
      color: MODEL_PALETTE,
      tooltip: {
        trigger: 'axis',
        backgroundColor: '#131828',
        borderColor: '#232b41',
        textStyle: { color: '#e2e8f0' },
        valueFormatter: (v: unknown) => fmtTokens(Number(v)),
      },
      legend: { textStyle: { color: '#8b95ab' }, top: 0, type: 'scroll', icon: 'roundRect' },
      grid: { left: 8, right: 12, top: 32, bottom: 0, containLabel: true },
      xAxis: {
        type: 'category',
        data: dates,
        axisLine: { lineStyle: { color: '#232b41' } },
        axisLabel: { color: '#8b95ab' },
      },
      yAxis: {
        type: 'value',
        axisLabel: { color: '#8b95ab', formatter: (v: number) => fmtTokens(v) },
        splitLine: { lineStyle: { color: 'rgba(35,43,65,0.5)' } },
      },
      series: models.map((m) => ({
        name: m,
        type: 'line',
        stack: 'total',
        areaStyle: { opacity: 0.3 },
        smooth: true,
        symbol: 'none',
        lineStyle: { width: 1.5 },
        emphasis: { focus: 'series' },
        data: dates.map((d) => map.get(`${d}|${m}`) ?? 0),
      })),
    } satisfies EChartsOption;
  });

  const cacheOption = $derived.by(() => {
    if (!cache.length) return undefined;
    const dates = cache.map((r) => r.date);
    const series = [
      { name: 'Cache read', color: '#34d399', key: 'cache_read' as const },
      { name: 'Fresh input', color: '#60a5fa', key: 'fresh_input' as const },
      { name: 'Cache write', color: '#f472b6', key: 'cache_write' as const },
    ];
    return {
      backgroundColor: 'transparent',
      tooltip: {
        trigger: 'axis',
        backgroundColor: '#131828',
        borderColor: '#232b41',
        textStyle: { color: '#e2e8f0' },
        valueFormatter: (v: unknown) => fmtTokens(Number(v)),
      },
      legend: { textStyle: { color: '#8b95ab' }, top: 0, icon: 'roundRect' },
      grid: { left: 8, right: 12, top: 32, bottom: 0, containLabel: true },
      xAxis: {
        type: 'category',
        data: dates,
        axisLine: { lineStyle: { color: '#232b41' } },
        axisLabel: { color: '#8b95ab' },
      },
      yAxis: {
        type: 'value',
        axisLabel: { color: '#8b95ab', formatter: (v: number) => fmtTokens(v) },
        splitLine: { lineStyle: { color: 'rgba(35,43,65,0.5)' } },
      },
      series: series.map((s) => ({
        name: s.name,
        type: 'line',
        stack: 'total',
        areaStyle: { opacity: 0.3 },
        smooth: true,
        symbol: 'none',
        lineStyle: { width: 1.5 },
        itemStyle: { color: s.color },
        emphasis: { focus: 'series' },
        data: cache.map((r) => r[s.key]),
      })),
    } satisfies EChartsOption;
  });

  const cacheShare = $derived.by(() => {
    const read = cache.reduce((a, r) => a + r.cache_read, 0);
    const fresh = cache.reduce((a, r) => a + r.fresh_input, 0);
    const write = cache.reduce((a, r) => a + r.cache_write, 0);
    const total = read + fresh + write;
    return total > 0 ? Math.round((read / total) * 100) : 0;
  });
</script>

<h1>Trends</h1>
<p class="sub">Which models you actually use, and how much context gets served from cache</p>

<div class="row" style="margin-bottom:16px">
  {#each [30, 90, 365] as d}
    <button class="pill" class:on={days === d} onclick={() => (days = d)}>{d === 365 ? '1 year' : `${d} days`}</button>
  {/each}
</div>

{#if error}
  <p class="error">{error}</p>
{:else}
  <div class="panel">
    <h2>Model share over time</h2>
    {#if modelOption}
      <Chart option={modelOption} height={320} />
    {:else}
      <div class="loading">no usage recorded yet</div>
    {/if}
  </div>

  <div class="panel">
    <h2>Context: cache vs fresh — {cacheShare}% served from cache read</h2>
    {#if cacheOption}
      <Chart option={cacheOption} height={280} />
    {:else}
      <div class="loading">no usage recorded yet</div>
    {/if}
  </div>
{/if}
