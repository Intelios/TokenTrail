<script lang="ts">
  import { onMount } from 'svelte';
  import type { EChartsOption } from 'echarts';
  import Chart from '$lib/Chart.svelte';
  import { api, type DailyCacheRow, type DailyModelRow } from '$lib/api';
  import { fmtTokens, MODEL_PALETTE, MIX_COLORS } from '$lib/format';
  import { TOOLTIP, ANIM, AXIS_LABEL, AXIS_LINE, SPLIT_LINE, LEGEND_TEXT, stackedBand, dateTick } from '$lib/chartTheme';

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
    const totals = new Map<string, number>();
    for (const r of byModel) totals.set(r.model, (totals.get(r.model) ?? 0) + r.tokens);
    // stack in rank order so band colors match the Models page ranking
    const models = [...totals.entries()].sort((a, b) => b[1] - a[1]).map(([m]) => m);
    const map = new Map(byModel.map((r) => [`${r.date}|${r.model}`, r.tokens]));
    return {
      backgroundColor: 'transparent',
      ...ANIM,
      animationDelay: (idx: number) => Math.min(idx, 8) * 90,
      tooltip: {
        trigger: 'axis',
        ...TOOLTIP,
        confine: true,
        formatter: (params: unknown) => {
          type TipParam = { axisValue?: string; marker: string; seriesName: string; value?: number };
          const all = params as TipParam[];
          const title = `<b>${all[0]?.axisValue ?? ''}</b>`;
          const list = all
            .filter((p) => Number(p.value ?? 0) > 0)
            .sort((a, b) => Number(b.value) - Number(a.value));
          if (!list.length) return `${title}<br/>no usage`;
          return (
            title +
            '<br/>' +
            list
              .map((p) => `${p.marker}${p.seriesName}&nbsp;&nbsp;<b>${fmtTokens(Number(p.value))}</b>`)
              .join('<br/>')
          );
        },
      },
      legend: { textStyle: LEGEND_TEXT, top: 0, type: 'scroll', icon: 'rect', itemWidth: 10, itemHeight: 10 },
      grid: { left: 8, right: 12, top: 34, bottom: 0, containLabel: true },
      xAxis: {
        type: 'category',
        data: dates,
        axisLine: AXIS_LINE,
        axisLabel: { ...AXIS_LABEL, hideOverlap: true, formatter: dateTick },
        axisTick: { show: false },
      },
      yAxis: {
        type: 'value',
        axisLabel: { ...AXIS_LABEL, formatter: (v: number) => fmtTokens(v) },
        splitLine: SPLIT_LINE,
      },
      series: models.map((m, i) =>
        stackedBand(m, dates.map((d) => map.get(`${d}|${m}`) ?? 0), MODEL_PALETTE[i % MODEL_PALETTE.length], {
          delay: Math.min(i, 8) * 90,
        }),
      ),
    } satisfies EChartsOption;
  });

  const cacheOption = $derived.by(() => {
    if (!cache.length) return undefined;
    const dates = cache.map((r) => r.date);
    const series = [
      { name: 'Cache read', color: MIX_COLORS[2], key: 'cache_read' as const },
      { name: 'Fresh input', color: MIX_COLORS[0], key: 'fresh_input' as const },
      { name: 'Cache write', color: MIX_COLORS[3], key: 'cache_write' as const },
    ];
    return {
      backgroundColor: 'transparent',
      ...ANIM,
      animationDelay: (idx: number) => idx * 90,
      tooltip: {
        trigger: 'axis',
        ...TOOLTIP,
        valueFormatter: (v: unknown) => fmtTokens(Number(v)),
      },
      legend: { textStyle: LEGEND_TEXT, top: 0, icon: 'rect', itemWidth: 10, itemHeight: 10 },
      grid: { left: 8, right: 12, top: 34, bottom: 0, containLabel: true },
      xAxis: {
        type: 'category',
        data: dates,
        axisLine: AXIS_LINE,
        axisLabel: { ...AXIS_LABEL, hideOverlap: true, formatter: dateTick },
        axisTick: { show: false },
      },
      yAxis: {
        type: 'value',
        axisLabel: { ...AXIS_LABEL, formatter: (v: number) => fmtTokens(v) },
        splitLine: SPLIT_LINE,
      },
      series: series.map((s, i) =>
        stackedBand(s.name, cache.map((r) => r[s.key]), s.color, { delay: i * 90 }),
      ),
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

<div class="tframe">
  <div class="thd">
    <div class="up">
      <h1>Trends</h1>
      <div class="sub">Which models you actually use, and how much context gets served from cache</div>
    </div>
    <div class="pills up">
      {#each [30, 90, 365] as d}
        <button class="pill" class:on={days === d} onclick={() => (days = d)}>{d === 365 ? '1 YEAR' : `${d}D`}</button>
      {/each}
    </div>
  </div>

  {#if error}
    <div class="errpad"><p class="error">{error}</p></div>
  {:else}
    <div class="tpanel up">
      <h2>Model share over time</h2>
      {#if modelOption}
        <Chart option={modelOption} height={320} />
      {:else}
        <div class="loading">no usage recorded yet</div>
      {/if}
    </div>

    <div class="tpanel up" style="animation-delay:100ms">
      <h2>
        Context: cache vs fresh<span class="rt"><b class="delta">{cacheShare}% served from cache read</b></span>
      </h2>
      {#if cacheOption}
        <Chart option={cacheOption} height={280} />
      {:else}
        <div class="loading">no usage recorded yet</div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .tframe {
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

  .errpad { padding: 20px 22px; }

  .tpanel {
    background: var(--bone);
    border-bottom: 2px solid var(--ink);
    padding: 16px clamp(22px, 1.8vw, 40px) 18px;
  }
  .tpanel:last-child {
    border-bottom: none;
  }
  .tpanel h2 {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    font: 600 11px/1 var(--font-ui);
    letter-spacing: 1.6px;
    text-transform: uppercase;
    margin: 0 0 12px;
  }

  .rt {
    display: flex;
    gap: 14px;
    align-items: baseline;
  }
  .delta {
    font: 500 10px/1 var(--font-mono);
    letter-spacing: 1px;
    color: var(--org);
  }
</style>
